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
    renderer::TermRenderer,
    shell_hooks::{Osc777Parser, ParsedPtyPart, ShellHookEvent, TempHookFiles},
    ui,
};

fn block_header_bytes(block_id: u64, command: &str) -> Vec<u8> {
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let width = cols as usize;
    let label = format!("#{block_id} · {command}");
    let label: String = label.chars().take(width.saturating_sub(7)).collect();
    let fill_len = width.saturating_sub(5 + label.chars().count()).max(1);
    format!(
        "\r\n┌─ {} {}┐\r\n",
        label,
        "─".repeat(fill_len)
    )
    .into_bytes()
}

fn block_footer_bytes(block_id: u64, exit_code: i32, duration_ms: Option<u64>) -> Vec<u8> {
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let width = cols as usize;
    let status = if exit_code == 0 { "ok" } else { "failed" };
    let label = format!(
        "#{block_id} · {status} · exit {exit_code} · {}",
        format_duration_ms(duration_ms)
    );
    let label: String = label.chars().take(width.saturating_sub(7)).collect();
    let fill_len = width.saturating_sub(5 + label.chars().count()).max(1);
    format!(
        "└─ {} {}┘\r\n",
        label,
        "─".repeat(fill_len)
    )
    .into_bytes()
}

pub fn run_shell(config: &Config) -> Result<()> {
    let _terminal_guard = TerminalGuard::enter()?;

    let hook_files = TempHookFiles::new().context("failed to create tide hook files")?;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(current_pty_size())
        .context("failed to open PTY")?;

    let mut command = CommandBuilder::new(&config.shell.program);
    command.env(
        "TERM",
        std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string()),
    );
    command.env(
        "ZDOTDIR",
        hook_files
            .zdotdir()
            .to_str()
            .context("hook directory path is not valid UTF-8")?,
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

    let master = Arc::new(Mutex::new(pair.master));
    let running = Arc::new(AtomicBool::new(true));
    let blocks = Arc::new(Mutex::new(BlockStore::new(
        std::env::current_dir().unwrap_or_else(|_| ".".into()),
        config.blocks.max_blocks,
        config.blocks.max_output_bytes_per_block,
    )));
    let debug_blocks = std::env::var_os("TIDE_DEBUG_BLOCKS").is_some();

    let (cols, rows) = terminal::size().unwrap_or((80, 24));

    let output_running = Arc::clone(&running);
    let output_blocks = Arc::clone(&blocks);
    let output_thread = thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buffer = [0_u8; 8192];
        let mut parser = Osc777Parser::default();
        let mut renderer = TermRenderer::new(rows, cols);

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
                                renderer.process(&visible);
                            }
                            ParsedPtyPart::Event(event) => {
                                if let Ok(mut blocks) = output_blocks.lock() {
                                    apply_shell_hook_event(
                                        &mut blocks,
                                        event,
                                        debug_blocks,
                                        &mut renderer,
                                    );
                                }
                            }
                        }
                    }

                    if renderer.render(&mut stdout).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let remaining = parser.flush_visible();
        if !remaining.is_empty() {
            renderer.process(&remaining);
            let _ = renderer.render(&mut stdout);
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

fn apply_shell_hook_event(
    blocks: &mut BlockStore,
    event: ShellHookEvent,
    debug_blocks: bool,
    renderer: &mut TermRenderer,
) {
    match event {
        ShellHookEvent::Preexec { command } => {
            blocks.start_command(command.clone());
            if let Some(block_id) = blocks.active_block_id() {
                let header = block_header_bytes(block_id, &command);
                renderer.process(&header);
            }
        }
        ShellHookEvent::Precmd { exit_code } => {
            let active_id = blocks.active_block_id();
            blocks.finish_command(exit_code);
            if let Some(id) = active_id {
                let duration_ms = blocks.block(id).and_then(|b| b.duration_ms);
                let footer = block_footer_bytes(id, exit_code, duration_ms);
                renderer.process(&footer);
                if debug_blocks {
                    if let Some(block) = blocks.block(id) {
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
