//! `rwm`: start, stop and control the rustWM daemon over its named pipe.

mod menu;

use std::io::{ErrorKind, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::bail;
use clap::{Parser, Subcommand};
use rwm_ipc::{Client, Request, Response};

#[derive(Debug, Parser)]
#[command(
    name = "rwm",
    version,
    about = "Control the rustWM tiling window manager",
    after_help = "Run rwm with no arguments for a menu.\n\n\
                  Any other command is sent to rustWM, e.g. `rwm focus left`, `rwm workspace 3`, \
                  `rwm layout master`. They are the same commands kerys binds to keys."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Start rustWM (and with it kerys, the hotkey daemon) in the background.
    Start {
        /// Config file to use instead of the default.
        #[arg(long, short)]
        config: Option<PathBuf>,
    },
    /// Stop rustWM and kerys, and restore all windows.
    Stop,
    /// Stop and start rustWM again.
    Restart {
        #[arg(long, short)]
        config: Option<PathBuf>,
    },
    /// Print rustWM's state as JSON: state, windows, focused, config or version.
    Query {
        #[arg(default_value = "state")]
        what: String,
    },
    /// Stream state changes as JSON lines: focus, workspace, window (default: all).
    Subscribe { topics: Vec<String> },
    /// Show every window rustWM has hidden. Works even if rustWM crashed.
    Restore,
    /// Check a config file for errors without starting rustWM.
    CheckConfig { path: Option<PathBuf> },
    /// Start rustWM when you log in: on, off or status.
    Autostart {
        #[arg(value_parser = ["on", "off", "status"])]
        state: String,
    },
    #[command(external_subcommand)]
    Other(Vec<String>),
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rwm: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        // Double-clicked or run bare in a console: show the menu.
        None if std::io::stdin().is_terminal() => menu::run(),
        None => {
            println!("rwm {}: try `rwm --help`", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Cmd::Start { config }) => start(config.as_deref()),
        Some(Cmd::Stop) => stop(),
        Some(Cmd::Restart { config }) => {
            if Client::connect().is_ok() {
                stop()?;
            }
            start(config.as_deref())
        }
        Some(Cmd::Query { what }) => {
            let response = connect()?.request(&Request::Query { query: what })?;
            let data = check(response)?.data.unwrap_or_default();
            println!("{}", serde_json::to_string_pretty(&data)?);
            Ok(())
        }
        Some(Cmd::Subscribe { topics }) => {
            for event in connect()?.subscribe(topics)? {
                println!("{}", serde_json::to_string(&event?)?);
            }
            Ok(())
        }
        Some(Cmd::Restore) => restore(),
        Some(Cmd::CheckConfig { path }) => {
            let path = path.unwrap_or_else(rwm_config::default_path);
            rwm_config::load(&path)?;
            println!("{}: OK", path.display());
            Ok(())
        }
        Some(Cmd::Autostart { state }) => autostart(&state),
        Some(Cmd::Other(words)) => {
            check(connect()?.command(&words.join(" "))?)?;
            Ok(())
        }
    }
}

fn connect() -> anyhow::Result<Client> {
    Client::connect().map_err(|e| match e.kind() {
        ErrorKind::NotFound => anyhow::anyhow!("rustWM is not running (start it with `rwm start`)"),
        _ => anyhow::Error::new(e).context("cannot reach rustWM"),
    })
}

fn check(response: Response) -> anyhow::Result<Response> {
    if response.ok {
        Ok(response)
    } else {
        bail!(
            "{}",
            response.error.unwrap_or_else(|| "request failed".into())
        )
    }
}

/// Polls `cond` until it holds or `timeout` passes.
fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    cond()
}

/// kerys.exe next to rwm.exe, else on PATH. rustwm.exe looks in the same
/// places (it sits next to rwm.exe).
pub(crate) fn kerys_path() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("kerys.exe")));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join("kerys.exe"));
    beside.into_iter().chain(on_path).find(|p| p.is_file())
}

/// Stops rustWM, which also stops the kerys it started, then any kerys that
/// was started on its own.
fn stop() -> anyhow::Result<()> {
    if Client::connect().is_ok() {
        stop_rustwm()?;
    } else {
        println!("rustWM is not running");
    }
    if kerys_running() && !wait_for(Duration::from_secs(2), || !kerys_running()) {
        if let Some(kerys) = kerys_path() {
            let _ = std::process::Command::new(kerys).arg("stop").status();
        }
    }
    if !kerys_running() {
        println!("kerys stopped");
    }
    Ok(())
}

fn stop_rustwm() -> anyhow::Result<()> {
    // The daemon may exit before it answers; a closed pipe also means stopped.
    match connect()?.command("quit") {
        Ok(r) => {
            check(r)?;
        }
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => {}
        Err(e) => return Err(e.into()),
    }
    if !wait_for(Duration::from_secs(5), || Client::connect().is_err()) {
        bail!("rustWM did not stop within 5 seconds");
    }
    println!("rustWM stopped");
    Ok(())
}

#[cfg(windows)]
fn daemon_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let daemon = exe.with_file_name("rustwm.exe");
    if !daemon.exists() {
        bail!("cannot find {} next to rwm.exe", daemon.display());
    }
    Ok(daemon)
}

#[cfg(windows)]
fn log_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("rustwm")
        .join("rustwm.log")
}

#[cfg(windows)]
fn start(config: Option<&std::path::Path>) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;

    use anyhow::Context;

    /// No console window for the background daemon.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if Client::connect().is_ok() {
        bail!("rustWM is already running");
    }
    let log = log_path();
    let mut cmd = std::process::Command::new(daemon_path()?);
    cmd.arg("--log").arg(&log).creation_flags(CREATE_NO_WINDOW);
    if let Some(c) = config {
        cmd.arg("--config").arg(c);
    }
    let mut child = cmd.spawn().context("failed to launch rustwm.exe")?;
    let up = wait_for(Duration::from_secs(5), || {
        Client::connect().is_ok() || matches!(child.try_wait(), Ok(Some(_)))
    });
    if !(up && Client::connect().is_ok()) {
        bail!("rustWM did not start; see {}", log.display())
    }
    println!("rustWM started (log: {})", log.display());
    report_kerys(config);
    Ok(())
}

/// Says whether rustwm.exe managed to start kerys, and why not if it didn't.
#[cfg(windows)]
fn report_kerys(config: Option<&std::path::Path>) {
    let config = config.map_or_else(rwm_config::default_path, Into::into);
    let hotkeys = rwm_config::load(&config)
        .map(|l| l.hotkeys)
        .unwrap_or_else(|_| Some(rwm_config::DEFAULT_HOTKEYS.into()));
    match hotkeys.as_deref() {
        None => println!("no hotkey daemon (hotkeys = \"\" in the config)"),
        Some(h) if !h.to_ascii_lowercase().contains("kerys") => {
            println!("hotkeys are handled by {h}")
        }
        Some(_) if wait_for(Duration::from_secs(3), kerys_running) => {
            println!("kerys started: hotkeys are on")
        }
        Some(_) if kerys_path().is_none() => println!(
            "kerys did not start: kerys.exe is missing. Put it next to rwm.exe \
             (it is in the same zip), then restart."
        ),
        Some(_) => println!(
            "kerys did not start; see {}",
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join("kerys")
                .join("kerys.log")
                .display()
        ),
    }
}

#[cfg(windows)]
fn kerys_running() -> bool {
    rwm_win32::kerys_running()
}

/// Starts kerys by itself, for when rustWM runs without it.
#[cfg(windows)]
fn start_kerys() -> anyhow::Result<()> {
    let Some(kerys) = kerys_path() else {
        bail!("kerys.exe is missing; put it next to rwm.exe");
    };
    let status = std::process::Command::new(kerys).arg("start").status()?;
    if !status.success() {
        bail!("kerys did not start ({status})");
    }
    Ok(())
}

#[cfg(windows)]
fn restore() -> anyhow::Result<()> {
    let shown = rwm_win32::restore_hidden_windows();
    println!("restored {shown} window(s)");
    Ok(())
}

#[cfg(windows)]
fn autostart(state: &str) -> anyhow::Result<()> {
    use rwm_win32::autostart;
    match state {
        "on" => {
            let rwm = std::env::current_exe()?;
            autostart::enable(&format!("\"{}\" start", rwm.display()))?;
            println!("rustWM will start when you log in");
        }
        "off" => {
            autostart::disable()?;
            println!("autostart is off");
        }
        _ => match autostart::current() {
            Some(cmd) => println!("on: {cmd}"),
            None => println!("off"),
        },
    }
    Ok(())
}

#[cfg(not(windows))]
fn kerys_running() -> bool {
    false
}

#[cfg(not(windows))]
fn start_kerys() -> anyhow::Result<()> {
    bail!("kerys only runs on Windows")
}

#[cfg(not(windows))]
fn start(_config: Option<&std::path::Path>) -> anyhow::Result<()> {
    bail!("rustWM only runs on Windows")
}

#[cfg(not(windows))]
fn restore() -> anyhow::Result<()> {
    bail!("rwm restore only works on Windows")
}

#[cfg(not(windows))]
fn autostart(_state: &str) -> anyhow::Result<()> {
    bail!("autostart only works on Windows")
}
