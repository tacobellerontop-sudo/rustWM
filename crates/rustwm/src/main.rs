//! The rustWM daemon. Users normally start it with `rwm start`.

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "rustwm",
    version,
    about = "The rustWM tiling window manager daemon"
)]
struct Args {
    /// Keep every window tiled in equal columns on the primary monitor
    /// (M1 debug mode, until real layouts land).
    #[arg(long)]
    debug_tile: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "rustWM starting");
    run(args)
}

#[cfg(windows)]
fn run(args: Args) -> anyhow::Result<()> {
    use rwm_core::wm::Flow;
    use rwm_core::{Event, Wm};

    rwm_win32::enable_dpi_awareness();

    // Whatever happens on the core thread, never leave windows hidden.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let shown = rwm_win32::restore_hidden_windows();
        eprintln!("rustWM panicked; restored {shown} hidden windows");
        default_hook(info);
    }));

    let (tx, rx) = std::sync::mpsc::channel();
    let events = rwm_win32::EventThread::spawn(tx.clone())?;
    rwm_win32::install_ctrl_handler();

    let mut wm = Wm::new(rwm_win32::Win32Platform::new());
    wm.start()?;
    if args.debug_tile {
        tx.send(Event::Command("debug-tile".into()))?;
    }
    tracing::info!("rustWM running; press Ctrl+C to quit");

    for event in rx {
        if wm.handle(event) == Flow::Exit {
            break;
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
