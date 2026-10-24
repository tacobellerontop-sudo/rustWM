//! The rustWM daemon. Users normally start it with `rwm start`.

#[cfg(windows)]
mod ipc;
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
    /// Write logs to this file instead of the console.
    #[arg(long)]
    log: Option<PathBuf>,
}

fn init_logging(log: Option<&std::path::Path>) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match log {
        Some(path) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let file = std::fs::File::create(path)?;
            builder
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .init();
        }
        None => builder.init(),
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_logging(args.log.as_deref())?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "rustWM starting");
    run(args)
}

/// Loads the config, falling back to the built-in defaults if it is broken so
/// rustWM always starts.
#[cfg(windows)]
fn load_config(path: &std::path::Path) -> (rwm_config::Loaded, Option<String>) {
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
            (loaded, None)
        }
        Err(e) => {
            tracing::error!(path = %path.display(), "{e}; using built-in defaults");
            let defaults = rwm_config::parse("").expect("empty config is valid");
            (defaults, Some(e.to_string()))
        }
    }
}

#[cfg(windows)]
fn apply_bindings(loaded: &rwm_config::Loaded) {
    rwm_win32::set_bindings(loaded.bindings.clone(), loaded.modes.clone());
    rwm_win32::set_focus_follows_mouse(loaded.settings.focus_follows_mouse);
}

#[cfg(windows)]
fn run(args: Args) -> anyhow::Result<()> {
    use std::sync::Arc;

    use rwm_core::wm::Flow;
    use rwm_core::{Event, Wm};

    // Claim the pipe first: if another rustWM is running, stop here before
    // touching any window.
    let server = rwm_ipc::Server::bind()?;

    rwm_win32::enable_dpi_awareness();

    // Whatever happens on the core thread, never leave windows hidden.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let shown = rwm_win32::restore_hidden_windows();
        eprintln!("rustWM panicked; restored {shown} hidden windows");
        tracing::error!("panic: {info}");
        default_hook(info);
    }));

    let config_path = args.config.unwrap_or_else(rwm_config::default_path);
    let (loaded, config_error) = load_config(&config_path);
    apply_bindings(&loaded);
    rwm_win32::set_config_path(config_path.clone());

    let (tx, rx) = std::sync::mpsc::channel();
    let events = rwm_win32::EventThread::spawn(tx.clone())?;
    rwm_win32::install_ctrl_handler();
    let _watcher = match watch::watch_config(&config_path, tx.clone()) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "config hot reload disabled");
            None
        }
    };

    let animation_ms = Arc::new(std::sync::atomic::AtomicU64::new(
        loaded.settings.animation_ms.into(),
    ));
    let verifier = spawn_verifier(tx.clone(), animation_ms.clone())?;
    let shared = Arc::new(ipc::Shared::new(tx));
    shared.set_config(&config_path, &loaded.bindings);
    {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("ipc".into())
            .spawn(move || {
                let handler = move |pipe| shared.serve_client(pipe);
                if let Err(e) = server.serve(handler) {
                    tracing::error!(error = %e, "IPC server stopped");
                }
            })?;
    }

    let mut wm = Wm::with_settings(rwm_win32::Win32Platform::new(), loaded.settings);
    wm.start()?;
    shared.publish(wm.snapshot());
    if wm.take_verify_request() {
        let _ = verifier.send(());
    }
    if let Some(e) = config_error {
        rwm_win32::notify("rustWM config error", &e);
    }
    tracing::info!("rustWM running; Win+Ctrl+Shift+Q, `rwm stop` or Ctrl+C to quit");

    let mut mode = wm.mode().to_string();
    for event in rx {
        let request = match &event {
            Event::Request { id, .. } => Some(*id),
            _ => None,
        };
        let flow = wm.handle(event);
        if flow == Flow::Reload {
            match rwm_config::load(&config_path) {
                Ok(loaded) => {
                    apply_bindings(&loaded);
                    shared.set_config(&config_path, &loaded.bindings);
                    animation_ms.store(
                        loaded.settings.animation_ms.into(),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    wm.set_settings(loaded.settings);
                    tracing::info!("config reloaded");
                }
                Err(e) => {
                    tracing::error!("{e}; keeping the previous config");
                    rwm_win32::notify("rustWM config error", &e.to_string());
                }
            }
        }
        if wm.mode() != mode {
            mode = wm.mode().to_string();
            rwm_win32::set_mode(&mode);
        }
        shared.publish(wm.snapshot());
        if wm.take_verify_request() {
            let _ = verifier.send(());
        }
        if let Some(id) = request {
            shared.finished(id);
        }
        if flow == Flow::Exit {
            break;
        }
    }
    wm.restore();
    rwm_win32::flush();
    events.stop();
    tracing::info!("rustWM stopped");
    Ok(())
}

/// Sends [`rwm_core::Event::Verify`] once windows have been left alone for a
/// moment after being moved, so the core can float the ones that would not
/// take their tile size.
#[cfg(windows)]
fn spawn_verifier(
    events: std::sync::mpsc::Sender<rwm_core::Event>,
    animation_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> std::io::Result<std::sync::mpsc::Sender<()>> {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::time::Duration;

    /// Windows resize asynchronously; give them this long to settle once
    /// any animation is over.
    const SETTLE_MS: u64 = 300;
    let (kick, kicks) = channel::<()>();
    std::thread::Builder::new()
        .name("verify".into())
        .spawn(move || {
            while kicks.recv().is_ok() {
                loop {
                    let settle = SETTLE_MS + animation_ms.load(Ordering::Relaxed);
                    match kicks.recv_timeout(Duration::from_millis(settle)) {
                        Ok(()) => continue,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if events.send(rwm_core::Event::Verify).is_err() {
                    return;
                }
            }
        })?;
    Ok(kick)
}

#[cfg(not(windows))]
fn run(_args: Args) -> anyhow::Result<()> {
    anyhow::bail!("rustWM only runs on Windows")
}
