use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use anyhow::{Context, Result};
use crossterm::terminal;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use signal_hook::{consts::signal::SIGWINCH, iterator::Signals};

use crate::{
    block::{BlockStore, format_duration_ms},
    config::Config,
    shell_hooks::{Osc777Parser, ParsedPtyPart, ShellHookEvent, install_script},
    ui,
};

pub fn run_shell(config: &Config) -> Result<()> {
    let _terminal_guard = TerminalGuard::enter()?;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(current_pty_size())
        .context("failed to open PTY")?;

    let mut command = CommandBuilder::new(&config.shell.program);
    command.env(
        "TERM",
        std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string()),
    );

    let mut child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("failed to spawn shell `{}`", config.shell.program))?;

    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .context("failed to clone PTY reader")?;
    let mut writer = pair
        .master
        .take_writer()
        .context("failed to take PTY writer")?;
    writer
        .write_all(hook_install_command().as_bytes())
        .context("failed to install zsh hooks")?;
    writer.flush().context("failed to flush zsh hook install")?;

    let master = Arc::new(Mutex::new(pair.master));
    let running = Arc::new(AtomicBool::new(true));
    let blocks = Arc::new(Mutex::new(BlockStore::new(
        std::env::current_dir().unwrap_or_else(|_| ".".into()),
        config.blocks.max_blocks,
        config.blocks.max_output_bytes_per_block,
    )));
    let debug_blocks = std::env::var_os("TIDE_DEBUG_BLOCKS").is_some();

    let output_running = Arc::clone(&running);
    let output_blocks = Arc::clone(&blocks);
    let output_thread = thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buffer = [0_u8; 8192];
        let mut parser = Osc777Parser::default();

        while output_running.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let parsed = parser.push(&buffer[..n]);

                    for part in parsed {
                        match part {
                            ParsedPtyPart::Visible(visible) => {
                                if let Ok(mut blocks) = output_blocks.lock() {
                                    blocks.append_output(&visible);
                                }

                                if stdout.write_all(&visible).is_err() {
                                    break;
                                }
                                if stdout.flush().is_err() {
                                    break;
                                }
                            }
                            ParsedPtyPart::Event(event) => {
                                if let Ok(mut blocks) = output_blocks.lock() {
                                    apply_shell_hook_event(&mut blocks, event, debug_blocks);
                                }
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let remaining = parser.flush_visible();
        if !remaining.is_empty() {
            let _ = stdout.write_all(&remaining);
            let _ = stdout.flush();
        }
    });

    let input_running = Arc::clone(&running);
    let input_blocks = Arc::clone(&blocks);
    let _input_thread = thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buffer = [0_u8; 8192];
        let mut pending_ctrl_x = false;

        while input_running.load(Ordering::SeqCst) {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    for byte in &buffer[..n] {
                        if pending_ctrl_x {
                            pending_ctrl_x = false;

                            if *byte == 0x02 {
                                let _ = ui::run_block_mode(Arc::clone(&input_blocks));
                                continue;
                            }

                            if writer.write_all(&[0x18, *byte]).is_err() {
                                return;
                            }
                            continue;
                        }

                        if *byte == 0x18 {
                            pending_ctrl_x = true;
                            continue;
                        }

                        if writer.write_all(&[*byte]).is_err() {
                            return;
                        }
                    }

                    let _ = writer.flush();
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    let resize_running = Arc::clone(&running);
    let resize_master = Arc::clone(&master);
    let resize_thread = thread::spawn(move || {
        let Ok(mut signals) = Signals::new([SIGWINCH]) else {
            return;
        };

        for _ in signals.forever() {
            if !resize_running.load(Ordering::SeqCst) {
                break;
            }

            let size = current_pty_size();
            if let Ok(master) = resize_master.lock() {
                let _ = master.resize(size);
            }
        }
    });

    let status = child.wait().context("failed to wait for shell process")?;
    running.store(false, Ordering::SeqCst);

    drop(master);

    let _ = output_thread.join();
    let _ = signal_hook::low_level::raise(SIGWINCH);
    let _ = resize_thread.join();

    if !status.success() {
        std::process::exit(status.exit_code() as i32);
    }

    Ok(())
}

fn hook_install_command() -> String {
    format!("{}\n", install_script())
}

fn apply_shell_hook_event(blocks: &mut BlockStore, event: ShellHookEvent, debug_blocks: bool) {
    match event {
        ShellHookEvent::Preexec { command } => {
            blocks.start_command(command);
        }
        ShellHookEvent::Precmd { exit_code } => {
            let active_block_id = blocks.active_block_id();
            blocks.finish_command(exit_code);
            if debug_blocks {
                if let Some(block) = active_block_id.and_then(|id| blocks.block(id)) {
                    eprintln!(
                        "\r\ntide block #{} status={:?} exit={} duration={} command={:?} output_bytes={}\r",
                        block.id,
                        block.status,
                        block.exit_code.unwrap_or(-1),
                        format_duration_ms(block.duration_ms),
                        block.command,
                        block.output_raw.len()
                    );
                }
            }
        }
        ShellHookEvent::Cwd { cwd } => {
            blocks.set_cwd(cwd);
        }
    }
}

fn current_pty_size() -> PtySize {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));

    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable terminal raw mode")?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
