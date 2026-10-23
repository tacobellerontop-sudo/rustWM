//! The rustWM daemon. Users normally start it with `rwm start`.

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "rustWM starting");
    run()
}

#[cfg(windows)]
fn run() -> anyhow::Result<()> {
    let mut wm = rwm_core::Wm::new(rwm_win32::Win32Platform::new());
    wm.start()?;
    Ok(())
}

#[cfg(not(windows))]
fn run() -> anyhow::Result<()> {
    anyhow::bail!("rustWM only runs on Windows")
}
