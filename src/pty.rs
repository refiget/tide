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
    app::{BlockKind, InputAccumulator, RenderState, ViewAnchor, ViewKind, ViewState},
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
                    while index < n {
                        let byte = buffer[index];

                        if let Some(consumed) =
                            handle_view_key_sequence(&buffer[index..n], &input_state)
                        {
                            index += consumed;
                            continue;
                        }

                        if byte == 0x02 {
                            if let Ok(mut state) = input_state.lock() {
                                if matches!(state.view.view, ViewKind::Plain)
                                    && state.blocks.active_block_id().is_none()
                                {
                                    enter_block_view(&mut state);
                                    drop(state);
                                    let _ = render_runtime(&input_state, &input_stdout);
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
            if let Ok(mut state) = resize_state.lock() {
                state.rows = size.rows;
                state.cols = size.cols;
            }
            let should_render = resize_state
                .lock()
                .map(|state| !matches!(state.view.view, ViewKind::Plain))
                .unwrap_or(false);
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

fn render_runtime(
    state: &Arc<Mutex<RuntimeState>>,
    stdout: &Arc<Mutex<io::Stdout>>,
) -> io::Result<()> {
    let (visual_lines, view, cursor, layout, rows, cols) = {
        let state = state
            .lock()
            .map_err(|_| io::Error::other("runtime state lock poisoned"))?;
        let visual_lines = Compositor::build_visual_lines(
            &state.shell,
            &state.blocks,
            &state.view,
            state.cols,
            state.rows,
            &state.config.block_layout,
            &state.config.block_view,
        );
        (
            visual_lines,
            state.view.clone(),
            state.shell.cursor_position(),
            state.config.block_layout.clone(),
            state.rows,
            state.cols,
        )
    };

    let mut stdout = stdout
        .lock()
        .map_err(|_| io::Error::other("stdout lock poisoned"))?;
    renderer::render(
        &mut *stdout,
        &visual_lines,
        &view,
        Some(cursor),
        &layout,
        rows,
        cols,
    )
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

        let elapsed = state.render_state.last_render_at.elapsed();
        if elapsed < FRAME_DURATION {
            FRAME_DURATION - elapsed
        } else {
            Duration::ZERO
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
        flush_navigation_delta(&mut state);
        state.render_state.dirty = false;
        state.render_state.last_render_at = Instant::now();
    }

    render_runtime(state, stdout)
}

fn enter_block_view(state: &mut RuntimeState) {
    state.view.view = ViewKind::Blocks;
    state.view.expanded_block = None;
    select_tail_block(state);
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
            [b'q', ..] | [b'\x1b', ..] => {
                state.view.view = ViewKind::Blocks;
                state.view.expanded_block = None;
                state.render_state.dirty = true;
                Some(1)
            }
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
            state.render_state.dirty = true;
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
            true
        }
        b'g' => {
            state.input_accumulator.pending_block_delta = 0;
            select_block_index(state, 0, ViewAnchor::Top);
            state.render_state.dirty = true;
            true
        }
        b'\r' | b'\n' => {
            flush_navigation_delta(state);
            state.view.expanded_block = state.view.selected_block;
            state.view.view = ViewKind::Detail;
            state.render_state.dirty = true;
            true
        }
        _ => true,
    }
}

fn accumulate_block_delta(state: &mut RuntimeState, delta: isize) {
    state.input_accumulator.pending_block_delta = state
        .input_accumulator
        .pending_block_delta
        .saturating_add(delta);
    state.input_accumulator.last_input_at = Some(Instant::now());
    state.render_state.dirty = true;
}

fn flush_navigation_delta(state: &mut RuntimeState) {
    let delta = state.input_accumulator.pending_block_delta;
    if delta == 0 {
        return;
    }
    state.input_accumulator.pending_block_delta = 0;
    select_relative_block(state, delta);
}

fn select_relative_block(state: &mut RuntimeState, delta: isize) {
    if state.blocks.is_empty() {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        return;
    }

    let current = state
        .view
        .block_viewport
        .selected_index
        .min(state.blocks.len().saturating_sub(1));
    let next = if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        (current + delta as usize).min(state.blocks.len().saturating_sub(1))
    };
    let anchor = if !delta.is_negative() && next == state.blocks.len().saturating_sub(1) {
        ViewAnchor::Tail
    } else {
        ViewAnchor::Manual
    };
    select_block_index(state, next, anchor);
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
        state.view.block_viewport.anchor = anchor;
        return;
    }

    let index = index.min(state.blocks.len().saturating_sub(1));
    state.view.block_viewport.selected_index = index;
    state.view.selected_block = state.blocks.block_id_at(index);
    state.view.block_viewport.anchor = anchor;
    match anchor {
        ViewAnchor::Tail => {
            state.view.block_viewport.scroll_offset = compute_tail_scroll_offset(state);
        }
        ViewAnchor::Top => {
            state.view.block_viewport.scroll_offset = 0;
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
        state.view.block_viewport.anchor = ViewAnchor::Tail;
        return;
    }

    let last = state.blocks.len().saturating_sub(1);
    state.view.block_viewport.selected_index = last;
    state.view.selected_block = state.blocks.block_id_at(last);
    state.view.block_viewport.anchor = ViewAnchor::Tail;
    state.view.block_viewport.scroll_offset = compute_tail_scroll_offset(state);
}

fn compute_tail_scroll_offset(state: &RuntimeState) -> usize {
    let mut used_height = 0_usize;
    let mut offset = state.blocks.len();
    let terminal_height = state.rows as usize;

    while offset > 0 {
        let Some(block_id) = state.blocks.block_id_at(offset - 1) else {
            break;
        };
        let Some(block) = state.blocks.block(block_id) else {
            break;
        };
        let height = block_preview_height(block, &state.config.block_view);

        if used_height.saturating_add(height) > terminal_height {
            break;
        }

        used_height = used_height.saturating_add(height);
        offset -= 1;
    }

    offset
}

#[derive(Debug, Clone, Copy)]
struct VisibleRange {
    start: usize,
    end: usize,
}

fn compute_visible_range_from_offset(state: &RuntimeState) -> VisibleRange {
    let start = state
        .view
        .block_viewport
        .scroll_offset
        .min(state.blocks.len());
    let mut used_height = 0_usize;
    let mut end = start;
    let terminal_height = state.rows as usize;

    for index in start..state.blocks.len() {
        let Some(block_id) = state.blocks.block_id_at(index) else {
            break;
        };
        let Some(block) = state.blocks.block(block_id) else {
            break;
        };
        let expanded = state.view.expanded_block == Some(block_id)
            && matches!(state.view.view, ViewKind::Detail);
        let height = compute_block_height(block, &state.config.block_view, expanded);
        if end > start && used_height.saturating_add(height) > terminal_height {
            break;
        }
        used_height = used_height.saturating_add(height);
        end = index;
    }

    VisibleRange { start, end }
}

fn ensure_selected_visible(state: &mut RuntimeState) {
    if state.blocks.is_empty() {
        return;
    }

    let range = compute_visible_range_from_offset(state);
    let selected = state.view.block_viewport.selected_index;
    if selected < range.start {
        state.view.block_viewport.scroll_offset = selected;
    } else if selected > range.end {
        state.view.block_viewport.scroll_offset = compute_scroll_offset_ending_at(state, selected);
    } else {
        let margin = state.config.block_view.scroll_margin_blocks;
        if range.start > 0 && selected <= range.start.saturating_add(margin) {
            state.view.block_viewport.scroll_offset = selected.saturating_sub(margin);
        } else if range.end < state.blocks.len().saturating_sub(1)
            && selected.saturating_add(margin) >= range.end
        {
            let target = selected
                .saturating_add(margin)
                .min(state.blocks.len().saturating_sub(1));
            state.view.block_viewport.scroll_offset =
                compute_scroll_offset_ending_at(state, target);
        }
    }
    state.view.block_viewport.anchor = ViewAnchor::Manual;
}

fn compute_scroll_offset_ending_at(state: &RuntimeState, selected_index: usize) -> usize {
    let mut used_height = 0_usize;
    let mut offset = selected_index.saturating_add(1).min(state.blocks.len());
    let terminal_height = state.rows as usize;

    while offset > 0 {
        let index = offset - 1;
        let Some(block_id) = state.blocks.block_id_at(index) else {
            break;
        };
        let Some(block) = state.blocks.block(block_id) else {
            break;
        };
        let expanded = state.view.expanded_block == Some(block_id)
            && matches!(state.view.view, ViewKind::Detail);
        let height = compute_block_height(block, &state.config.block_view, expanded);

        if used_height.saturating_add(height) > terminal_height {
            break;
        }

        used_height = used_height.saturating_add(height);
        offset -= 1;
    }

    offset
}

fn compute_block_height(
    block: &crate::app::CommandBlock,
    config: &crate::config::BlockViewConfig,
    expanded: bool,
) -> usize {
    let body_lines = if block.kind == BlockKind::RawProgram {
        1
    } else {
        let output_lines = block.output_text.lines().count().max(1);
        let limit = if expanded {
            config.expanded_lines
        } else {
            config.preview_lines
        };
        let shown = output_lines.min(limit);
        let truncation = usize::from(output_lines > shown);
        shown + truncation
    };
    let detail_lines = if expanded { 10 } else { 0 };

    1 + body_lines + detail_lines + 1 + config.block_gap
}

fn block_preview_height(
    block: &crate::app::CommandBlock,
    config: &crate::config::BlockViewConfig,
) -> usize {
    let body_lines = if block.kind == BlockKind::RawProgram {
        1
    } else {
        let output_lines = block.output_text.lines().count();
        let output_lines = output_lines.max(1);
        let shown = output_lines.min(config.preview_lines);
        let truncation = usize::from(output_lines > shown);
        shown + truncation
    };

    1 + body_lines + 1 + config.block_gap
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
