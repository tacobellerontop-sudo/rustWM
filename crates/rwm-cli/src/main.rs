//! `rwm`: control a running rustWM daemon over its named pipe.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "rwm",
    version,
    about = "Control the rustWM tiling window manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show every window rustWM has hidden. Works even if the daemon crashed.
    Restore,
    /// Any other window-manager command, e.g. `rwm focus left`.
    #[command(external_subcommand)]
    Other(Vec<String>),
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => {
            println!(
                "rwm {}: nothing to do (try `rwm --help`)",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        Some(Command::Restore) => restore(),
        Some(Command::Other(words)) => anyhow::bail!(
            "the IPC client is not implemented yet; it would send `{}` to {}",
            words.join(" "),
            rwm_ipc::PIPE_NAME
        ),
    }
}

#[cfg(windows)]
fn restore() -> anyhow::Result<()> {
    let shown = rwm_win32::restore_hidden_windows();
    println!("restored {shown} window(s)");
    Ok(())
}

#[cfg(not(windows))]
fn restore() -> anyhow::Result<()> {
    anyhow::bail!("rwm restore only works on Windows")
}
