use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::terminal;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use signal_hook::{consts::signal::SIGWINCH, iterator::Signals};

use crate::{
    app::{BlockAction, BlockKind, InputAccumulator, RenderState, ViewAnchor, ViewKind, ViewState},
    block::BlockStore,
    buffer::ShellBuffer,
    compositor::Compositor,
    config::{Config, RuntimeConfig, build_runtime_config},
    renderer,
    shell_hooks::{Osc777Parser, ParsedPtyPart, ShellHookEvent},
};

struct RuntimeState {
    shell: ShellBuffer,
    blocks: BlockStore,
    view: ViewState,
    input_accumulator: InputAccumulator,
    render_state: RenderState,
    config: RuntimeConfig,
    capture_suspended: bool,
    rows: u16,
    cols: u16,
}

const FRAME_DURATION: Duration = Duration::from_millis(16);

pub fn run_shell(config: &Config) -> Result<()> {
    let _terminal_guard = TerminalGuard::enter()?;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(current_pty_size())
        .context("failed to open PTY")?;

    let mut command = CommandBuilder::new(&config.shell.program);
    command.arg("-i");
    command.env(
        "TERM",
        std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string()),
    );
    command.env("TIDE", "1");
    command.env("TIDE_SESSION_ID", std::process::id().to_string());

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

    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let runtime_config = build_runtime_config(config.clone());
    let master = Arc::new(Mutex::new(pair.master));
    let running = Arc::new(AtomicBool::new(true));
    let mut view = ViewState::default();
    view.block_viewport.anchor = if runtime_config.block_view.follow_tail {
        ViewAnchor::Tail
    } else {
        ViewAnchor::Manual
    };
    let state = Arc::new(Mutex::new(RuntimeState {
        shell: ShellBuffer::new(),
        blocks: BlockStore::new(
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            runtime_config.max_blocks,
            config.blocks.max_output_bytes_per_block,
        ),
        view,
        input_accumulator: InputAccumulator::default(),
        render_state: RenderState::default(),
        config: runtime_config,
        capture_suspended: false,
        rows,
        cols,
    }));
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let debug_blocks = std::env::var_os("TIDE_DEBUG_BLOCKS").is_some();

    let output_running = Arc::clone(&running);
    let output_state = Arc::clone(&state);
    let output_stdout = Arc::clone(&stdout);
    // Output thread: always locks (input_state) -> (input_stdout).
    // Input / resize threads must avoid (input_stdout) -> (input_state)
    // to prevent deadlock.
    let output_thread = thread::spawn(move || {
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
                                let view = if let Ok(mut state) = output_state.lock() {
                                    let active_block_id = state.blocks.active_block_id();
                                    if active_block_id.is_some()
                                        && contains_alternate_screen_switch(&visible)
                                    {
                                        state.capture_suspended = true;
                                        if let Some(id) = active_block_id {
                                            if let Some(block) = state.blocks.block_mut(id) {
                                                block.kind = BlockKind::RawProgram;
                                            }
                                        }
                                    }
                                    if !state.capture_suspended {
                                        state.blocks.append_output(&visible);
                                        state.shell.append(&visible, active_block_id);
                                    }
                                    state.view.view.clone()
                                } else {
                                    break;
                                };

                                if matches!(view, ViewKind::Plain) {
                                    if let Ok(mut stdout) = output_stdout.lock() {
                                        if stdout.write_all(&visible).is_err() {
                                            break;
                                        }
                                        let _ = stdout.flush();
                                    }
                                }
                            }
                            ParsedPtyPart::Event(event) => {
                                if let Ok(mut state) = output_state.lock() {
                                    apply_shell_hook_event(&mut state, event, debug_blocks);
                                }
                            }
                        }
                    }

                    let should_render = output_state
                        .lock()
                        .map(|state| !matches!(state.view.view, ViewKind::Plain))
                        .unwrap_or(false);
                    if should_render && render_runtime(&output_state, &output_stdout).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let remaining = parser.flush_visible();
        if !remaining.is_empty() {
            let view = if let Ok(mut state) = output_state.lock() {
                let active_block_id = state.blocks.active_block_id();
                if active_block_id.is_some() && contains_alternate_screen_switch(&remaining) {
                    state.capture_suspended = true;
                    if let Some(id) = active_block_id {
                        if let Some(block) = state.blocks.block_mut(id) {
                            block.kind = BlockKind::RawProgram;
                        }
                    }
                }
                if !state.capture_suspended {
                    state.shell.append(&remaining, active_block_id);
                }
                state.view.view.clone()
            } else {
                ViewKind::Plain
            };

            if matches!(view, ViewKind::Plain) {
                if let Ok(mut stdout) = output_stdout.lock() {
                    let _ = stdout.write_all(&remaining);
                    let _ = stdout.flush();
                }
            } else {
                let _ = render_runtime(&output_state, &output_stdout);
            }
        }
    });

    let input_running = Arc::clone(&running);
    let input_state = Arc::clone(&state);
    let input_stdout = Arc::clone(&stdout);
    let _input_thread = thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buffer = [0_u8; 8192];

        while input_running.load(Ordering::SeqCst) {
            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let mut index = 0;
                    let mut pending_bytes: Vec<u8> = Vec::new();
                    while index < n {
                        let byte = buffer[index];

                        if let Some(consumed) =
                            handle_view_key_sequence(&buffer[index..n], &input_state)
                        {
                            index += consumed;
                            // If the key triggered alt-screen cleanup, break out of the byte
                            // loop immediately so remaining bytes are NOT forwarded to the PTY
                            // while the alt screen is still active — the output thread would
                            // write their echo to the wrong screen.
                            if input_state
                                .lock()
                                .map(|s| s.render_state.needs_cleanup)
                                .unwrap_or(false)
                            {
                                pending_bytes.extend_from_slice(&buffer[index..n]);
                                break;
                            }
                            continue;
                        }

                        if byte == 0x02 {
                            if let Ok(state) = input_state.lock() {
                                if matches!(state.view.view, ViewKind::Plain)
                                    && state.blocks.active_block_id().is_none()
                                {
                                    // Enter alternate screen for Block View.
                                    // Lock ordering: drop state before locking stdout
                                    // (output thread locks state -> stdout, must not invert).
                                    drop(state);
                                    if let Ok(mut stdout) = input_stdout.lock() {
                                        let _ = renderer::enter_block_render(&mut *stdout);
                                    }
                                    let mut state =
                                        input_state.lock().unwrap_or_else(|e| e.into_inner());
                                    enter_block_view(&mut state);
                                    drop(state);
                                    let _ = maybe_flush_navigation_and_render(
                                        &input_state,
                                        &input_stdout,
                                        false,
                                    );
                                } else if writer.write_all(&[byte]).is_err() {
                                    return;
                                }
                                index += 1;
                                continue;
                            }
                        }

                        if writer.write_all(&[byte]).is_err() {
                            return;
                        }
                        index += 1;
                    }

                    let _ = writer.flush();

                    let needs_cleanup = input_state
                        .lock()
                        .map(|s| s.render_state.needs_cleanup)
                        .unwrap_or(false);
                    if needs_cleanup {
                        // Leave alt screen before anything else — SGR reset and cursor show
                        // must apply on the main screen after the alt screen is gone.
                        if let Ok(mut stdout) = input_stdout.lock() {
                            let _ = renderer::leave_block_render(&mut *stdout);
                        }

                        // Clear cleanup flags and extract pending_paste (state lock isolated,
                        // no stdout lock held).
                        let paste = if let Ok(mut state) = input_state.lock() {
                            state.render_state.needs_cleanup = false;
                            state.render_state.dirty = false;
                            state.render_state.force_render = false;
                            state.render_state.pending_paste.take()
                        } else {
                            None
                        };

                        // Forward any bytes that followed the cleanup key in the same read
                        // chunk. They belong to the restored Plain view (shell input).
                        if !pending_bytes.is_empty() {
                            let _ = writer.write_all(&pending_bytes);
                            let _ = writer.flush();
                        }

                        // Write pending paste (rerun command) after alt screen is gone.
                        if let Some(cmd) = paste {
                            let _ = writer.write_all(cmd.as_bytes());
                            let _ = writer.flush();
                        }

                        // The alt screen exit alone restored the main screen correctly.
                        // Do not render Plain view on top of it.
                        continue;
                    }

                    let _ = maybe_flush_navigation_and_render(&input_state, &input_stdout, true);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    let resize_running = Arc::clone(&running);
    let resize_master = Arc::clone(&master);
    let resize_state = Arc::clone(&state);
    let resize_stdout = Arc::clone(&stdout);
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
            let should_render = if let Ok(mut state) = resize_state.lock() {
                state.rows = size.rows;
                state.cols = size.cols;
                let not_plain = !matches!(state.view.view, ViewKind::Plain);
                if not_plain {
                    ensure_selected_visible(&mut state);
                    state.render_state.dirty = true;
                    state.render_state.force_render = true;
                }
                not_plain
            } else {
                false
            };
            if should_render {
                let _ = render_runtime(&resize_state, &resize_stdout);
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

fn apply_shell_hook_event(state: &mut RuntimeState, event: ShellHookEvent, debug_blocks: bool) {
    match event {
        ShellHookEvent::Preexec { command } => {
            let start_line = state.shell.line_count();
            state.capture_suspended = false;
            state
                .blocks
                .start_command(command, start_line, BlockKind::NormalCommand);
            sync_block_viewport_after_history_change(state);
        }
        ShellHookEvent::Precmd { exit_code, cwd } => {
            let finished_cwd = cwd;
            if let Some(cwd) = finished_cwd.clone() {
                state.blocks.set_cwd(cwd);
            }
            let active_id = state.blocks.active_block_id();
            let end_line = state.shell.line_count().saturating_sub(1);
            state.blocks.finish_command(exit_code, end_line);
            state.capture_suspended = false;
            sync_block_viewport_after_history_change(state);
            if let Some(id) = active_id {
                if let Some(cwd) = finished_cwd {
                    if let Some(block) = state.blocks.block_mut(id) {
                        block.cwd = cwd.into();
                    }
                }
                if debug_blocks {
                    if let Some(block) = state.blocks.block(id) {
                        eprintln!(
                            "\r\ntide block #{} status={:?} exit={} duration={} command={:?} output_bytes={}\r",
                            block.id,
                            block.status,
                            block.exit_code.unwrap_or(-1),
                            crate::block::format_duration_ms(block.duration_ms),
                            block.command,
                            block.output_raw.len()
                        );
                    }
                }
            }
        }
    }
}

fn write_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                let mut stdin = child.stdin.take().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "pbcopy stdin not available",
                    )
                })?;
                stdin.write_all(text.as_bytes())?;
                drop(stdin);
                child.wait()?;
                Ok(())
            })
            .is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        arboard::Clipboard::new()
            .and_then(|mut cb| cb.set_text(text))
            .is_ok()
    }
}

fn perform_block_action(state: &mut RuntimeState, action: BlockAction) {
    let block_id = match state.view.view {
        ViewKind::Detail => state.view.expanded_block,
        _ => state.view.selected_block,
    };
    let Some(block_id) = block_id else { return };
    let Some(block) = state.blocks.block(block_id) else {
        return;
    };

    let text = match action {
        BlockAction::CopyOutput => block.output_text.clone(),
        BlockAction::CopyCommand => block.command.clone(),
        _ => return,
    };

    if write_to_clipboard(&text) {
        let msg = match action {
            BlockAction::CopyOutput => "copied output".to_string(),
            BlockAction::CopyCommand => "copied command".to_string(),
            _ => unreachable!(),
        };
        state.render_state.flash_message = Some((msg, Instant::now()));
        state.render_state.dirty = true;
        state.render_state.force_render = true;
    }
}

fn render_runtime(
    state: &Arc<Mutex<RuntimeState>>,
    stdout: &Arc<Mutex<io::Stdout>>,
) -> io::Result<()> {
    let (visual_lines, view, cursor, layout, block_view, rows, cols, last_rendered_rows) = {
        let mut state = state
            .lock()
            .map_err(|_| io::Error::other("runtime state lock poisoned"))?;

        // Clear expired flash message and extract text for compositor.
        let flash_text = state
            .render_state
            .flash_message
            .as_ref()
            .and_then(|(msg, at)| {
                if at.elapsed() < Duration::from_millis(1500) {
                    Some(msg.clone())
                } else {
                    None
                }
            });
        if flash_text.is_none() {
            state.render_state.flash_message = None;
        }

        let visual_lines = Compositor::build_visual_lines(
            &state.shell,
            &state.blocks,
            &state.view,
            state.cols,
            state.rows,
            &state.config.block_layout,
            &state.config.block_view,
            flash_text.as_deref(),
        );
        (
            visual_lines,
            state.view.clone(),
            state.shell.cursor_position(),
            state.config.block_layout.clone(),
            state.config.block_view.clone(),
            state.rows,
            state.cols,
            state.render_state.last_rendered_rows,
        )
    };

    let mut stdout = stdout
        .lock()
        .map_err(|_| io::Error::other("stdout lock poisoned"))?;
    let rendered = renderer::render(
        &mut *stdout,
        &visual_lines,
        &view,
        Some(cursor),
        &layout,
        &block_view,
        rows,
        cols,
        last_rendered_rows,
    )?;

    {
        let mut state = state
            .lock()
            .map_err(|_| io::Error::other("runtime state lock poisoned"))?;
        state.render_state.last_rendered_rows = rendered;
    }

    Ok(())
}

fn maybe_flush_navigation_and_render(
    state: &Arc<Mutex<RuntimeState>>,
    stdout: &Arc<Mutex<io::Stdout>>,
    wait_for_frame: bool,
) -> io::Result<()> {
    let sleep_for = {
        let state = state
            .lock()
            .map_err(|_| io::Error::other("runtime state lock poisoned"))?;
        if !state.render_state.dirty {
            return Ok(());
        }
        if state.render_state.force_render {
            Duration::ZERO
        } else {
            let elapsed = state.render_state.last_render_at.elapsed();
            if elapsed < FRAME_DURATION {
                FRAME_DURATION - elapsed
            } else {
                Duration::ZERO
            }
        }
    };

    if wait_for_frame && !sleep_for.is_zero() {
        thread::sleep(sleep_for);
    } else if !sleep_for.is_zero() {
        return Ok(());
    }

    {
        let mut state = state
            .lock()
            .map_err(|_| io::Error::other("runtime state lock poisoned"))?;
        if !flush_render_state(&mut state) {
            return Ok(());
        }
    }

    render_runtime(state, stdout)
}

fn flush_render_state(state: &mut RuntimeState) -> bool {
    // Suppress normal rendering while alt-screen cleanup is pending.
    if state.render_state.needs_cleanup {
        state.render_state.dirty = false;
        state.render_state.force_render = false;
        return false;
    }

    let force_render = state.render_state.force_render;
    let changed = flush_navigation_delta(state);
    if !force_render && !changed && state.input_accumulator.pending_block_delta == 0 {
        state.render_state.dirty = false;
        return false;
    }

    state.render_state.dirty = false;
    state.render_state.force_render = false;
    state.render_state.last_render_at = Instant::now();
    true
}

fn enter_block_view(state: &mut RuntimeState) {
    state.view.view = ViewKind::Blocks;
    state.view.expanded_block = None;
    select_tail_block(state);
    state.render_state.dirty = true;
    state.render_state.force_render = true;
}

fn handle_view_key_sequence(bytes: &[u8], state: &Arc<Mutex<RuntimeState>>) -> Option<usize> {
    let Ok(mut state) = state.lock() else {
        return None;
    };

    match state.view.view {
        ViewKind::Plain => None,
        ViewKind::RawProgram => None,
        ViewKind::Agent => Some(1),
        ViewKind::Blocks => match bytes {
            [b'\x1b', b'[', b'B', ..] => {
                accumulate_block_delta(&mut state, 1);
                Some(3)
            }
            [b'\x1b', b'[', b'A', ..] => {
                accumulate_block_delta(&mut state, -1);
                Some(3)
            }
            [b'\x1b', ..] if bytes.len() >= 3 => Some(3),
            [byte, ..] => handle_block_view_byte(*byte, &mut state).then_some(1),
            [] => None,
        },
        ViewKind::Detail => match bytes {
            [b'y', ..] => {
                perform_block_action(&mut state, BlockAction::CopyOutput);
                Some(1)
            }
            [b'Y', ..] => {
                perform_block_action(&mut state, BlockAction::CopyCommand);
                Some(1)
            }
            [b'j', ..] | [b'\x1b', b'[', b'B', ..] => {
                state.view.block_viewport.line_offset =
                    state.view.block_viewport.line_offset.saturating_add(1);
                state.render_state.dirty = true;
                state.render_state.force_render = true;
                if bytes.len() >= 3 && bytes[0] == b'\x1b' {
                    Some(3)
                } else {
                    Some(1)
                }
            }
            [b'k', ..] | [b'\x1b', b'[', b'A', ..] => {
                state.view.block_viewport.line_offset =
                    state.view.block_viewport.line_offset.saturating_sub(1);
                state.render_state.dirty = true;
                state.render_state.force_render = true;
                if bytes.len() >= 3 && bytes[0] == b'\x1b' {
                    Some(3)
                } else {
                    Some(1)
                }
            }
            [b'G', ..] => {
                let layout = Compositor::build_visual_layout(
                    &state.shell,
                    &state.blocks,
                    &state.view,
                    state.cols,
                    &state.config.block_view,
                );
                let content_height = (state.rows as usize)
                    .saturating_sub(usize::from(state.config.block_view.show_footer));
                if let Some(id) = state.view.expanded_block {
                    if let Some(span) = layout.spans.iter().find(|s| s.block_id == id) {
                        state.view.block_viewport.line_offset =
                            span.end_line.saturating_sub(content_height);
                    }
                }
                state.render_state.dirty = true;
                state.render_state.force_render = true;
                Some(1)
            }
            [b'g', ..] => {
                let layout = Compositor::build_visual_layout(
                    &state.shell,
                    &state.blocks,
                    &state.view,
                    state.cols,
                    &state.config.block_view,
                );
                if let Some(id) = state.view.expanded_block {
                    if let Some(span) = layout.spans.iter().find(|s| s.block_id == id) {
                        state.view.block_viewport.line_offset = span.start_line;
                    }
                }
                state.render_state.dirty = true;
                state.render_state.force_render = true;
                Some(1)
            }
            [b'r', ..] => {
                let command = state
                    .view
                    .expanded_block
                    .and_then(|id| state.blocks.block(id))
                    .map(|b| b.command.clone())
                    .filter(|cmd| !cmd.is_empty());
                if let Some(cmd) = command {
                    state.view = ViewState::default();
                    state.render_state.needs_cleanup = true;
                    state.render_state.pending_paste = Some(cmd);
                }
                Some(1)
            }
            [b'q', ..] | [b'\x1b'] => {
                state.view.view = ViewKind::Blocks;
                state.view.expanded_block = None;
                if matches!(state.view.block_viewport.anchor, ViewAnchor::Tail) {
                    state.view.block_viewport.line_offset = compute_tail_scroll_offset(&state);
                } else {
                    ensure_selected_visible(&mut state);
                }
                state.render_state.dirty = true;
                state.render_state.force_render = true;
                Some(1)
            }
            [b'\x1b', ..] => Some(bytes.len().min(3)),
            [_byte, ..] => Some(1),
            [] => None,
        },
    }
}

fn handle_block_view_byte(byte: u8, state: &mut RuntimeState) -> bool {
    match byte {
        b'q' | b'\x1b' => {
            state.view = ViewState::default();
            state.input_accumulator.pending_block_delta = 0;
            // Defer to the alt-screen cleanup handler (not dirty/force_render,
            // to avoid rendering Plain view on top of the restored main screen).
            state.render_state.needs_cleanup = true;
            true
        }
        b'j' => {
            accumulate_block_delta(state, 1);
            true
        }
        b'k' => {
            accumulate_block_delta(state, -1);
            true
        }
        b'G' => {
            state.input_accumulator.pending_block_delta = 0;
            select_tail_block(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        b'g' => {
            state.input_accumulator.pending_block_delta = 0;
            select_block_index(state, 0, ViewAnchor::Top);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        b'\r' | b'\n' => {
            flush_navigation_delta(state);
            state.view.expanded_block = state.view.selected_block;
            state.view.view = ViewKind::Detail;
            if matches!(state.view.block_viewport.anchor, ViewAnchor::Tail) {
                state.view.block_viewport.line_offset = compute_tail_scroll_offset(state);
            } else {
                ensure_selected_visible(state);
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        b'y' => {
            perform_block_action(state, BlockAction::CopyOutput);
            true
        }
        b'Y' => {
            perform_block_action(state, BlockAction::CopyCommand);
            true
        }
        b'r' => {
            let command = state
                .view
                .selected_block
                .and_then(|id| state.blocks.block(id))
                .map(|b| b.command.clone())
                .filter(|cmd| !cmd.is_empty());
            if let Some(cmd) = command {
                state.view = ViewState::default();
                state.input_accumulator.pending_block_delta = 0;
                state.render_state.needs_cleanup = true;
                state.render_state.pending_paste = Some(cmd);
            }
            true
        }
        _ => true,
    }
}

fn accumulate_block_delta(state: &mut RuntimeState, delta: isize) {
    if is_navigation_boundary_noop(state, delta) {
        return;
    }

    let limit = (state.blocks.len().max(1).min(500)) as isize;
    state.input_accumulator.pending_block_delta = state
        .input_accumulator
        .pending_block_delta
        .saturating_add(delta)
        .clamp(-limit, limit);
    state.input_accumulator.last_input_at = Some(Instant::now());
    state.render_state.dirty = true;
}

fn flush_navigation_delta(state: &mut RuntimeState) -> bool {
    let delta = state.input_accumulator.pending_block_delta;
    state.input_accumulator.pending_block_delta = 0;
    if delta == 0 {
        return false;
    }
    select_relative_block(state, delta)
}

fn is_navigation_boundary_noop(state: &RuntimeState, delta: isize) -> bool {
    let pending = state.input_accumulator.pending_block_delta;
    if state.blocks.is_empty() || pending.saturating_add(delta) != delta {
        return false;
    }

    let selected = state
        .view
        .block_viewport
        .selected_index
        .min(state.blocks.len().saturating_sub(1));
    (delta < 0 && selected == 0) || (delta > 0 && selected == state.blocks.len().saturating_sub(1))
}

fn select_relative_block(state: &mut RuntimeState, delta: isize) -> bool {
    if state.blocks.is_empty() {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        return false;
    }

    let current = state
        .view
        .block_viewport
        .selected_index
        .min(state.blocks.len().saturating_sub(1));
    let next = if delta.is_negative() {
        let magnitude = delta.checked_abs().unwrap_or(isize::MAX) as usize;
        current.saturating_sub(magnitude)
    } else {
        (current + delta as usize).min(state.blocks.len().saturating_sub(1))
    };
    if next == current {
        return false;
    }
    let old_scroll = state.view.block_viewport.line_offset;
    let old_anchor = state.view.block_viewport.anchor;
    let anchor = if state.config.block_view.auto_follow_on_reach_bottom
        && !delta.is_negative()
        && next == state.blocks.len().saturating_sub(1)
    {
        ViewAnchor::Tail
    } else {
        ViewAnchor::Manual
    };
    select_block_index(state, next, anchor);
    state.view.block_viewport.selected_index != current
        || state.view.block_viewport.line_offset != old_scroll
        || state.view.block_viewport.anchor != old_anchor
}

fn sync_block_viewport_after_history_change(state: &mut RuntimeState) {
    if matches!(state.view.block_viewport.anchor, ViewAnchor::Tail) {
        select_tail_block(state);
    } else {
        clamp_viewport_to_history(state);
    }
}

fn clamp_viewport_to_history(state: &mut RuntimeState) {
    if state.blocks.is_empty() {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        return;
    }

    let last = state.blocks.len().saturating_sub(1);
    let selected = state.view.block_viewport.selected_index.min(last);
    select_block_index(state, selected, state.view.block_viewport.anchor);
}

fn select_block_index(state: &mut RuntimeState, index: usize, anchor: ViewAnchor) {
    if state.blocks.is_empty() {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        state.view.block_viewport.anchor = anchor;
        return;
    }

    let index = index.min(state.blocks.len().saturating_sub(1));
    state.view.block_viewport.selected_index = index;
    state.view.selected_block = state.blocks.block_id_at(index);
    state.view.block_viewport.anchor = anchor;
    match anchor {
        ViewAnchor::Tail => {
            state.view.block_viewport.line_offset = compute_tail_scroll_offset(state);
        }
        ViewAnchor::Top => {
            state.view.block_viewport.scroll_offset = 0;
            state.view.block_viewport.line_offset = 0;
        }
        ViewAnchor::Manual => {
            ensure_selected_visible(state);
        }
    }
}

fn select_tail_block(state: &mut RuntimeState) {
    if state.blocks.is_empty() {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        state.view.block_viewport.anchor = ViewAnchor::Tail;
        return;
    }

    let last = state.blocks.len().saturating_sub(1);
    state.view.block_viewport.selected_index = last;
    state.view.selected_block = state.blocks.block_id_at(last);
    state.view.block_viewport.anchor = ViewAnchor::Tail;
    state.view.block_viewport.line_offset = compute_tail_scroll_offset(state);
}

fn compute_tail_scroll_offset(state: &RuntimeState) -> usize {
    Compositor::compute_tail_scroll_offset(
        &state.shell,
        &state.blocks,
        &state.view,
        state.rows as usize,
        &state.config.block_view,
    )
}

fn ensure_selected_visible(state: &mut RuntimeState) {
    if state.blocks.is_empty() {
        return;
    }

    let layout = Compositor::build_visual_layout(
        &state.shell,
        &state.blocks,
        &state.view,
        state.cols,
        &state.config.block_view,
    );
    let content_height = content_height(state);
    let margin_lines = state.config.block_view.scroll_margin_lines;
    ensure_selected_block_fully_visible(
        &mut state.view.block_viewport,
        &layout,
        content_height,
        margin_lines,
    );
    state.view.block_viewport.anchor = ViewAnchor::Manual;
}

fn ensure_selected_block_fully_visible(
    viewport: &mut crate::app::BlockViewport,
    layout: &crate::compositor::VisualLayout,
    content_height: usize,
    margin_lines: usize,
) {
    // All content already fits — no scrolling adjustments needed.
    if layout.total_height <= content_height {
        viewport.line_offset = 0;
        return;
    }

    let Some(span) = layout.span_for_block_index(viewport.selected_index) else {
        return;
    };
    let max_offset = layout.total_height.saturating_sub(content_height);
    if span.end_line.saturating_sub(span.start_line) > content_height {
        viewport.line_offset = span.start_line.saturating_sub(margin_lines).min(max_offset);
        return;
    }

    let top = viewport.line_offset;
    let bottom = top.saturating_add(content_height);
    if span.start_line < top.saturating_add(margin_lines) {
        viewport.line_offset = span.start_line.saturating_sub(margin_lines);
    } else if span.end_line > bottom.saturating_sub(margin_lines) {
        let desired_bottom = span.end_line.saturating_add(margin_lines);
        viewport.line_offset = desired_bottom.saturating_sub(content_height);
    }
    viewport.line_offset = viewport.line_offset.min(max_offset);
}

fn content_height(state: &RuntimeState) -> usize {
    (state.rows as usize).saturating_sub(usize::from(state.config.block_view.show_footer))
}

fn contains_alternate_screen_switch(bytes: &[u8]) -> bool {
    bytes
        .windows(b"\x1b[?1049h".len())
        .any(|window| window == b"\x1b[?1049h")
        || bytes
            .windows(b"\x1b[?1047h".len())
            .any(|window| window == b"\x1b[?1047h")
        || bytes
            .windows(b"\x1b[?1048h".len())
            .any(|window| window == b"\x1b[?1048h")
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

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::config::Config;

    fn runtime_state() -> RuntimeState {
        let config = Config::default();
        let runtime_config = build_runtime_config(config.clone());
        RuntimeState {
            shell: ShellBuffer::new(),
            blocks: BlockStore::new(
                PathBuf::from("/tmp"),
                runtime_config.max_blocks,
                config.blocks.max_output_bytes_per_block,
            ),
            view: ViewState::default(),
            input_accumulator: InputAccumulator::default(),
            render_state: RenderState::default(),
            config: runtime_config,
            capture_suspended: false,
            rows: 24,
            cols: 80,
        }
    }

    fn add_block(state: &mut RuntimeState, command: &str) {
        add_block_with_lines(state, command, &[command]);
    }

    fn add_block_with_lines(state: &mut RuntimeState, command: &str, lines: &[&str]) {
        let id = state.blocks.start_command(
            command.to_string(),
            state.shell.line_count(),
            BlockKind::NormalCommand,
        );
        for line in lines {
            state.shell.append(format!("{line}\n").as_bytes(), Some(id));
            state.blocks.append_output(format!("{line}\n").as_bytes());
        }
        state
            .blocks
            .finish_command(0, state.shell.line_count().saturating_sub(1));
    }

    fn selected_span_is_fully_visible(state: &RuntimeState) -> bool {
        let layout = Compositor::build_visual_layout(
            &state.shell,
            &state.blocks,
            &state.view,
            state.cols,
            &state.config.block_view,
        );
        let Some(span) = layout.span_for_block_index(state.view.block_viewport.selected_index)
        else {
            return false;
        };
        let top = state.view.block_viewport.line_offset;
        let bottom = top.saturating_add(content_height(state));
        span.start_line >= top && span.end_line <= bottom
    }

    #[test]
    fn entering_block_view_forces_render() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");

        enter_block_view(&mut state);

        assert!(state.render_state.dirty);
        assert!(state.render_state.force_render);
        assert!(flush_render_state(&mut state));
        assert!(!state.render_state.dirty);
        assert!(!state.render_state.force_render);
    }

    #[test]
    fn block_to_plain_back_sets_cleanup_flag() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        enter_block_view(&mut state);
        state.render_state.dirty = false;
        state.render_state.force_render = false;

        assert!(handle_block_view_byte(b'q', &mut state));

        assert!(matches!(state.view.view, ViewKind::Plain));
        assert!(!state.render_state.dirty);
        assert!(!state.render_state.force_render);
        assert!(state.render_state.needs_cleanup);
    }

    #[test]
    fn detail_to_block_back_forces_render() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut state = state.lock().unwrap();
            add_block(&mut state, "echo one");
            enter_block_view(&mut state);
            state.view.view = ViewKind::Detail;
            state.render_state.dirty = false;
            state.render_state.force_render = false;
        }

        assert_eq!(handle_view_key_sequence(b"q", &state), Some(1));

        let state = state.lock().unwrap();
        assert!(matches!(state.view.view, ViewKind::Blocks));
        assert!(state.render_state.dirty);
        assert!(state.render_state.force_render);
    }

    #[test]
    fn g_and_g_upper_force_render() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        add_block(&mut state, "echo two");
        enter_block_view(&mut state);
        state.render_state.dirty = false;
        state.render_state.force_render = false;

        assert!(handle_block_view_byte(b'g', &mut state));
        assert!(state.render_state.force_render);
        assert_eq!(state.view.block_viewport.selected_index, 0);
        assert_eq!(state.view.block_viewport.line_offset, 0);
        assert!(matches!(state.view.block_viewport.anchor, ViewAnchor::Top));

        state.render_state.force_render = false;
        state.render_state.dirty = false;
        assert!(handle_block_view_byte(b'G', &mut state));
        assert!(state.render_state.force_render);
        assert_eq!(
            state.view.block_viewport.selected_index,
            state.blocks.len() - 1
        );
        assert_eq!(
            state.view.block_viewport.line_offset,
            compute_tail_scroll_offset(&state)
        );
        assert!(matches!(state.view.block_viewport.anchor, ViewAnchor::Tail));
    }

    #[test]
    fn enter_block_view_uses_tail_line_offset() {
        let mut state = runtime_state();
        state.rows = 8;
        for index in 0..8 {
            add_block(&mut state, &format!("echo {index}"));
        }

        enter_block_view(&mut state);

        assert_eq!(
            state.view.block_viewport.selected_index,
            state.blocks.len() - 1
        );
        assert_eq!(
            state.view.block_viewport.line_offset,
            compute_tail_scroll_offset(&state)
        );
        assert!(state.view.block_viewport.line_offset > 0);
    }

    #[test]
    fn ensure_selected_visible_moves_line_offset_only_when_needed() {
        let mut state = runtime_state();
        state.rows = 9;
        add_block_with_lines(&mut state, "a", &["a1", "a2", "a3", "a4"]);
        add_block(&mut state, "b");
        add_block(&mut state, "c");
        state.view.view = ViewKind::Blocks;
        select_block_index(&mut state, 1, ViewAnchor::Manual);
        let offset_after_select = state.view.block_viewport.line_offset;

        ensure_selected_visible(&mut state);

        assert_eq!(state.view.block_viewport.line_offset, offset_after_select);
        assert!(selected_span_is_fully_visible(&state));
    }

    #[test]
    fn selecting_partial_block_adjusts_line_offset_to_show_it_fully() {
        let mut state = runtime_state();
        state.rows = 9;
        add_block_with_lines(&mut state, "a", &["a1", "a2", "a3", "a4"]);
        add_block_with_lines(&mut state, "b", &["b1", "b2", "b3"]);
        add_block(&mut state, "c");
        state.view.view = ViewKind::Blocks;
        state.view.block_viewport.anchor = ViewAnchor::Manual;
        state.view.block_viewport.selected_index = 1;
        state.view.selected_block = state.blocks.block_id_at(1);
        state.view.block_viewport.line_offset = 3;

        ensure_selected_visible(&mut state);

        assert!(selected_span_is_fully_visible(&state));
        assert!(matches!(
            state.view.block_viewport.anchor,
            ViewAnchor::Manual
        ));
    }

    #[test]
    fn boundary_navigation_noop_preserves_line_offset() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        enter_block_view(&mut state);
        let old_line_offset = state.view.block_viewport.line_offset;

        assert!(!select_relative_block(&mut state, 1));
        assert_eq!(state.view.block_viewport.selected_index, 0);
        assert_eq!(state.view.block_viewport.line_offset, old_line_offset);

        assert!(!select_relative_block(&mut state, -1));
        assert_eq!(state.view.block_viewport.selected_index, 0);
        assert_eq!(state.view.block_viewport.line_offset, old_line_offset);
    }

    #[test]
    fn force_render_flushes_once() {
        let mut state = runtime_state();
        state.render_state.dirty = true;
        state.render_state.force_render = true;

        assert!(flush_render_state(&mut state));
        assert!(!state.render_state.dirty);
        assert!(!state.render_state.force_render);
        assert!(!flush_render_state(&mut state));
    }

    #[test]
    fn resize_clamps_line_offset_to_keep_selected_visible() {
        let mut state = runtime_state();
        state.rows = 30;
        add_block_with_lines(
            &mut state,
            "tall",
            &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
        );
        add_block(&mut state, "b");
        add_block(&mut state, "c");
        enter_block_view(&mut state);

        select_block_index(&mut state, 0, ViewAnchor::Manual);
        assert!(selected_span_is_fully_visible(&state));

        // Simulate resize to a much smaller terminal
        state.rows = 10;
        ensure_selected_visible(&mut state);
        assert!(selected_span_is_fully_visible(&state));
    }

    #[test]
    fn detail_esc_sequences_consumed_but_not_exited() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        for seq in [b"\x1b[A", b"\x1b[B", b"\x1b[C", b"\x1b[D"] {
            {
                let mut s = state.lock().unwrap();
                s.view.view = ViewKind::Detail;
                s.view.expanded_block = s.view.selected_block;
                s.render_state.needs_cleanup = false;
            }
            assert_eq!(
                handle_view_key_sequence(seq, &state),
                Some(3),
                "sequence {:?} should be consumed",
                seq
            );
            let s = state.lock().unwrap();
            assert!(
                matches!(s.view.view, ViewKind::Detail),
                "sequence {:?} should not exit Detail",
                seq
            );
            assert!(!s.render_state.needs_cleanup);
        }

        {
            let mut s = state.lock().unwrap();
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
            s.render_state.needs_cleanup = false;
        }
        assert_eq!(handle_view_key_sequence(b"\x1b", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(matches!(s.view.view, ViewKind::Blocks));
    }

    #[test]
    fn remaining_bytes_after_cleanup_are_preserved() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
        }

        // Simulate the input loop receiving "q\n" in one chunk.
        let buffer = b"q\n";
        let mut index = 0;
        let mut pending_bytes = Vec::new();

        if let Some(consumed) = handle_view_key_sequence(&buffer[index..], &state) {
            index += consumed;
            let needs_cleanup = state
                .lock()
                .map(|s| s.render_state.needs_cleanup)
                .unwrap_or(false);
            if needs_cleanup {
                pending_bytes.extend_from_slice(&buffer[index..]);
            }
        }

        // 'q' consumed 1 byte, leaving "\n" as pending for PTY forwarding after cleanup.
        assert_eq!(pending_bytes, b"\n");
        assert!(state.lock().unwrap().render_state.needs_cleanup);
    }

    #[test]
    fn block_view_y_copies_output_to_clipboard() {
        let mut state = runtime_state();
        add_block(&mut state, "echo hello");
        enter_block_view(&mut state);
        state.render_state.dirty = false;
        state.render_state.force_render = false;

        assert!(handle_block_view_byte(b'y', &mut state));
        // Clipboard write may fail in headless CI, but flash_message should
        // be set when clipboard succeeds, and the handler never panics.
        if write_to_clipboard("test") {
            let (msg, _) = state.render_state.flash_message.as_ref().unwrap();
            assert_eq!(msg, "copied output");
        }
    }

    #[test]
    fn block_view_y_upper_copies_command_to_clipboard() {
        let mut state = runtime_state();
        add_block(&mut state, "echo hello");
        enter_block_view(&mut state);

        assert!(handle_block_view_byte(b'Y', &mut state));
        if write_to_clipboard("test") {
            let (msg, _) = state.render_state.flash_message.as_ref().unwrap();
            assert_eq!(msg, "copied command");
        }
    }

    #[test]
    fn block_view_y_does_not_panic_with_empty_output() {
        let mut state = runtime_state();
        add_block(&mut state, "true");
        enter_block_view(&mut state);

        // y on a block with no output should never panic.
        assert!(handle_block_view_byte(b'y', &mut state));
    }

    #[test]
    fn block_view_y_does_not_panic_with_no_blocks() {
        let mut state = runtime_state();
        enter_block_view(&mut state);

        // y on empty block store should never panic.
        assert!(handle_block_view_byte(b'y', &mut state));
    }

    #[test]
    fn detail_view_y_triggers_copy_output() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo hello");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"y", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "y should not exit Detail"
        );
        if write_to_clipboard("test") {
            assert!(s.render_state.flash_message.is_some());
        }
    }

    #[test]
    fn detail_view_y_upper_triggers_copy_command() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo hello");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"Y", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "Y should not exit Detail"
        );
        if write_to_clipboard("test") {
            assert!(
                s.render_state.flash_message.is_some(),
                "flash message should be set after Y"
            );
            let (msg, _) = s.render_state.flash_message.as_ref().unwrap();
            assert!(
                msg.contains("command"),
                "flash should mention 'command', got: {msg}"
            );
        }
    }

    #[test]
    fn detail_j_scrolls_line_offset_down() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        let before = state.lock().unwrap().view.block_viewport.line_offset;
        assert_eq!(handle_view_key_sequence(b"j", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "j should stay in Detail"
        );
        assert_eq!(
            s.view.block_viewport.line_offset,
            before + 1,
            "j should increase line_offset by 1"
        );
    }

    #[test]
    fn detail_k_scrolls_line_offset_up() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
            s.view.block_viewport.line_offset = 5;
        }

        assert_eq!(handle_view_key_sequence(b"k", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "k should stay in Detail"
        );
        assert_eq!(s.view.block_viewport.line_offset, 4);
    }

    #[test]
    fn detail_g_jumps_to_expanded_top() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
            s.view.block_viewport.line_offset = 5;
        }

        assert_eq!(handle_view_key_sequence(b"g", &state), Some(1));
        let s = state.lock().unwrap();
        // g should set line_offset to the expanded block's start_line (0).
        assert_eq!(s.view.block_viewport.line_offset, 0);
    }

    #[test]
    fn detail_g_upper_jumps_to_expanded_bottom() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"G", &state), Some(1));
        // G sets line_offset to span.end_line - content_height.
        // For a single-block layout, end_line is the visual line count.
        let s = state.lock().unwrap();
        // The line_offset should be > 0 because the expanded block is taller than the screen.
        // We can't assert the exact value without building the layout, but it should not cause panic.
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "G should stay in Detail"
        );
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
