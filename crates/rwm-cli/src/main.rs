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
                  `rwm layout master`. They are the same commands as in [bindings]."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Start rustWM in the background.
    Start {
        /// Config file to use instead of the default.
        #[arg(long, short)]
        config: Option<PathBuf>,
    },
    /// Stop rustWM and restore all windows.
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

fn stop() -> anyhow::Result<()> {
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
    if up && Client::connect().is_ok() {
        println!("rustWM started (log: {})", log.display());
        Ok(())
    } else {
        bail!("rustWM did not start; see {}", log.display())
    }
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
