//! The rustWM daemon. Users normally start it with `rwm start`.

#[cfg(windows)]
mod watch;

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "rustwm",
    version,
    about = "The rustWM tiling window manager daemon"
)]
struct Args {
    /// Config file to use instead of %USERPROFILE%\.config\rustwm\rustwm.toml.
    #[arg(long, short)]
    config: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "rustWM starting");
    run(args)
}

/// Loads the config, falling back to the built-in defaults if it is broken so
/// rustWM always starts.
#[cfg(windows)]
fn load_config(path: &std::path::Path) -> rwm_config::Loaded {
    match rwm_config::ensure_default(path) {
        Ok(true) => tracing::info!(path = %path.display(), "wrote default config"),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "could not write default config")
        }
    }
    match rwm_config::load(path) {
        Ok(loaded) => {
            tracing::info!(path = %path.display(), "config loaded");
            loaded
        }
        Err(e) => {
            tracing::error!(path = %path.display(), "{e}; using built-in defaults");
            rwm_config::parse("").expect("empty config is valid")
        }
    }
}

#[cfg(windows)]
fn run(args: Args) -> anyhow::Result<()> {
    use rwm_core::Wm;
    use rwm_core::wm::Flow;

    rwm_win32::enable_dpi_awareness();

    // Whatever happens on the core thread, never leave windows hidden.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let shown = rwm_win32::restore_hidden_windows();
        eprintln!("rustWM panicked; restored {shown} hidden windows");
        default_hook(info);
    }));

    let config_path = args.config.unwrap_or_else(rwm_config::default_path);
    let loaded = load_config(&config_path);
    rwm_win32::set_bindings(loaded.bindings);

    let (tx, rx) = std::sync::mpsc::channel();
    let events = rwm_win32::EventThread::spawn(tx.clone())?;
    rwm_win32::install_ctrl_handler();
    let _watcher = match watch::watch_config(&config_path, tx) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "config hot reload disabled");
            None
        }
    };

    let mut wm = Wm::with_settings(rwm_win32::Win32Platform::new(), loaded.settings);
    wm.start()?;
    tracing::info!("rustWM running; Win+Ctrl+Shift+Q or Ctrl+C to quit");

    for event in rx {
        match wm.handle(event) {
            Flow::Continue => {}
            Flow::Reload => match rwm_config::load(&config_path) {
                Ok(loaded) => {
                    rwm_win32::set_bindings(loaded.bindings);
                    wm.set_settings(loaded.settings);
                    tracing::info!("config reloaded");
                }
                Err(e) => tracing::error!("{e}; keeping the previous config"),
            },
            Flow::Exit => break,
        }
    }
    wm.restore();
    events.stop();
    tracing::info!("rustWM stopped");
    Ok(())
}

#[cfg(not(windows))]
fn run(_args: Args) -> anyhow::Result<()> {
    anyhow::bail!("rustWM only runs on Windows")
}
