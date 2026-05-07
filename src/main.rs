mod app;
mod config;
mod pty;

use anyhow::Result;
use tracing::debug;

use crate::config::Config;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_level(false)
        .without_time()
        .init();

    let config = Config::load()?;
    debug!(?config, "loaded config");

    pty::run_shell(&config.shell.program)
}
