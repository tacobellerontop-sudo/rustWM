//! The menu shown when `rwm` is run with no arguments, e.g. by
//! double-clicking `rwm.exe`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use rwm_ipc::{Client, Request};

/// Runs the menu until the user quits or stdin closes.
pub fn run() -> anyhow::Result<()> {
    let config = rwm_config::default_path();
    let mut stdin = std::io::stdin().lock();
    loop {
        clear_screen();
        let running = Client::connect().is_ok();
        let kerys = super::kerys_running();
        print_header(running, kerys, &config);
        let items = items(running, kerys);
        for (i, item) in items.iter().enumerate() {
            println!("  {}  {}", i + 1, item.label());
        }
        println!("  0  Quit");
        println!();
        print!("  Choose an option: ");
        std::io::stdout().flush()?;

        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let choice = line.trim();
        if matches!(choice, "0" | "q" | "Q" | "quit" | "exit") {
            return Ok(());
        }
        let Some(item) = choice
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| items.get(n))
        else {
            continue;
        };
        println!();
        let result = match item {
            // The Settings screen has its own loop and returns here.
            Item::Settings => match crate::settings::run(&config, &mut stdin) {
                Ok(()) => continue,
                Err(e) => Err(e),
            },
            _ => item.run(&config),
        };
        if let Err(e) = result {
            println!("  Error: {e:#}");
        }
        println!();
        print!("  Press Enter to go back to the menu...");
        std::io::stdout().flush()?;
        line.clear();
        if stdin.read_line(&mut line)? == 0 {
            return Ok(());
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Item {
    Start,
    StartKerys,
    Stop,
    Restart,
    Settings,
    EditConfig,
    CheckConfig,
    EditKeys,
    Keybindings,
    Autostart(bool),
    AddToPath,
    Restore,
    OpenLog,
}

fn items(running: bool, kerys: bool) -> Vec<Item> {
    let mut items = if running {
        vec![Item::Stop, Item::Restart]
    } else {
        vec![Item::Start]
    };
    if running && !kerys {
        items.insert(0, Item::StartKerys);
    }
    items.extend([
        Item::Settings,
        Item::EditConfig,
        Item::CheckConfig,
        Item::EditKeys,
        Item::Keybindings,
    ]);
    items.push(Item::Autostart(!autostart_on().unwrap_or(false)));
    if in_path() == Some(false) {
        items.push(Item::AddToPath);
    }
    items.extend([Item::Restore, Item::OpenLog]);
    items
}

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Start => "Start rustWM + kerys",
            Item::StartKerys => "Start kerys (hotkeys are off)",
            Item::Stop => "Stop rustWM + kerys (all windows come back)",
            Item::Restart => "Restart rustWM + kerys",
            Item::Settings => "Settings (focus follows mouse, border, gaps, ...)",
            Item::EditConfig => "Edit config file (changes apply when you save)",
            Item::CheckConfig => "Check config for errors",
            Item::EditKeys => "Edit keybindings (settings window)",
            Item::Keybindings => "Show keybindings",
            Item::Autostart(true) => "Start rustWM when I log in",
            Item::Autostart(false) => "Don't start rustWM when I log in",
            Item::AddToPath => "Add rwm to PATH (use `rwm` from any terminal)",
            Item::Restore => "Restore hidden windows (after a crash)",
            Item::OpenLog => "Open log file",
        }
    }

    fn run(self, config: &Path) -> anyhow::Result<()> {
        match self {
            // Handled by the menu loop, which owns stdin.
            Item::Settings => Ok(()),
            Item::Start => super::start(None),
            Item::StartKerys => super::start_kerys(),
            Item::Stop => super::stop(),
            Item::Restart => {
                super::stop()?;
                super::start(None)
            }
            Item::EditConfig => {
                if rwm_config::ensure_default(config)? {
                    println!("  Wrote the default config to {}", config.display());
                }
                open(config)?;
                println!("  Opened {}", config.display());
                Ok(())
            }
            Item::CheckConfig => {
                rwm_config::load(config)?;
                println!("  {}: OK", config.display());
                Ok(())
            }
            Item::EditKeys => {
                let kerys = super::kerys_path().ok_or_else(|| {
                    anyhow::anyhow!("kerys.exe is missing; put it next to rwm.exe")
                })?;
                std::process::Command::new(kerys)
                    .arg("gui")
                    .spawn()
                    .map_err(|e| {
                        anyhow::anyhow!("cannot run kerys ({e}); keep kerys.exe next to rwm.exe")
                    })?;
                println!("  Opened the kerys settings window");
                Ok(())
            }
            Item::Keybindings => {
                let kerys = super::kerys_path().ok_or_else(|| {
                    anyhow::anyhow!("kerys.exe is missing; put it next to rwm.exe")
                })?;
                let status = std::process::Command::new(kerys).arg("bindings").status();
                match status {
                    Ok(s) if s.success() => Ok(()),
                    Ok(s) => anyhow::bail!("kerys bindings failed ({s})"),
                    Err(e) => {
                        anyhow::bail!("cannot run kerys ({e}); keep kerys.exe next to rwm.exe")
                    }
                }
            }
            Item::Autostart(on) => super::autostart(if on { "on" } else { "off" }),
            Item::AddToPath => add_to_path(),
            Item::Restore => super::restore(),
            Item::OpenLog => {
                let log = log_path();
                if !log.exists() {
                    anyhow::bail!("no log yet ({})", log.display());
                }
                open(&log)
            }
        }
    }
}

fn print_header(running: bool, kerys: bool, config: &Path) {
    println!();
    println!("  rustWM {}", env!("CARGO_PKG_VERSION"));
    println!("  ----------------------------------------------");
    let status = if running {
        match summary() {
            Some(s) => format!("running ({s})"),
            None => "running".into(),
        }
    } else {
        "not running".into()
    };
    println!("  Status:     {status}");
    let hotkeys = if kerys {
        "on (kerys running)"
    } else if super::kerys_path().is_none() {
        "off: kerys.exe is missing, put it next to rwm.exe"
    } else if running {
        "off: kerys is not running"
    } else {
        "off (kerys starts with rustWM)"
    };
    println!("  Hotkeys:    {hotkeys}");
    println!("  Config:     {}", config.display());
    if let Some(on) = autostart_on() {
        println!("  Autostart:  {}", if on { "on" } else { "off" });
    }
    if in_path() == Some(false) {
        println!();
        println!("  Tip: in PowerShell, run commands as .\\rwm.exe <command>,");
        println!("  or pick \"Add rwm to PATH\" below to type just rwm.");
    }
    println!();
}

/// "2 monitors, 7 windows", from the daemon's state.
fn summary() -> Option<String> {
    let response = Client::connect()
        .ok()?
        .request(&Request::Query {
            query: "state".into(),
        })
        .ok()?;
    let state = response.data?;
    let monitors = state["monitors"].as_array()?;
    let windows: usize = monitors
        .iter()
        .flat_map(|m| m["workspaces"].as_array().into_iter().flatten())
        .map(|w| w["windows"].as_array().map_or(0, Vec::len))
        .sum();
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    Some(format!(
        "{}, {}",
        plural(monitors.len(), "monitor"),
        plural(windows, "window")
    ))
}

#[cfg(windows)]
pub(crate) fn clear_screen() {
    let _ = std::process::Command::new("cmd")
        .args(["/c", "cls"])
        .status();
}

#[cfg(windows)]
fn open(path: &Path) -> anyhow::Result<()> {
    std::process::Command::new("notepad.exe")
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(windows)]
fn log_path() -> PathBuf {
    super::log_path()
}

#[cfg(windows)]
fn autostart_on() -> Option<bool> {
    Some(rwm_win32::autostart::current().is_some())
}

#[cfg(windows)]
fn exe_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

#[cfg(windows)]
fn in_path() -> Option<bool> {
    Some(rwm_win32::user_path::contains(&exe_dir()?))
}

#[cfg(windows)]
fn add_to_path() -> anyhow::Result<()> {
    let dir = exe_dir().ok_or_else(|| anyhow::anyhow!("cannot find rwm.exe's folder"))?;
    if rwm_win32::user_path::add(&dir)? {
        println!("  Added {} to your PATH.", dir.display());
        println!("  Open a new terminal and type `rwm --help`.");
    } else {
        println!("  {} is already in your PATH.", dir.display());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn clear_screen() {}

#[cfg(not(windows))]
fn open(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("opening files only works on Windows")
}

#[cfg(not(windows))]
fn log_path() -> PathBuf {
    std::env::temp_dir().join("rustwm.log")
}

#[cfg(not(windows))]
fn autostart_on() -> Option<bool> {
    None
}

#[cfg(not(windows))]
fn in_path() -> Option<bool> {
    None
}

#[cfg(not(windows))]
fn add_to_path() -> anyhow::Result<()> {
    anyhow::bail!("only works on Windows")
}
