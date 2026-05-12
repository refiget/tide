mod ansi;
mod app;
mod block;
mod block_export;
mod buffer;
mod cli;
mod compositor;
mod config;
mod format;
mod index;
mod pty;
mod renderer;
mod shell_hooks;
mod theme;

use anyhow::Result;
use tracing::debug;

use crate::cli::{CliCommand, parse_cli_command, run_export_command};
use crate::config::Config;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_level(false)
        .without_time()
        .init();

    match parse_cli_command()? {
        CliCommand::RunShell => {
            let config = Config::load()?;
            debug!(?config, "loaded config");
            pty::run_shell(&config)
        }
        CliCommand::Export(args) => {
            run_export_command(args);
            Ok(())
        }
    }
}
