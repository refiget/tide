mod app;
mod block;
mod config;
mod pty;
mod shell_hooks;
mod ui;

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

    pty::run_shell(&config)
}
