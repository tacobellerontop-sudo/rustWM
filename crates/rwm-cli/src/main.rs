//! `rwm`: control a running rustWM daemon over its named pipe.

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "rwm",
    version,
    about = "Control the rustWM tiling window manager"
)]
struct Cli {
    /// A command such as `focus left` or `workspace 2`.
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.command.is_empty() {
        println!(
            "rwm {}: nothing to do (try `rwm --help`)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    anyhow::bail!(
        "the IPC client is not implemented yet; it would send `{}` to {}",
        cli.command.join(" "),
        rwm_ipc::PIPE_NAME
    )
}
