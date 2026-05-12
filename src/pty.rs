use std::{
    collections::HashMap,
    io::{self, Read, Write},
    path::Path,
    process::Command,
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
    app::{
        BlockAction, BlockId, BlockKind, BlockStatus, BlockViewAction, ConfirmKind, ConfirmState,
        DEFAULT_TUI_COMMANDS, DetailViewAction, HelpState, InputAccumulator, RenderState,
        ReturnPanelTarget, TuiAppMatch, TuiAppMatchSource, TuiRuntimeState, ViewAnchor, ViewKind,
        ViewState, VisibleSource,
    },
    block::BlockStore,
    buffer::ShellBuffer,
    compositor::Compositor,
    config::{Config, RuntimeConfig, build_runtime_config},
    format::{CopyFormat, CopyPart, format_blocks},
    renderer::{self, BLOCK_HELP_ENTRIES, DETAIL_HELP_ENTRIES},
    shell_hooks::{Osc777Parser, ParsedPtyPart, ShellHookEvent},
};

struct RuntimeState {
    shell: ShellBuffer,
    blocks: BlockStore,
    view: ViewState,
    input_accumulator: InputAccumulator,
    render_state: RenderState,
    config: RuntimeConfig,
    rows: u16,
    cols: u16,
    index: crate::index::BlockIndex,
    /// TUI full-screen lifecycle state machine.
    tui_state: TuiRuntimeState,
    /// Whether the PTY (child process) is currently in an alternate screen buffer.
    pty_alt_screen_active: bool,
    /// Whether Tide's own UI (Block View) is currently in an alternate screen buffer.
    tide_alt_screen_active: bool,
    tide_id: String,
    opencode_by_block: HashMap<BlockId, String>,
    opencode_jump_stack: Vec<String>,
}

#[derive(Clone)]
enum TuiLifecycleEvent {
    PreexecMatch {
        app_match: TuiAppMatch,
        command: String,
    },
    PreexecNoMatch,
    AltScreenEnter {
        block_id: BlockId,
    },
    AltScreenExit,
    Precmd,
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
        rows,
        cols,
        index: crate::index::BlockIndex::new(),
        tui_state: TuiRuntimeState::Idle,
        pty_alt_screen_active: false,
        tide_alt_screen_active: false,
        tide_id: format!("tide-{}", std::process::id()),
        opencode_by_block: HashMap::new(),
        opencode_jump_stack: Vec::new(),
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
                                if let Ok(mut state) = output_state.lock() {
                                    let active_block_id = state.blocks.active_block_id();
                                    if !state.tui_state.is_capture_suspended() {
                                        state.blocks.append_output(&visible);
                                        state.shell.append(&visible, active_block_id);
                                    }
                                    let view = state.view.view.clone();
                                    drop(state);

                                    if matches!(view, ViewKind::Plain) {
                                        if let Ok(mut stdout) = output_stdout.lock() {
                                            if stdout.write_all(&visible).is_err() {
                                                break;
                                            }
                                            let _ = stdout.flush();
                                        }
                                    }
                                } else {
                                    break;
                                };
                            }
                            ParsedPtyPart::Event(event) => {
                                if let Ok(mut state) = output_state.lock() {
                                    match event {
                                        ShellHookEvent::AltScreenEnter => on_alt_screen_enter(&mut state),
                                        ShellHookEvent::AltScreenExit => on_alt_screen_exit(&mut state),
                                        _ => apply_shell_hook_event(&mut state, event, debug_blocks),
                                    }
                                }
                            }
                        }
                    }

                    let should_render = output_state
                        .lock()
                        .map(|state| {
                            !matches!(state.view.view, ViewKind::Plain | ViewKind::Help)
                                && state.view.confirm.is_none()
                        })
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
            let (view, has_overlay) = if let Ok(mut state) = output_state.lock() {
                let active_block_id = state.blocks.active_block_id();
                if !state.tui_state.is_capture_suspended() {
                    state.shell.append(&remaining, active_block_id);
                }
                let has_overlay =
                    matches!(state.view.view, ViewKind::Help) || state.view.confirm.is_some();
                (state.view.view.clone(), has_overlay)
            } else {
                (ViewKind::Plain, false)
            };

            if matches!(view, ViewKind::Plain) {
                if let Ok(mut stdout) = output_stdout.lock() {
                    let _ = stdout.write_all(&remaining);
                    let _ = stdout.flush();
                }
            } else if !has_overlay {
                // Skip re-render when a static overlay (Help/Confirm) is showing —
                // the overlay was drawn with force_render when opened; re-rendering
                // from PTY output would cause visible flicker without adding value.
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
                                if matches!(state.view.view, ViewKind::Plain) {
                                    // Enter alternate screen for Block View.
                                    // Lock ordering: drop state before locking stdout
                                    // (output thread locks state -> stdout, must not invert).
                                    drop(state);
                                    if let Ok(mut stdout) = input_stdout.lock() {
                                        let _ = renderer::enter_block_render(&mut *stdout);
                                    }
                                    let mut state =
                                        input_state.lock().unwrap_or_else(|e| e.into_inner());
                                    state.tide_alt_screen_active = true;
                                    enter_block_view(&mut state);
                                    drop(state);
                                    let _ = maybe_flush_navigation_and_render(
                                        &input_state,
                                        &input_stdout,
                                        false,
                                    );
                                }
                                // Always consume Ctrl-B — never forward to the PTY.
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

                    let (needs_cleanup, was_alt_screen) = if let Ok(state) = input_state.lock() {
                        let should_emit_leave = if state.render_state.force_pty_alt_screen_cleanup {
                            state.pty_alt_screen_active
                        } else if state.tide_alt_screen_active {
                            !state.pty_alt_screen_active
                        } else {
                            false
                        };
                        (state.render_state.needs_cleanup, should_emit_leave)
                    } else {
                        (false, false)
                    };

                    if needs_cleanup {
                        // Leave alt screen only if needed.
                        if let Ok(mut stdout) = input_stdout.lock() {
                            let _ = renderer::leave_block_render(&mut *stdout, was_alt_screen);
                        }

                        // Clear cleanup flags and extract pending_paste (state lock isolated,
                        // no stdout lock held).
                        let paste = if let Ok(mut state) = input_state.lock() {
                            // If it was a forced PTY cleanup, reset PTY flag.
                            if state.render_state.force_pty_alt_screen_cleanup {
                                state.pty_alt_screen_active = false;
                                state.render_state.force_pty_alt_screen_cleanup = false;
                            }
                            state.tide_alt_screen_active = false;
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
                    // Force Help to re-render its underlying view at the new size.
                    if let Some(h) = &mut state.view.help {
                        h.underlying_rendered = false;
                    }
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

/// Extract the actual command name from a shell command line.
///
/// Handles common prefixes: sudo, doas, env, command, noglob, builtin, time,
/// and `KEY=value` environment variable assignments.
fn extract_command_name(command_line: &str) -> Option<String> {
    let tokens: Vec<&str> = command_line.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        match token {
            "sudo" | "doas" => {
                i += 1;
                while i < tokens.len() {
                    let t = tokens[i];
                    if !t.starts_with('-') {
                        break;
                    }
                    i += 1;
                    if t == "-u" && i < tokens.len() {
                        i += 1;
                    }
                }
                continue;
            }
            "command" | "noglob" | "builtin" | "time" | "/usr/bin/time" => {
                i += 1;
                continue;
            }
            "env" => {
                i += 1;
                while i < tokens.len() {
                    let t = tokens[i];
                    if t.contains('=') && !t.starts_with('-') {
                        i += 1;
                        continue;
                    }
                    if t.starts_with('-') {
                        i += 1;
                        if t == "-u" && i < tokens.len() {
                            i += 1;
                        }
                        continue;
                    }
                    break;
                }
                continue;
            }
            _ if token.contains('=') && !token.starts_with('/') => {
                // Handles: FOO=bar nvim
                i += 1;
                continue;
            }
            _ => {
                return Some(basename_command(token));
            }
        }
    }
    None
}

fn basename_command(command: &str) -> String {
    command.rsplit('/').next().unwrap_or(command).to_string()
}

/// Match a command name against known TUI apps.
///
/// Priority:
/// 1. User-configured app in `tui_apps` (exact command match)
/// 2. User `tui_extra_commands` list
/// 3. Builtin default list (`DEFAULT_TUI_COMMANDS`)
fn detect_tui_app(
    command_line: &str,
    extra_commands: &[String],
    tui_apps: &std::collections::BTreeMap<String, crate::config::TuiAppConfig>,
) -> Option<TuiAppMatch> {
    let command_name = extract_command_name(command_line)?;

    // 1. User app config exact match
    for (app_name, app_cfg) in tui_apps {
        if app_cfg.commands.iter().any(|c| c == &command_name) {
            return Some(TuiAppMatch {
                app_name: app_name.clone(),
                command_name: command_name.clone(),
                source: TuiAppMatchSource::UserConfig,
            });
        }
    }

    // 2. User extra_commands list
    if extra_commands.iter().any(|c| c == &command_name) {
        return Some(TuiAppMatch {
            app_name: command_name.clone(),
            command_name,
            source: TuiAppMatchSource::UserConfig,
        });
    }

    // 3. Builtin default list
    if DEFAULT_TUI_COMMANDS.contains(&command_name.as_str()) {
        return Some(TuiAppMatch {
            app_name: command_name.clone(),
            command_name,
            source: TuiAppMatchSource::Builtin,
        });
    }

    None
}

fn is_opencode_command(command_line: &str) -> bool {
    matches!(extract_command_name(command_line).as_deref(), Some("opencode"))
}

fn is_opencode_process_command(command: &str) -> bool {
    let Some(exe) = command.split_whitespace().next() else {
        return false;
    };
    let base = exe.rsplit('/').next().unwrap_or(exe).to_ascii_lowercase();
    base == "opencode" || base.starts_with("opencode-")
}

fn tmux_current_tty() -> Option<String> {
    let out = Command::new("tmux")
        .args(["display-message", "-p", "#{pane_tty}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn tty_has_opencode_process(tty_path: &str) -> bool {
    let tty = tty_path.strip_prefix("/dev/").unwrap_or(tty_path);
    let out = Command::new("ps")
        .args(["-axo", "tty=,command="])
        .output();
    let Ok(out) = out else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().any(|line| {
        let mut parts = line.split_whitespace();
        let Some(line_tty) = parts.next() else {
            return false;
        };
        if line_tty != tty {
            return false;
        }
        let command = line.trim_start_matches(line_tty).trim_start();
        is_opencode_process_command(command)
    })
}

fn register_running_opencode_block(state: &mut RuntimeState, id: BlockId, command: &str) {
    if state.opencode_by_block.contains_key(&id) {
        return;
    }
    let Some(block) = state.blocks.block(id) else {
        return;
    };
    let Some(target) = tmux_current_target() else {
        return;
    };
    let project = block
        .cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "opencode".to_string());
    if let Ok(alias) = crate::opencode_registry::register_running(
        &state.tide_id,
        id.0,
        command,
        &block.cwd.display().to_string(),
        &project,
        &target,
    ) {
        state.opencode_by_block.insert(id, alias);
    }
}

fn detect_and_register_opencode(state: &mut RuntimeState, id: BlockId, command: &str) {
    if let Some(tty) = tmux_current_tty() {
        for _ in 0..8 {
            if tty_has_opencode_process(&tty) {
                register_running_opencode_block(state, id, command);
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    // Fallback: direct command match when process probing is unavailable.
    if is_opencode_command(command) {
        register_running_opencode_block(state, id, command);
    }
}

fn is_running_opencode_block(state: &RuntimeState, id: BlockId) -> bool {
    if state.opencode_by_block.contains_key(&id) {
        return true;
    }
    state
        .blocks
        .block(id)
        .map(|b| {
            b.status == BlockStatus::Running
                && b
                    .app_name
                    .as_deref()
                    .map(|s| s.starts_with("opencode_alias:"))
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn is_shared_opencode_block(block: &crate::app::CommandBlock) -> Option<String> {
    block
        .app_name
        .as_deref()
        .and_then(|a| a.strip_prefix("opencode_alias:"))
        .map(|s| s.to_string())
}

fn synthetic_opencode_block_id(alias: &str) -> BlockId {
    let mut h: u64 = 1469598103934665603;
    for b in alias.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(1099511628211);
    }
    BlockId(u64::MAX - (h % 1_000_000_000))
}

fn tmux_current_target() -> Option<String> {
    let out = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}:#{window_index}.#{pane_index}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn tmux_target_exists(target: &str) -> bool {
    Command::new("tmux")
        .args(["display-message", "-p", "-t", target, "#{pane_id}"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tmux_jump_and_zoom(target: &str) -> bool {
    let Some((session, rest)) = target.split_once(':') else {
        return false;
    };
    let Some((window, pane)) = rest.split_once('.') else {
        return false;
    };

    if Command::new("tmux")
        .args(["switch-client", "-t", session])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && Command::new("tmux")
            .args(["select-window", "-t", &format!("{session}:{window}")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        && Command::new("tmux")
            .args(["select-pane", "-t", &format!("{session}:{window}.{pane}")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    {
        let zoomed = Command::new("tmux")
            .args(["display-message", "-p", "-t", target, "#{window_zoomed_flag}"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if zoomed == "0" {
            let _ = Command::new("tmux").args(["resize-pane", "-Z"]).status();
        }
        true
    } else {
        false
    }
}

fn sync_shared_opencode_blocks(state: &mut RuntimeState) {
    let Ok(records) = crate::opencode_registry::list_running() else {
        return;
    };

    let existing_ids: std::collections::HashSet<BlockId> = state
        .blocks
        .timeline
        .iter()
        .copied()
        .filter(|id| {
            state
                .blocks
                .block(*id)
                .and_then(is_shared_opencode_block)
                .is_some()
        })
        .collect();

    for id in existing_ids {
        state.blocks.remove(id);
    }

    for rec in records {
        let id = synthetic_opencode_block_id(&rec.alias);
        let cwd = std::path::PathBuf::from(&rec.cwd);
        let project = if rec.project_name.is_empty() {
            Path::new(&rec.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "opencode".to_string())
        } else {
            rec.project_name.clone()
        };

        state.blocks.executions.insert(
            id,
            crate::app::CommandBlock {
                id,
                command: format!("[{}] opencode {}", rec.alias, project),
                cwd,
                status: BlockStatus::Running,
                kind: BlockKind::SystemEvent,
                app_name: Some(format!("opencode_alias:{}", rec.alias)),
                ..crate::app::CommandBlock::default()
            },
        );
        state.blocks.timeline.push(id);
    }
}

fn move_running_opencode_to_bottom(state: &mut RuntimeState) {
    if state.blocks.timeline.len() < 2 {
        return;
    }

    let mut normal = Vec::with_capacity(state.blocks.timeline.len());
    let mut running_opencode = Vec::new();

    for &id in &state.blocks.timeline {
        if is_running_opencode_block(state, id) {
            running_opencode.push(id);
        } else {
            normal.push(id);
        }
    }

    if running_opencode.is_empty() {
        return;
    }

    normal.extend(running_opencode);
    state.blocks.timeline = normal;

    let reordered_filtered = match &state.view.visible {
        VisibleSource::Filtered(ids) => {
            let running: std::collections::HashSet<BlockId> = ids
                .iter()
                .copied()
                .filter(|id| is_running_opencode_block(state, *id))
                .collect();
            if running.is_empty() {
                None
            } else {
                let mut non_running: Vec<BlockId> =
                    ids.iter().copied().filter(|id| !running.contains(id)).collect();
                let mut running_ids: Vec<BlockId> =
                    ids.iter().copied().filter(|id| running.contains(id)).collect();
                non_running.append(&mut running_ids);
                Some(non_running)
            }
        }
        VisibleSource::AllTimeline => None,
    };

    if let (Some(reordered), VisibleSource::Filtered(ids)) =
        (reordered_filtered, &mut state.view.visible)
    {
        *ids = reordered;
    }
}

fn advance_tui_state(
    prev: TuiRuntimeState,
    event: TuiLifecycleEvent,
) -> (TuiRuntimeState, Option<BlockId>) {
    match event {
        TuiLifecycleEvent::PreexecMatch { app_match, command } => {
            (TuiRuntimeState::Pending { app_match, command }, None)
        }
        TuiLifecycleEvent::PreexecNoMatch => (TuiRuntimeState::Idle, None),
        TuiLifecycleEvent::AltScreenEnter { block_id } => {
            (TuiRuntimeState::InAltScreen { block_id }, None)
        }
        TuiLifecycleEvent::AltScreenExit => match prev {
            // Preserve non-InAltScreen states to avoid clobbering Pending on unrelated exits.
            TuiRuntimeState::InAltScreen { block_id } => {
                (TuiRuntimeState::ExitedAltScreen { block_id }, None)
            }
            other => (other, None),
        },
        TuiLifecycleEvent::Precmd => match prev {
            TuiRuntimeState::ExitedAltScreen { block_id } | TuiRuntimeState::InAltScreen { block_id } => {
                (TuiRuntimeState::Idle, Some(block_id))
            }
            TuiRuntimeState::Pending { .. } | TuiRuntimeState::Idle => (TuiRuntimeState::Idle, None),
        },
    }
}

fn apply_shell_hook_event(state: &mut RuntimeState, event: ShellHookEvent, debug_blocks: bool) {
    match event {
        ShellHookEvent::AltScreenEnter => on_alt_screen_enter(state),
        ShellHookEvent::AltScreenExit => on_alt_screen_exit(state),
        ShellHookEvent::Preexec { command } => {
            let start_line = state.shell.line_count();
            let block_id =
                state
                    .blocks
                    .start_command(command.clone(), start_line, BlockKind::NormalCommand);
            state.index.index_command(block_id, &command);

            // Detect known TUI apps for later handoff / return panel.
            let lifecycle_event = if let Some(app_match) = detect_tui_app(
                &command,
                &state.config.tui_extra_commands,
                &state.config.tui_apps,
            ) {
                TuiLifecycleEvent::PreexecMatch {
                    app_match,
                    command: command.clone(),
                }
            } else {
                TuiLifecycleEvent::PreexecNoMatch
            };
            let prev = std::mem::take(&mut state.tui_state);
            let (next, _) = advance_tui_state(prev, lifecycle_event);
            state.tui_state = next;

            if let Some(id) = state.blocks.active_block_id() {
                detect_and_register_opencode(state, id, &command);
            }

            // Pin running opencode blocks to the bottom in Block View.
            sync_shared_opencode_blocks(state);
            move_running_opencode_to_bottom(state);
            sync_block_viewport_after_history_change(state);
        }
        ShellHookEvent::Precmd { exit_code, cwd } => {
            let finished_cwd = cwd;
            if let Some(ref cwd) = finished_cwd {
                state.blocks.set_cwd(cwd.clone());
            }

            // TUI session finalization — decide Return Panel or direct Plain.
            let prev = std::mem::take(&mut state.tui_state);
            let (next, tui_finalize_block) = advance_tui_state(prev, TuiLifecycleEvent::Precmd);
            state.tui_state = next;

            let active_id = state.blocks.active_block_id();
            let end_line = state.shell.line_count().saturating_sub(1);
            state.blocks.finish_command(exit_code, end_line);
            sync_block_viewport_after_history_change(state);

            if let Some(block_id) = tui_finalize_block {
                finalize_exited_tui_on_precmd(state, block_id);
            }

            if let Some(block_id) = active_id {
                if let Some(block) = state.blocks.block(block_id) {
                    if block.status == BlockStatus::Failed {
                        state.index.on_block_failed(block_id);
                        if state.view.filter.is_active() {
                            rebuild_visible(state);
                            restore_or_clamp_selection(state);
                        }
                    }
                }
            }
            if let Some(id) = active_id {
                if state.opencode_by_block.remove(&id).is_some() {
                    let _ = crate::opencode_registry::unregister_running(&state.tide_id, id.0);
                }
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
            sync_shared_opencode_blocks(state);
            move_running_opencode_to_bottom(state);
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

    let part = match action {
        BlockAction::CopyOutput => CopyPart::Output,
        BlockAction::CopyCommand => CopyPart::Command,
        BlockAction::CopyBlock => CopyPart::Both,
        _ => return,
    };
    let fmt = state.config.block_view.copy_format;
    let text = format_blocks(&[block], part, fmt);

    if write_to_clipboard(&text) {
        let msg = copy_flash(1, part, fmt);
        state.render_state.flash_message = Some((msg, Instant::now()));
        state.render_state.dirty = true;
        state.render_state.force_render = true;
    }
}

/// Returns all BlockIds in the current visual range (anchor → cursor), in timeline order.
/// When no visual mode is active, returns the single selected block (if any).
fn visual_range_ids(state: &RuntimeState) -> Vec<BlockId> {
    let ids = state.view.visible.ids(&state.blocks);
    let cur_idx = state
        .view
        .block_viewport
        .selected_index
        .min(ids.len().saturating_sub(1));
    match state.view.visual_anchor {
        None => ids.get(cur_idx).copied().into_iter().collect(),
        Some(anchor) => {
            let anchor_idx = ids.iter().position(|&id| id == anchor).unwrap_or(cur_idx);
            let lo = anchor_idx.min(cur_idx);
            let hi = anchor_idx.max(cur_idx).min(ids.len().saturating_sub(1));
            ids[lo..=hi].to_vec()
        }
    }
}

fn exit_visual_mode(state: &mut RuntimeState) {
    state.view.visual_anchor = None;
}

fn copy_blocks(state: &mut RuntimeState, part: CopyPart) {
    let ids = visual_range_ids(state);
    if ids.is_empty() {
        return;
    }
    let fmt = state.config.block_view.copy_format;
    let blocks: Vec<&_> = ids
        .iter()
        .filter_map(|&id| state.blocks.block(id))
        .collect();
    let text = format_blocks(&blocks, part, fmt);
    if write_to_clipboard(&text) {
        let msg = copy_flash(blocks.len(), part, fmt);
        state.render_state.flash_message = Some((msg, Instant::now()));
        state.render_state.dirty = true;
        state.render_state.force_render = true;
    }
    exit_visual_mode(state);
}

fn copy_flash(count: usize, part: CopyPart, fmt: CopyFormat) -> String {
    let what = match part {
        CopyPart::Command => {
            if count == 1 {
                "command"
            } else {
                "commands"
            }
        }
        CopyPart::Output => {
            if count == 1 {
                "output"
            } else {
                "outputs"
            }
        }
        CopyPart::Both => {
            if count == 1 {
                "block"
            } else {
                "blocks"
            }
        }
    };
    let prefix = if count == 1 {
        format!("copied {what}")
    } else {
        format!("copied {count} {what}")
    };
    if fmt == CopyFormat::Plaintext {
        prefix
    } else {
        format!("{prefix} · {}", fmt.name())
    }
}

/// For Detail View: copy output respecting visual line selection.
fn detail_copy_output(state: &RuntimeState) -> Option<String> {
    let id = state.view.expanded_block?;
    let block = state.blocks.block(id)?;
    if let Some(anchor) = state.view.detail_visual_anchor {
        let plain_lines: Vec<String> = crate::ansi::parse_ansi_lines(&block.output_raw)
            .into_iter()
            .map(|s| crate::ansi::styled_to_plain(&s))
            .collect();
        let total = plain_lines.len();
        if total == 0 {
            return Some(String::new());
        }
        let cursor = state.view.detail_line_cursor.min(total.saturating_sub(1));
        let lo = anchor.min(cursor);
        let hi = anchor.max(cursor).min(total.saturating_sub(1));
        Some(plain_lines[lo..=hi].join("\n"))
    } else {
        Some(block.output_text.clone())
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
            std::env::var("HOME")
                .ok()
                .map(std::path::PathBuf::from)
                .as_deref(),
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
    let (rendered, drew_underlying) = renderer::render(
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
        // Mark underlying view as cleanly rendered so subsequent Help
        // navigations can skip it (avoiding full-screen flicker on j/k).
        if drew_underlying {
            if let Some(h) = &mut state.view.help {
                h.underlying_rendered = true;
            }
        }
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
    sync_shared_opencode_blocks(state);
    move_running_opencode_to_bottom(state);
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
        ViewKind::ReturnPanel => {
            // Stub: will be wired in Step 5. Consume all input for now.
            Some(bytes.len().min(3))
        }
        ViewKind::Agent => Some(1),
        ViewKind::Blocks => match bytes {
            // While a confirm dialog is open, all input goes through its handler.
            _ if state.view.confirm.is_some() => {
                let byte = bytes[0];
                handle_block_view_byte(byte, &mut state);
                Some(bytes.len().min(3))
            }
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
            // Multi-byte escape sequences — hardcoded (not remappable)
            [b'\x1b', b'[', b'B', ..] => {
                execute_detail_view_action(DetailViewAction::NavDown, &mut state);
                Some(3)
            }
            [b'\x1b', b'[', b'A', ..] => {
                execute_detail_view_action(DetailViewAction::NavUp, &mut state);
                Some(3)
            }
            // Catch other escape sequences (3+ bytes)
            [b'\x1b', ..] if bytes.len() >= 3 => Some(bytes.len().min(3)),
            // Single byte: dispatch through resolved keymap
            [byte, ..] => {
                if let Some(action) = state.config.resolved_detail_keymap.get(byte) {
                    execute_detail_view_action(*action, &mut state);
                }
                Some(1)
            }
            [] => None,
        },
        ViewKind::Help => {
            let return_view = state
                .view
                .help
                .as_ref()
                .map(|h| h.return_view.clone())
                .unwrap_or(ViewKind::Blocks);
            let entries = match &return_view {
                ViewKind::Detail => DETAIL_HELP_ENTRIES,
                _ => BLOCK_HELP_ENTRIES,
            };
            let n = entries.len();

            let ensure_scroll = |cursor: usize, scroll: usize, visible: usize| -> usize {
                if cursor < scroll {
                    cursor
                } else if cursor >= scroll + visible && visible > 0 {
                    cursor + 1 - visible
                } else {
                    scroll
                }
            };

            let visible = n.min((state.rows as usize).saturating_sub(5));

            let close_help = |state: &mut RuntimeState| {
                let rv = state
                    .view
                    .help
                    .take()
                    .map(|h| h.return_view)
                    .unwrap_or(ViewKind::Blocks);
                state.view.view = rv;
                state.render_state.dirty = true;
                state.render_state.force_render = true;
            };

            match bytes {
                [b'j', ..] | [b'\x1b', b'[', b'B', ..] => {
                    if let Some(h) = &mut state.view.help {
                        if h.cursor + 1 < n {
                            h.cursor += 1;
                            h.scroll = ensure_scroll(h.cursor, h.scroll, visible);
                            state.render_state.dirty = true;
                            state.render_state.force_render = true;
                        }
                    }
                    Some(if bytes[0] == b'\x1b' { 3 } else { 1 })
                }
                [b'k', ..] | [b'\x1b', b'[', b'A', ..] => {
                    if let Some(h) = &mut state.view.help {
                        if h.cursor > 0 {
                            h.cursor -= 1;
                            h.scroll = ensure_scroll(h.cursor, h.scroll, visible);
                            state.render_state.dirty = true;
                            state.render_state.force_render = true;
                        }
                    }
                    Some(if bytes[0] == b'\x1b' { 3 } else { 1 })
                }
                [b'g', ..] => {
                    if let Some(h) = &mut state.view.help {
                        h.cursor = 0;
                        h.scroll = 0;
                    }
                    state.render_state.dirty = true;
                    state.render_state.force_render = true;
                    Some(1)
                }
                [b'G', ..] => {
                    if let Some(h) = &mut state.view.help {
                        h.cursor = n.saturating_sub(1);
                        h.scroll = ensure_scroll(h.cursor, h.scroll, visible);
                    }
                    state.render_state.dirty = true;
                    state.render_state.force_render = true;
                    Some(1)
                }
                [b'q', ..] | [b'?', ..] | [b'\x1b'] => {
                    close_help(&mut *state);
                    Some(1)
                }
                [b'\x1b', ..] => Some(bytes.len().min(3)),
                [_, ..] => {
                    close_help(&mut *state);
                    Some(1)
                }
                [] => None,
            }
        }
    }
}

fn rebuild_visible(state: &mut RuntimeState) {
    let has_failed = state.view.filter.failed_only;
    let has_query = !state.view.filter.command_query.is_empty();

    match (has_failed, has_query) {
        (false, false) => {
            state.view.visible = crate::app::VisibleSource::AllTimeline;
        }
        (true, false) => {
            let ids = state.index.query_failed(&state.blocks.executions);
            state.view.visible = crate::app::VisibleSource::Filtered(ids);
        }
        (false, true) => {
            let ids = state
                .index
                .query_command(&state.view.filter.command_query, &state.blocks.executions);
            state.view.visible = crate::app::VisibleSource::Filtered(ids);
        }
        (true, true) => {
            let failed = state.index.query_failed(&state.blocks.executions);
            let cmd = state
                .index
                .query_command(&state.view.filter.command_query, &state.blocks.executions);
            let cmd_set: std::collections::HashSet<_> = cmd.into_iter().collect();
            let ids: Vec<_> = failed
                .into_iter()
                .filter(|id| cmd_set.contains(id))
                .collect();
            state.view.visible = crate::app::VisibleSource::Filtered(ids);
        }
    }

    // Keep running opencode blocks at the end even in filtered lists.
    sync_shared_opencode_blocks(state);
    move_running_opencode_to_bottom(state);
}

fn restore_or_clamp_selection(state: &mut RuntimeState) {
    let len = state.view.visible.len(&state.blocks);
    if len == 0 {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        return;
    }
    if let Some(prev) = state.view.selected_block {
        if let Some(idx) = state.view.visible.index_of(&state.blocks, prev) {
            state.view.block_viewport.selected_index = idx;
            return;
        }
    }
    let tail_idx = len - 1;
    state.view.block_viewport.selected_index = tail_idx;
    state.view.selected_block = state.view.visible.block_at(&state.blocks, tail_idx);
}

fn handle_search_input(byte: u8, state: &mut RuntimeState) -> bool {
    match byte {
        b'\r' | b'\n' => {
            let query = state.view.search_buffer.take().unwrap_or_default();
            state.view.filter.command_query = query;
            rebuild_visible(state);
            if state.view.visible.len(&state.blocks) == 0
                && !state.view.filter.command_query.is_empty()
            {
                state.view.filter.command_query = String::new();
                rebuild_visible(state);
                state.render_state.flash_message =
                    Some(("no matches".to_string(), std::time::Instant::now()));
            }
            restore_or_clamp_selection(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        b'\x1b' => {
            state.view.filter.command_query = state.view.pre_search_query.clone();
            rebuild_visible(state);
            restore_or_clamp_selection(state);
            state.view.search_buffer = None;
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        b'\x7f' | b'\x08' => {
            if let Some(buf) = &mut state.view.search_buffer {
                buf.pop();
            }
            state.view.filter.command_query = state
                .view
                .search_buffer
                .as_deref()
                .unwrap_or("")
                .to_string();
            rebuild_visible(state);
            restore_or_clamp_selection(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        0x20..=0x7e => {
            if let Some(buf) = &mut state.view.search_buffer {
                buf.push(byte as char);
            }
            state.view.filter.command_query = state
                .view
                .search_buffer
                .as_deref()
                .unwrap_or("")
                .to_string();
            rebuild_visible(state);
            restore_or_clamp_selection(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        _ => {}
    }
    true
}

fn execute_delete_blocks(state: &mut RuntimeState, block_ids: Vec<BlockId>) {
    if block_ids.is_empty() {
        return;
    }

    // Determine a neighbor outside the deleted range to land on after deletion.
    // Use the block after the last deleted id, or before the first.
    let last_id = *block_ids.last().unwrap();
    let first_id = block_ids[0];
    let next_sel = state
        .blocks
        .next_block(last_id)
        .filter(|id| !block_ids.contains(id))
        .or_else(|| {
            state
                .blocks
                .prev_block(first_id)
                .filter(|id| !block_ids.contains(id))
        });

    let id_set: std::collections::HashSet<BlockId> = block_ids.iter().copied().collect();

    for &id in &block_ids {
        state.blocks.remove(id);
        // BlockIndex queries filter by executions.contains_key(), so no explicit index cleanup needed.
    }

    // Sync VisibleSource if a filter is active.
    if let VisibleSource::Filtered(ref mut ids) = state.view.visible {
        ids.retain(|id| !id_set.contains(id));
    }

    // Clear expanded state if it pointed at a deleted block.
    if state
        .view
        .expanded_block
        .map_or(false, |id| id_set.contains(&id))
    {
        state.view.expanded_block = None;
    }

    state.view.selected_block = next_sel;
    exit_visual_mode(state);
    restore_or_clamp_selection(state);

    state.render_state.dirty = true;
    state.render_state.force_render = true;
}

fn execute_block_view_action(action: BlockViewAction, state: &mut RuntimeState) -> bool {
    match action {
        BlockViewAction::Quit => {
            if state.view.visual_anchor.is_some() {
                exit_visual_mode(state);
                state.render_state.dirty = true;
                state.render_state.force_render = true;
            } else {
                state.view = ViewState::default();
                state.input_accumulator.pending_block_delta = 0;
                state.render_state.needs_cleanup = true;
            }
            true
        }
        BlockViewAction::NavDown => {
            accumulate_block_delta(state, 1);
            true
        }
        BlockViewAction::NavUp => {
            accumulate_block_delta(state, -1);
            true
        }
        BlockViewAction::NavBottom => {
            state.input_accumulator.pending_block_delta = 0;
            select_tail_block(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::NavTop => {
            state.input_accumulator.pending_block_delta = 0;
            select_block_index(state, 0, ViewAnchor::Top);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::ScrollHalfUp => {
            let half = (state.rows / 4).max(1) as isize;
            accumulate_block_delta(state, -half);
            true
        }
        BlockViewAction::ScrollHalfDown => {
            let half = (state.rows / 4).max(1) as isize;
            accumulate_block_delta(state, half);
            true
        }
        BlockViewAction::ScrollFullUp => {
            let full = (state.rows / 2).max(1) as isize;
            accumulate_block_delta(state, -full);
            true
        }
        BlockViewAction::ScrollFullDown => {
            let full = (state.rows / 2).max(1) as isize;
            accumulate_block_delta(state, full);
            true
        }
        BlockViewAction::SearchNext => {
            if state.view.filter.is_active() {
                let len = state.view.visible.len(&state.blocks);
                if len > 0 {
                    let cur = state.view.block_viewport.selected_index.min(len - 1);
                    let next = (cur + 1) % len;
                    select_block_index(state, next, ViewAnchor::Manual);
                    state.render_state.dirty = true;
                    state.render_state.force_render = true;
                }
            }
            true
        }
        BlockViewAction::SearchPrev => {
            if state.view.filter.is_active() {
                let len = state.view.visible.len(&state.blocks);
                if len > 0 {
                    let cur = state.view.block_viewport.selected_index.min(len - 1);
                    let prev = if cur == 0 { len - 1 } else { cur - 1 };
                    select_block_index(state, prev, ViewAnchor::Manual);
                    state.render_state.dirty = true;
                    state.render_state.force_render = true;
                }
            }
            true
        }
        BlockViewAction::VisualMode => {
            if state.view.visual_anchor.is_some() {
                exit_visual_mode(state);
            } else {
                state.view.visual_anchor = state.view.selected_block;
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::Expand => {
            flush_navigation_delta(state);
            let selected = state.view.selected_block;
            if state.view.expanded_block == selected && selected.is_some() {
                state.view.expanded_block = None;
            } else {
                state.view.expanded_block = selected;
            }
            ensure_selected_visible(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::CopyCommand => {
            copy_blocks(state, CopyPart::Command);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::CopyOutput => {
            copy_blocks(state, CopyPart::Output);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::CopyBoth => {
            copy_blocks(state, CopyPart::Both);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::Rerun => {
            let ids = visual_range_ids(state);
            if ids.len() > 1 {
                state.view.confirm = Some(ConfirmState::multi(ConfirmKind::RerunBlocks, ids));
                state.render_state.dirty = true;
                state.render_state.force_render = true;
            } else {
                let command = ids
                    .first()
                    .and_then(|&id| state.blocks.block(id))
                    .map(|b| b.command.clone())
                    .filter(|cmd| !cmd.is_empty());
                if let Some(cmd) = command {
                    exit_visual_mode(state);
                    state.view = ViewState::default();
                    state.input_accumulator.pending_block_delta = 0;
                    state.render_state.needs_cleanup = true;
                    state.render_state.pending_paste = Some(cmd);
                }
            }
            true
        }
        BlockViewAction::DetailView => {
            if let Some(selected) = state.view.selected_block {
                if let Some(block) = state.blocks.block(selected)
                    && let Some(alias) = is_shared_opencode_block(block)
                    && let Ok(Some(rec)) = crate::opencode_registry::find_by_alias(&alias)
                {
                    if tmux_target_exists(&rec.tmux_target) {
                        if let Some(cur) = tmux_current_target() {
                            state.opencode_jump_stack.push(cur);
                        }
                        if tmux_jump_and_zoom(&rec.tmux_target) {
                            state.render_state.flash_message =
                                Some((format!("jumped [{}]", alias), Instant::now()));
                        } else {
                            state.render_state.flash_message =
                                Some((format!("jump failed [{}]", alias), Instant::now()));
                        }
                    } else {
                        state.render_state.flash_message =
                            Some((format!("stale [{}]", alias), Instant::now()));
                    }
                    state.render_state.dirty = true;
                    state.render_state.force_render = true;
                    return true;
                }

                exit_visual_mode(state);
                state.view.view = ViewKind::Detail;
                state.view.expanded_block = Some(selected);
                state.view.block_viewport.line_offset = 0;
                state.view.detail_line_cursor = 0;
                state.render_state.dirty = true;
                state.render_state.force_render = true;
            }
            true
        }
        BlockViewAction::ToggleFailedFilter => {
            state.view.filter.failed_only = !state.view.filter.failed_only;
            rebuild_visible(state);
            restore_or_clamp_selection(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::OpenSearch => {
            state.view.pre_search_query = state.view.filter.command_query.clone();
            state.view.search_buffer = Some(String::new());
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
        BlockViewAction::Delete => {
            let ids = visual_range_ids(state);
            if !ids.is_empty() {
                let kind = if ids.len() == 1 {
                    ConfirmKind::DeleteBlock
                } else {
                    ConfirmKind::DeleteBlocks
                };
                state.view.confirm = Some(ConfirmState::multi(kind, ids));
                state.render_state.dirty = true;
                state.render_state.force_render = true;
            }
            true
        }
        BlockViewAction::Help => {
            state.view.help = Some(HelpState::open(ViewKind::Blocks));
            state.view.view = ViewKind::Help;
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            true
        }
    }
}

fn execute_detail_view_action(action: DetailViewAction, state: &mut RuntimeState) {
    match action {
        DetailViewAction::NavDown => {
            let total = detail_output_line_count(state);
            if total > 0 && state.view.detail_line_cursor + 1 < total {
                state.view.detail_line_cursor += 1;
                let inner = detail_inner_height(state);
                let lo = state.view.block_viewport.line_offset;
                if state.view.detail_line_cursor >= lo + inner {
                    state.view.block_viewport.line_offset = state
                        .view
                        .detail_line_cursor
                        .saturating_sub(inner.saturating_sub(1));
                }
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::NavUp => {
            if state.view.detail_line_cursor > 0 {
                state.view.detail_line_cursor -= 1;
                if state.view.detail_line_cursor < state.view.block_viewport.line_offset {
                    state.view.block_viewport.line_offset = state.view.detail_line_cursor;
                }
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::NavTop => {
            state.view.detail_line_cursor = 0;
            state.view.block_viewport.line_offset = 0;
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::NavBottom => {
            let total = detail_output_line_count(state);
            let inner = detail_inner_height(state);
            if total > 0 {
                state.view.detail_line_cursor = total.saturating_sub(1);
                state.view.block_viewport.line_offset = total.saturating_sub(inner);
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::ScrollHalfDown => {
            let half = (state.rows / 4).max(1) as usize;
            let total = detail_output_line_count(state);
            if total > 0 {
                let target = state
                    .view
                    .detail_line_cursor
                    .saturating_add(half)
                    .min(total.saturating_sub(1));
                state.view.detail_line_cursor = target;
                let inner = detail_inner_height(state);
                let lo = state.view.block_viewport.line_offset;
                if state.view.detail_line_cursor >= lo + inner {
                    state.view.block_viewport.line_offset =
                        target.saturating_sub(inner.saturating_sub(1));
                }
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::ScrollHalfUp => {
            let half = (state.rows / 4).max(1) as usize;
            if state.view.detail_line_cursor > 0 {
                state.view.detail_line_cursor = state.view.detail_line_cursor.saturating_sub(half);
                if state.view.detail_line_cursor < state.view.block_viewport.line_offset {
                    state.view.block_viewport.line_offset = state.view.detail_line_cursor;
                }
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::ScrollFullDown => {
            let full = (state.rows / 2).max(1) as usize;
            let total = detail_output_line_count(state);
            if total > 0 {
                let target = state
                    .view
                    .detail_line_cursor
                    .saturating_add(full)
                    .min(total.saturating_sub(1));
                state.view.detail_line_cursor = target;
                let inner = detail_inner_height(state);
                let lo = state.view.block_viewport.line_offset;
                if state.view.detail_line_cursor >= lo + inner {
                    state.view.block_viewport.line_offset =
                        target.saturating_sub(inner.saturating_sub(1));
                }
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::ScrollFullUp => {
            let full = (state.rows / 2).max(1) as usize;
            if state.view.detail_line_cursor > 0 {
                state.view.detail_line_cursor = state.view.detail_line_cursor.saturating_sub(full);
                if state.view.detail_line_cursor < state.view.block_viewport.line_offset {
                    state.view.block_viewport.line_offset = state.view.detail_line_cursor;
                }
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::CopyCommand => {
            perform_block_action(state, BlockAction::CopyCommand);
        }
        DetailViewAction::CopyOutput => {
            let fmt = state.config.block_view.copy_format;
            let text = detail_copy_output(state);
            if let Some(text) = text {
                if write_to_clipboard(&text) {
                    state.render_state.flash_message =
                        Some((copy_flash(1, CopyPart::Output, fmt), Instant::now()));
                }
            }
            state.view.detail_visual_anchor = None;
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::CopyBoth => {
            let fmt = state.config.block_view.copy_format;
            let out = detail_copy_output(state).unwrap_or_default();
            let cmd = state
                .view
                .expanded_block
                .and_then(|id| state.blocks.block(id))
                .map(|b| b.command.clone())
                .unwrap_or_default();
            let text = format_blocks(
                &[&crate::app::CommandBlock {
                    command: cmd,
                    output_text: out,
                    ..crate::app::CommandBlock::default()
                }],
                CopyPart::Both,
                fmt,
            );
            if write_to_clipboard(&text) {
                state.render_state.flash_message =
                    Some((copy_flash(1, CopyPart::Both, fmt), Instant::now()));
            }
            state.view.detail_visual_anchor = None;
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::Rerun => {
            let command = state
                .view
                .expanded_block
                .and_then(|id| state.blocks.block(id))
                .map(|b| b.command.clone())
                .filter(|cmd| !cmd.is_empty());
            if let Some(cmd) = command {
                state.view = ViewState::default();
                state.input_accumulator.pending_block_delta = 0;
                state.render_state.needs_cleanup = true;
                state.render_state.pending_paste = Some(cmd);
            }
        }
        DetailViewAction::VisualMode => {
            if state.view.detail_visual_anchor.is_some() {
                state.view.detail_visual_anchor = None;
            } else {
                state.view.detail_visual_anchor = Some(state.view.detail_line_cursor);
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::Help => {
            state.view.help = Some(HelpState::open(ViewKind::Detail));
            state.view.view = ViewKind::Help;
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
        DetailViewAction::Quit => {
            state.view.view = ViewKind::Blocks;
            state.view.expanded_block = None;
            state.view.detail_line_cursor = 0;
            state.view.detail_visual_anchor = None;
            state.view.block_viewport.line_offset = 0;
            ensure_selected_visible(state);
            state.render_state.dirty = true;
            state.render_state.force_render = true;
        }
    }
}

fn handle_block_view_byte(byte: u8, state: &mut RuntimeState) -> bool {
    // Confirm dialog intercepts all input while open.
    if state.view.confirm.is_some() {
        match byte {
            b'y' | b'Y' | b'\r' | b'\n' => {
                if let Some(cs) = state.view.confirm.take() {
                    match cs.kind {
                        ConfirmKind::DeleteBlock | ConfirmKind::DeleteBlocks => {
                            execute_delete_blocks(state, cs.block_ids);
                        }
                        ConfirmKind::RerunBlocks => {
                            // Rerun the first command (sequential rerun is out of scope).
                            if let Some(&first_id) = cs.block_ids.first() {
                                let command = state
                                    .blocks
                                    .block(first_id)
                                    .map(|b| b.command.clone())
                                    .filter(|cmd| !cmd.is_empty());
                                if let Some(cmd) = command {
                                    exit_visual_mode(state);
                                    state.view = ViewState::default();
                                    state.input_accumulator.pending_block_delta = 0;
                                    state.render_state.needs_cleanup = true;
                                    state.render_state.pending_paste = Some(cmd);
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                state.view.confirm = None;
                state.render_state.dirty = true;
                state.render_state.force_render = true;
            }
        }
        return true;
    }

    if state.view.search_buffer.is_some() {
        return handle_search_input(byte, state);
    }

    if byte == b'b' {
        if let Some(target) = state.opencode_jump_stack.pop() {
            if tmux_target_exists(&target) && tmux_jump_and_zoom(&target) {
                state.render_state.flash_message = Some(("jumped back".to_string(), Instant::now()));
            } else {
                state.render_state.flash_message = Some(("back target missing".to_string(), Instant::now()));
            }
            state.render_state.dirty = true;
            state.render_state.force_render = true;
            return true;
        }
    }

    if let Some(action) = state.config.resolved_block_keymap.get(&byte) {
        return execute_block_view_action(*action, state);
    }

    true
}

fn accumulate_block_delta(state: &mut RuntimeState, delta: isize) {
    if is_navigation_boundary_noop(state, delta) {
        return;
    }

    let limit = (state.view.visible.len(&state.blocks).max(1).min(500)) as isize;
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
    let len = state.view.visible.len(&state.blocks);
    if len == 0 || pending.saturating_add(delta) != delta {
        return false;
    }

    let selected = state
        .view
        .block_viewport
        .selected_index
        .min(len.saturating_sub(1));
    (delta < 0 && selected == 0) || (delta > 0 && selected == len.saturating_sub(1))
}

fn select_relative_block(state: &mut RuntimeState, delta: isize) -> bool {
    let len = state.view.visible.len(&state.blocks);
    if len == 0 {
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
        .min(len.saturating_sub(1));
    let next = if delta.is_negative() {
        let magnitude = delta.checked_abs().unwrap_or(isize::MAX) as usize;
        current.saturating_sub(magnitude)
    } else {
        (current + delta as usize).min(len.saturating_sub(1))
    };
    if next == current {
        return false;
    }
    let old_scroll = state.view.block_viewport.line_offset;
    let old_anchor = state.view.block_viewport.anchor;
    let anchor = if state.config.block_view.auto_follow_on_reach_bottom
        && !delta.is_negative()
        && next == len.saturating_sub(1)
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
    let len = state.view.visible.len(&state.blocks);
    if len == 0 {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        return;
    }

    let last = len.saturating_sub(1);
    let selected = state.view.block_viewport.selected_index.min(last);
    select_block_index(state, selected, state.view.block_viewport.anchor);
}

fn select_block_index(state: &mut RuntimeState, index: usize, anchor: ViewAnchor) {
    let len = state.view.visible.len(&state.blocks);
    if len == 0 {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        state.view.block_viewport.anchor = anchor;
        return;
    }

    let idx = index.min(len.saturating_sub(1));
    state.view.block_viewport.selected_index = idx;
    state.view.selected_block = state.view.visible.block_at(&state.blocks, idx);
    // When in expanded mode, the expanded block follows the selection.
    if state.view.expanded_block.is_some() {
        state.view.expanded_block = state.view.selected_block;
    }
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
    let len = state.view.visible.len(&state.blocks);
    if len == 0 {
        state.view.selected_block = None;
        state.view.block_viewport.selected_index = 0;
        state.view.block_viewport.scroll_offset = 0;
        state.view.block_viewport.line_offset = 0;
        state.view.block_viewport.anchor = ViewAnchor::Tail;
        return;
    }

    let last = len.saturating_sub(1);
    state.view.block_viewport.selected_index = last;
    state.view.selected_block = state.view.visible.block_at(&state.blocks, last);
    state.view.block_viewport.anchor = ViewAnchor::Tail;
    state.view.block_viewport.line_offset = compute_tail_scroll_offset(state);
    if state.view.expanded_block.is_some() {
        state.view.expanded_block = state.view.selected_block;
    }
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

fn detail_output_line_count(state: &RuntimeState) -> usize {
    let Some(id) = state.view.expanded_block else {
        return 0;
    };
    let Some(block) = state.blocks.block(id) else {
        return 0;
    };
    if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
        return 1;
    }
    if block.output_raw.is_empty() {
        return 1;
    }
    let lines = crate::ansi::parse_ansi_lines(&block.output_raw);
    if lines.is_empty() { 1 } else { lines.len() }
}

fn detail_meta_line_count(state: &RuntimeState) -> usize {
    let Some(id) = state.view.expanded_block else {
        return 0;
    };
    let Some(block) = state.blocks.block(id) else {
        return 0;
    };
    let mut count: usize = 9;
    if block.output_truncated {
        count += 2;
    }
    if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
        count += 2;
    }
    count
}

fn detail_inner_height(state: &RuntimeState) -> usize {
    let meta = detail_meta_line_count(state);
    (state.rows as usize).saturating_sub(4).saturating_sub(meta)
}

fn ensure_selected_visible(state: &mut RuntimeState) {
    if state.view.visible.is_empty(&state.blocks) {
        return;
    }

    let layout = Compositor::build_visual_layout(
        &state.shell,
        &state.blocks,
        &state.view,
        state.cols,
        &state.config.block_view,
        None,
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

fn on_alt_screen_enter(state: &mut RuntimeState) {
    state.pty_alt_screen_active = true;
    // Extract app_name from Pending state (immutable borrow scope).
    let (is_known_tui, app_name) = {
        match &state.tui_state {
            TuiRuntimeState::Pending { app_match, .. } => (true, Some(app_match.app_name.clone())),
            _ => (false, None),
        }
    };
    // The block was already created by preexec as NormalCommand.
    // Promote it to TuiSession or RawProgram based on detection.
    if let Some(id) = state.blocks.active_block_id() {
        if let Some(block) = state.blocks.block_mut(id) {
            block.kind = if is_known_tui {
                BlockKind::TuiSession
            } else {
                BlockKind::RawProgram
            };
            if let Some(name) = app_name {
                block.app_name = Some(name);
            }
        }
        let prev = std::mem::take(&mut state.tui_state);
        let (next, _) = advance_tui_state(prev, TuiLifecycleEvent::AltScreenEnter { block_id: id });
        state.tui_state = next;
    } else {
        // No active block — shouldn't happen, but be defensive.
        state.tui_state = TuiRuntimeState::Idle;
    }
}

fn on_alt_screen_exit(state: &mut RuntimeState) {
    state.pty_alt_screen_active = false;
    let prev = std::mem::take(&mut state.tui_state);
    let (next, _) = advance_tui_state(prev, TuiLifecycleEvent::AltScreenExit);
    state.tui_state = next;
}

/// Called from precmd when a TUI session has exited alt-screen.
/// Determines whether to show the Return Panel or go directly to Plain view.
fn finalize_exited_tui_on_precmd(state: &mut RuntimeState, block_id: BlockId) {
    let app_name = state
        .blocks
        .block(block_id)
        .and_then(|b| b.app_name.clone())
        .unwrap_or_default();

    let app_cfg = state.config.tui_apps.get(&app_name).or_else(|| {
        state
            .config
            .tui_apps
            .values()
            .find(|cfg| cfg.commands.contains(&app_name))
    });

    let target = app_cfg
        .map(|cfg| cfg.return_panel)
        .unwrap_or(ReturnPanelTarget::None);
    let needs_clear = app_cfg
        .map(|cfg| cfg.after_exit.iter().any(|c| c == "clear"))
        .unwrap_or(false);

    match target {
        ReturnPanelTarget::None => {
            // No Return Panel — exit alt screen back to Plain transparently.
            state.view.return_panel = None;
            state.view.view = ViewKind::Plain;
            state.render_state.needs_cleanup = true;
            
            // If the TUI process exited but we still think it's in the alt screen,
            // it likely crashed without cleaning up. Force cleanup.
            if state.pty_alt_screen_active {
                state.render_state.force_pty_alt_screen_cleanup = true;
            }
        }
        _ => {
            let panel = crate::app::ReturnPanelState {
                block_id,
                target,
                clear_main_screen_before_show: needs_clear,
            };
            crate::app::enter_return_panel(&mut state.view, panel);
            state.render_state.force_render = true;
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

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::Once,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::config::Config;

    static TEST_REGISTRY_ENV: Once = Once::new();

    fn init_test_registry_env() {
        TEST_REGISTRY_ENV.call_once(|| {
            let dir = std::env::temp_dir().join(format!("tide-test-registry-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            // SAFETY: tests set this process-wide env var once before runtime state creation.
            unsafe { std::env::set_var("TIDE_REGISTRY_DIR", dir) };
        });
    }

    fn runtime_state() -> RuntimeState {
        init_test_registry_env();
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
            rows: 24,
            cols: 80,
            index: crate::index::BlockIndex::new(),
            tui_state: TuiRuntimeState::Idle,
            pty_alt_screen_active: false,
            tide_alt_screen_active: false,
            tide_id: "test".to_string(),
            opencode_by_block: HashMap::new(),
            opencode_jump_stack: Vec::new(),
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
        state.index.index_command(id, command);
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
            None,
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
    fn block_view_o_copies_output_to_clipboard() {
        let mut state = runtime_state();
        add_block(&mut state, "echo hello");
        enter_block_view(&mut state);
        state.render_state.dirty = false;
        state.render_state.force_render = false;

        assert!(handle_block_view_byte(b'o', &mut state));
        if write_to_clipboard("test") {
            let (msg, _) = state.render_state.flash_message.as_ref().unwrap();
            assert_eq!(msg, "copied output");
        }
    }

    #[test]
    fn block_view_c_copies_command_to_clipboard() {
        let mut state = runtime_state();
        add_block(&mut state, "echo hello");
        enter_block_view(&mut state);

        assert!(handle_block_view_byte(b'c', &mut state));
        if write_to_clipboard("test") {
            let (msg, _) = state.render_state.flash_message.as_ref().unwrap();
            assert_eq!(msg, "copied command");
        }
    }

    #[test]
    fn block_view_y_copies_combined_to_clipboard() {
        let mut state = runtime_state();
        add_block(&mut state, "echo hello");
        enter_block_view(&mut state);

        assert!(handle_block_view_byte(b'y', &mut state));
        if write_to_clipboard("test") {
            let (msg, _) = state.render_state.flash_message.as_ref().unwrap();
            assert_eq!(msg, "copied block");
        }
    }

    #[test]
    fn block_view_y_does_not_panic_with_empty_output() {
        let mut state = runtime_state();
        add_block(&mut state, "true");
        enter_block_view(&mut state);

        // o/y on a block with no output should never panic.
        assert!(handle_block_view_byte(b'o', &mut state));
        assert!(handle_block_view_byte(b'y', &mut state));
    }

    #[test]
    fn block_view_y_does_not_panic_with_no_blocks() {
        let mut state = runtime_state();
        enter_block_view(&mut state);

        // copy keys on empty block store should never panic.
        assert!(handle_block_view_byte(b'o', &mut state));
        assert!(handle_block_view_byte(b'y', &mut state));
    }

    #[test]
    fn tide_ui_transition_does_not_clobber_pty_alt_screen_state() {
        let mut state = runtime_state();

        // 1. PTY enters alt screen (e.g. nvim starts)
        on_alt_screen_enter(&mut state);
        assert!(state.pty_alt_screen_active);
        assert!(!state.tide_alt_screen_active);

        // 2. User enters Tide Block View (Ctrl-B)
        state.tide_alt_screen_active = true;
        enter_block_view(&mut state);
        assert!(state.pty_alt_screen_active);
        assert!(state.tide_alt_screen_active);

        // 3. User leaves Tide Block View (q)
        // Cleanup logic simulated:
        state.tide_alt_screen_active = false;
        state.render_state.needs_cleanup = false;

        // PTY state must be preserved
        assert!(state.pty_alt_screen_active);
    }

    #[test]
    fn unrelated_alt_screen_exit_preserves_pending_tui_state() {
        let mut state = runtime_state();
        let app_match = TuiAppMatch {
            app_name: "nvim".to_string(),
            command_name: "nvim".to_string(),
            source: TuiAppMatchSource::Builtin,
        };
        state.tui_state = TuiRuntimeState::Pending {
            app_match,
            command: "nvim".to_string(),
        };

        // Unrelated alt-screen exit observed before the TUI actually starts (or even if it's unrelated)
        on_alt_screen_exit(&mut state);

        // State must still be Pending so precmd can finalize it correctly if it never enters alt-screen,
        // or so that it can still transition to InAltScreen later.
        assert!(matches!(state.tui_state, TuiRuntimeState::Pending { .. }));
    }

    #[test]
    fn precmd_triggers_cleanup_only_if_pty_alt_screen_still_active() {
        let mut state = runtime_state();

        // Case A: Normal TUI exit (observed leave sequence)
        on_alt_screen_enter(&mut state);
        on_alt_screen_exit(&mut state);
        assert!(!state.pty_alt_screen_active);

        let block_id = state.blocks.start_command("yazi".to_string(), 0, BlockKind::TuiSession);
        state.tui_state = TuiRuntimeState::InAltScreen { block_id };
        // Simulated alt-screen exit happened before precmd
        on_alt_screen_exit(&mut state); 

        apply_shell_hook_event(&mut state, ShellHookEvent::Precmd { exit_code: 0, cwd: None }, false);
        // If pty_alt_screen_active is false, renderer::leave_block_render(stdout, false) 
        // would be called in the real loop, which is idempotent.
        assert!(!state.pty_alt_screen_active);
        assert!(!state.render_state.force_pty_alt_screen_cleanup);

        // Case B: Crashed TUI (no leave sequence observed)
        let block_id = state.blocks.start_command("crash".to_string(), 0, BlockKind::TuiSession);
        state.tui_state = TuiRuntimeState::InAltScreen { block_id };
        // We set pty_alt_screen_active to true manually to simulate what on_alt_screen_enter does
        state.pty_alt_screen_active = true; 

        apply_shell_hook_event(&mut state, ShellHookEvent::Precmd { exit_code: 1, cwd: None }, false);
        // Precmd should have set force_pty_alt_screen_cleanup because pty_alt_screen_active was true.
        assert!(state.render_state.needs_cleanup);
        assert!(state.pty_alt_screen_active);
        assert!(state.render_state.force_pty_alt_screen_cleanup);
    }

    #[test]
    fn tui_lifecycle_state_transitions_cover_key_paths() {
        #[derive(Clone, Copy)]
        enum Scenario {
            PendingThenPrecmdNoAlt,
            PendingThenUnrelatedExitThenPrecmd,
            PendingEnterExitPrecmd,
            PendingEnterPrecmdNoExit,
            NonTuiPrecmd,
        }

        let scenarios = [
            Scenario::PendingThenPrecmdNoAlt,
            Scenario::PendingThenUnrelatedExitThenPrecmd,
            Scenario::PendingEnterExitPrecmd,
            Scenario::PendingEnterPrecmdNoExit,
            Scenario::NonTuiPrecmd,
        ];

        for scenario in scenarios {
            let mut state = runtime_state();
            let command = match scenario {
                Scenario::NonTuiPrecmd => "ls".to_string(),
                _ => "nvim".to_string(),
            };

            apply_shell_hook_event(
                &mut state,
                ShellHookEvent::Preexec {
                    command: command.clone(),
                },
                false,
            );

            let block_id = state.blocks.active_block_id().expect("active block after preexec");

            match scenario {
                Scenario::PendingThenPrecmdNoAlt => {
                    assert!(matches!(state.tui_state, TuiRuntimeState::Pending { .. }));
                    apply_shell_hook_event(
                        &mut state,
                        ShellHookEvent::Precmd {
                            exit_code: 0,
                            cwd: None,
                        },
                        false,
                    );
                    assert!(matches!(state.tui_state, TuiRuntimeState::Idle));
                    let block = state.blocks.block(block_id).expect("finished block");
                    assert_eq!(block.kind, BlockKind::NormalCommand);
                    assert!(!state.render_state.needs_cleanup);
                }
                Scenario::PendingThenUnrelatedExitThenPrecmd => {
                    on_alt_screen_exit(&mut state);
                    assert!(matches!(state.tui_state, TuiRuntimeState::Pending { .. }));
                    apply_shell_hook_event(
                        &mut state,
                        ShellHookEvent::Precmd {
                            exit_code: 0,
                            cwd: None,
                        },
                        false,
                    );
                    assert!(matches!(state.tui_state, TuiRuntimeState::Idle));
                    let block = state.blocks.block(block_id).expect("finished block");
                    assert_eq!(block.kind, BlockKind::NormalCommand);
                    assert!(!state.render_state.needs_cleanup);
                }
                Scenario::PendingEnterExitPrecmd => {
                    on_alt_screen_enter(&mut state);
                    assert!(matches!(
                        state.tui_state,
                        TuiRuntimeState::InAltScreen { .. }
                    ));
                    on_alt_screen_exit(&mut state);
                    assert!(matches!(
                        state.tui_state,
                        TuiRuntimeState::ExitedAltScreen { .. }
                    ));
                    apply_shell_hook_event(
                        &mut state,
                        ShellHookEvent::Precmd {
                            exit_code: 0,
                            cwd: None,
                        },
                        false,
                    );
                    assert!(matches!(state.tui_state, TuiRuntimeState::Idle));
                    let block = state.blocks.block(block_id).expect("finished block");
                    assert_eq!(block.kind, BlockKind::TuiSession);
                    assert!(state.render_state.needs_cleanup);
                    assert!(!state.render_state.force_pty_alt_screen_cleanup);
                }
                Scenario::PendingEnterPrecmdNoExit => {
                    on_alt_screen_enter(&mut state);
                    assert!(state.pty_alt_screen_active);
                    apply_shell_hook_event(
                        &mut state,
                        ShellHookEvent::Precmd {
                            exit_code: 1,
                            cwd: None,
                        },
                        false,
                    );
                    assert!(matches!(state.tui_state, TuiRuntimeState::Idle));
                    let block = state.blocks.block(block_id).expect("finished block");
                    assert_eq!(block.kind, BlockKind::TuiSession);
                    assert!(state.render_state.needs_cleanup);
                    assert!(state.render_state.force_pty_alt_screen_cleanup);
                }
                Scenario::NonTuiPrecmd => {
                    assert!(matches!(state.tui_state, TuiRuntimeState::Idle));
                    apply_shell_hook_event(
                        &mut state,
                        ShellHookEvent::Precmd {
                            exit_code: 0,
                            cwd: None,
                        },
                        false,
                    );
                    assert!(matches!(state.tui_state, TuiRuntimeState::Idle));
                    let block = state.blocks.block(block_id).expect("finished block");
                    assert_eq!(block.kind, BlockKind::NormalCommand);
                    assert!(!state.render_state.needs_cleanup);
                }
            }
        }
    }

    #[test]
    fn advance_tui_state_transition_table() {
        let app_match = TuiAppMatch {
            app_name: "nvim".to_string(),
            command_name: "nvim".to_string(),
            source: TuiAppMatchSource::Builtin,
        };
        let block_id = BlockId(42);

        let (state, finalize) = advance_tui_state(
            TuiRuntimeState::Idle,
            TuiLifecycleEvent::PreexecMatch {
                app_match: app_match.clone(),
                command: "nvim".to_string(),
            },
        );
        assert!(matches!(state, TuiRuntimeState::Pending { .. }));
        assert!(finalize.is_none());

        let (state, finalize) = advance_tui_state(
            TuiRuntimeState::Pending {
                app_match: app_match.clone(),
                command: "nvim".to_string(),
            },
            TuiLifecycleEvent::AltScreenEnter { block_id },
        );
        assert!(matches!(
            state,
            TuiRuntimeState::InAltScreen {
                block_id: BlockId(42)
            }
        ));
        assert!(finalize.is_none());

        let (state, finalize) = advance_tui_state(
            TuiRuntimeState::InAltScreen { block_id },
            TuiLifecycleEvent::AltScreenExit,
        );
        assert!(matches!(
            state,
            TuiRuntimeState::ExitedAltScreen {
                block_id: BlockId(42)
            }
        ));
        assert!(finalize.is_none());

        let (state, finalize) =
            advance_tui_state(TuiRuntimeState::ExitedAltScreen { block_id }, TuiLifecycleEvent::Precmd);
        assert!(matches!(state, TuiRuntimeState::Idle));
        assert_eq!(finalize, Some(block_id));

        let (state, finalize) = advance_tui_state(
            TuiRuntimeState::Pending {
                app_match,
                command: "nvim".to_string(),
            },
            TuiLifecycleEvent::AltScreenExit,
        );
        assert!(matches!(state, TuiRuntimeState::Pending { .. }));
        assert!(finalize.is_none());
    }

    #[test]
    fn live_search_filters_while_typing() {
        let mut state = runtime_state();
        add_block(&mut state, "cargo test");
        add_block(&mut state, "ls -la");
        enter_block_view(&mut state);

        // Open search bar
        assert!(handle_block_view_byte(b'/', &mut state));
        assert_eq!(state.view.filter.command_query, "");

        // Type 'c' — visible should filter to "cargo test"
        assert!(handle_search_input(b'c', &mut state));
        assert_eq!(state.view.search_buffer.as_deref(), Some("c"));
        assert_eq!(state.view.visible.len(&state.blocks), 1);

        // Type 'a'
        assert!(handle_search_input(b'a', &mut state));
        assert_eq!(state.view.search_buffer.as_deref(), Some("ca"));
        assert_eq!(state.view.visible.len(&state.blocks), 1);

        // Type 'r'
        assert!(handle_search_input(b'r', &mut state));
        assert_eq!(state.view.search_buffer.as_deref(), Some("car"));
        assert_eq!(state.view.visible.len(&state.blocks), 1);

        // Type 'z' — no match
        assert!(handle_search_input(b'z', &mut state));
        assert_eq!(state.view.visible.len(&state.blocks), 0);

        // Backspace to remove 'z' — back to "car"
        assert!(handle_search_input(b'\x7f', &mut state));
        assert_eq!(state.view.search_buffer.as_deref(), Some("car"));
        assert_eq!(state.view.visible.len(&state.blocks), 1);
    }

    #[test]
    fn esc_restores_pre_search_filter() {
        let mut state = runtime_state();
        add_block(&mut state, "cargo test");
        add_block(&mut state, "ls -la");
        enter_block_view(&mut state);

        // Set an active filter via n/N (simulate by setting filter directly)
        state.view.filter.command_query = "cargo".to_string();
        rebuild_visible(&mut state);
        assert_eq!(state.view.visible.len(&state.blocks), 1);

        // Open search bar — saves pre_search_query, command_query unchanged
        assert!(handle_block_view_byte(b'/', &mut state));
        assert_eq!(state.view.pre_search_query, "cargo");
        assert_eq!(state.view.filter.command_query, "cargo");

        // Type 'z' — filter changes to empty
        assert!(handle_search_input(b'z', &mut state));
        assert_eq!(state.view.filter.command_query, "z");
        assert_eq!(state.view.visible.len(&state.blocks), 0);

        // Esc restores pre_search_query
        assert!(handle_search_input(b'\x1b', &mut state));
        assert_eq!(state.view.filter.command_query, "cargo");
        assert!(state.view.search_buffer.is_none());
        assert_eq!(state.view.visible.len(&state.blocks), 1);
    }

    // ─── Rerun flow tests ────────────────────────────────────────────────

    #[test]
    fn rerun_single_block_sets_pending_paste() {
        let mut state = runtime_state();
        add_block(&mut state, "echo hello");
        enter_block_view(&mut state);

        assert!(handle_block_view_byte(b'r', &mut state));

        assert!(state.render_state.needs_cleanup);
        assert_eq!(
            state.render_state.pending_paste.as_deref(),
            Some("echo hello")
        );
        assert!(matches!(state.view.view, ViewKind::Plain));
        assert_eq!(state.input_accumulator.pending_block_delta, 0);
    }

    #[test]
    fn rerun_empty_command_does_nothing() {
        let mut state = runtime_state();
        add_block(&mut state, "");
        enter_block_view(&mut state);

        state.render_state.needs_cleanup = false;
        state.render_state.pending_paste = None;

        assert!(handle_block_view_byte(b'r', &mut state));

        assert!(state.render_state.pending_paste.is_none());
        assert!(!state.render_state.needs_cleanup);
    }

    #[test]
    fn rerun_visual_multi_shows_confirm() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        add_block(&mut state, "echo two");
        add_block(&mut state, "echo three");
        enter_block_view(&mut state);

        // Enter visual mode on first block (index 0)
        select_block_index(&mut state, 0, ViewAnchor::Manual);
        state.view.visual_anchor = state.view.selected_block;

        // Navigate to block at index 2, extending visual selection to [0, 1, 2]
        select_block_index(&mut state, 2, ViewAnchor::Manual);

        // r opens confirm with RerunBlocks
        assert!(handle_block_view_byte(b'r', &mut state));
        let cs = state.view.confirm.as_ref().unwrap();
        assert_eq!(cs.kind, ConfirmKind::RerunBlocks);
        assert_eq!(cs.block_ids.len(), 3);
    }

    #[test]
    fn rerun_visual_confirm_reruns_first_block() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        add_block(&mut state, "echo two");
        add_block(&mut state, "echo three");
        enter_block_view(&mut state);

        // Visual select all three blocks
        select_block_index(&mut state, 0, ViewAnchor::Manual);
        state.view.visual_anchor = state.view.selected_block;
        select_block_index(&mut state, 2, ViewAnchor::Manual);

        // r opens confirm
        assert!(handle_block_view_byte(b'r', &mut state));
        assert!(state.view.confirm.is_some());

        // y confirms — reruns first block only
        assert!(handle_block_view_byte(b'y', &mut state));
        assert!(state.view.confirm.is_none());
        assert_eq!(
            state.render_state.pending_paste.as_deref(),
            Some("echo one")
        );
        assert!(state.render_state.needs_cleanup);
        assert!(state.view.visual_anchor.is_none());
        assert!(matches!(state.view.view, ViewKind::Plain));
    }

    #[test]
    fn rerun_visual_cancel_leaves_state_intact() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        add_block(&mut state, "echo two");
        add_block(&mut state, "echo three");
        enter_block_view(&mut state);

        // Visual select all three blocks
        select_block_index(&mut state, 0, ViewAnchor::Manual);
        state.view.visual_anchor = state.view.selected_block;
        select_block_index(&mut state, 2, ViewAnchor::Manual);

        // r opens confirm
        assert!(handle_block_view_byte(b'r', &mut state));
        assert!(state.view.confirm.is_some());

        // n cancels
        assert!(handle_block_view_byte(b'n', &mut state));
        assert!(state.view.confirm.is_none());
        assert!(state.render_state.pending_paste.is_none());
        assert!(!state.render_state.needs_cleanup);
        assert_eq!(state.blocks.len(), 3);
    }

    #[test]
    fn rerun_from_detail_view_sets_pending_paste() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "cargo build");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"r", &state), Some(1));

        let s = state.lock().unwrap();
        assert_eq!(s.render_state.pending_paste.as_deref(), Some("cargo build"));
        assert!(s.render_state.needs_cleanup);
        assert!(matches!(s.view.view, ViewKind::Plain));
        assert_eq!(s.input_accumulator.pending_block_delta, 0);
    }

    // ─── Delete flow tests ───────────────────────────────────────────────

    #[test]
    fn delete_single_block_via_confirm() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        add_block(&mut state, "echo two");
        add_block(&mut state, "echo three");
        enter_block_view(&mut state);

        // Select middle block
        select_block_index(&mut state, 1, ViewAnchor::Manual);
        let deleted_id = state.view.selected_block.unwrap();

        // Press d — opens confirm dialog
        assert!(handle_block_view_byte(b'd', &mut state));
        let cs = state.view.confirm.as_ref().unwrap();
        assert_eq!(cs.kind, ConfirmKind::DeleteBlock);
        assert_eq!(cs.block_ids, vec![deleted_id]);

        // Press y to confirm
        state.render_state.dirty = false;
        state.render_state.force_render = false;
        assert!(handle_block_view_byte(b'y', &mut state));
        assert!(state.view.confirm.is_none());

        // Block removed from store
        assert_eq!(state.blocks.len(), 2);
        assert!(state.blocks.block(deleted_id).is_none());

        // Selection moves to neighbor (next block after deleted one)
        let remaining: Vec<BlockId> = state.blocks.timeline.iter().copied().collect();
        assert_ne!(remaining, vec![deleted_id]);
        assert!(state.view.selected_block.is_some());
        assert_ne!(state.view.selected_block, Some(deleted_id));
    }

    #[test]
    fn delete_cancel_leaves_block_intact() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        let original_len = state.blocks.len();
        enter_block_view(&mut state);
        let original_id = state.view.selected_block.unwrap();

        // Press d — opens confirm
        assert!(handle_block_view_byte(b'd', &mut state));
        assert!(state.view.confirm.is_some());

        // Press n to cancel (any non-confirm key works)
        assert!(handle_block_view_byte(b'n', &mut state));
        assert!(state.view.confirm.is_none());

        // Block store untouched
        assert_eq!(state.blocks.len(), original_len);
        assert!(state.blocks.block(original_id).is_some());
    }

    #[test]
    fn delete_last_block_leaves_empty_store() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        enter_block_view(&mut state);

        // d + y to confirm
        assert!(handle_block_view_byte(b'd', &mut state));
        assert!(handle_block_view_byte(b'y', &mut state));

        assert_eq!(state.blocks.len(), 0);
        assert!(state.view.selected_block.is_none());
        assert_eq!(state.view.block_viewport.selected_index, 0);
    }

    #[test]
    fn delete_clears_expanded_block() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one");
        add_block(&mut state, "echo two");
        enter_block_view(&mut state);

        // Enter expands the selected block (tail selects the last one = BlockId 2)
        assert!(handle_block_view_byte(b'\r', &mut state));
        assert_eq!(state.view.expanded_block, state.view.selected_block);
        let deleted_id = state.view.selected_block.unwrap();

        // d + y to delete the expanded block
        assert!(handle_block_view_byte(b'd', &mut state));
        assert!(handle_block_view_byte(b'y', &mut state));

        assert_eq!(state.blocks.len(), 1);
        assert!(state.view.expanded_block.is_none());
        assert!(state.view.selected_block.is_some());
        assert_ne!(state.view.selected_block, Some(deleted_id));
    }

    #[test]
    fn delete_visual_selection_removes_multiple() {
        let mut state = runtime_state();
        add_block(&mut state, "echo one"); // BlockId 1
        add_block(&mut state, "echo two"); // BlockId 2
        add_block(&mut state, "echo three"); // BlockId 3
        add_block(&mut state, "echo four"); // BlockId 4
        enter_block_view(&mut state);

        // Start visual mode on first block (index 0)
        select_block_index(&mut state, 0, ViewAnchor::Manual);
        state.view.visual_anchor = state.view.selected_block;

        // Navigate to block at index 2, extending visual selection to [0, 1, 2]
        select_block_index(&mut state, 2, ViewAnchor::Manual);
        assert!(state.view.visual_anchor.is_some());

        // d + y removes blocks 0-2 (3 blocks)
        assert!(handle_block_view_byte(b'd', &mut state));
        let cs = state.view.confirm.as_ref().unwrap();
        assert_eq!(cs.kind, ConfirmKind::DeleteBlocks);
        assert_eq!(cs.block_ids.len(), 3);
        let deleted_ids = cs.block_ids.clone();

        assert!(handle_block_view_byte(b'y', &mut state));
        assert!(state.view.confirm.is_none());

        // Only block at index 3 remains
        assert_eq!(state.blocks.len(), 1);
        for id in &deleted_ids {
            assert!(state.blocks.block(*id).is_none());
        }

        // Visual mode exited
        assert!(state.view.visual_anchor.is_none());

        // Selection moved to surviving neighbor
        assert!(state.view.selected_block.is_some());
        assert!(!deleted_ids.contains(&state.view.selected_block.unwrap()));
    }

    #[test]
    fn detail_j_scrolls_cursor_down() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block_with_lines(&mut s, "echo", &["a", "b", "c"]);
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"j", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "j should stay in Detail"
        );
        assert_eq!(s.view.detail_line_cursor, 1);
    }

    #[test]
    fn detail_k_scrolls_cursor_up() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block_with_lines(&mut s, "echo", &["a", "b", "c"]);
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
            s.view.detail_line_cursor = 1;
        }

        assert_eq!(handle_view_key_sequence(b"k", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "k should stay in Detail"
        );
        assert_eq!(s.view.detail_line_cursor, 0);
    }

    #[test]
    fn detail_g_jumps_to_top() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo one");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
            s.view.detail_line_cursor = 5;
        }

        assert_eq!(handle_view_key_sequence(b"g", &state), Some(1));
        let s = state.lock().unwrap();
        assert_eq!(s.view.detail_line_cursor, 0);
        assert_eq!(s.view.block_viewport.line_offset, 0);
    }

    #[test]
    fn detail_g_upper_jumps_to_bottom() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block_with_lines(
                &mut s,
                "echo",
                &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
            );
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"G", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(
            matches!(s.view.view, ViewKind::Detail),
            "G should stay in Detail"
        );
        // 10 lines, inner_height = 24-4 = 20 → short mode.
        // G sets cursor = total-1 = 9, line_offset = total.saturating_sub(inner_height) = 0.
        assert_eq!(s.view.detail_line_cursor, 9);
        assert_eq!(s.view.block_viewport.line_offset, 0);
    }

    #[test]
    fn detail_c_copies_command() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo hello");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"c", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(matches!(s.view.view, ViewKind::Detail));
        if write_to_clipboard("test") {
            let (msg, _) = s.render_state.flash_message.as_ref().unwrap();
            assert!(
                msg.contains("command"),
                "flash should mention 'command', got: {msg}"
            );
        }
    }

    #[test]
    fn detail_o_copies_output() {
        let state = Arc::new(Mutex::new(runtime_state()));
        {
            let mut s = state.lock().unwrap();
            add_block(&mut s, "echo hello");
            enter_block_view(&mut s);
            s.view.view = ViewKind::Detail;
            s.view.expanded_block = s.view.selected_block;
        }

        assert_eq!(handle_view_key_sequence(b"o", &state), Some(1));
        let s = state.lock().unwrap();
        assert!(matches!(s.view.view, ViewKind::Detail));
        if write_to_clipboard("test") {
            assert!(s.render_state.flash_message.is_some());
        }
    }

    // ─── extract_command_name tests ──────────────────────────────────────

    #[test]
    fn extract_bare_command() {
        assert_eq!(extract_command_name("vim main.rs"), Some("vim".into()));
    }

    #[test]
    fn extract_sudo_prefix() {
        assert_eq!(extract_command_name("sudo nvim"), Some("nvim".into()));
    }

    #[test]
    fn extract_sudo_with_flag_arg() {
        assert_eq!(
            extract_command_name("sudo -u alice nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_sudo_with_multiple_flags() {
        assert_eq!(
            extract_command_name("sudo -E -u alice nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_sudo_flag_arg_before_command() {
        assert_eq!(
            extract_command_name("sudo -u alice -E nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_doas_with_flag_arg() {
        assert_eq!(
            extract_command_name("doas -u bob nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_env_key_value() {
        assert_eq!(
            extract_command_name("env FOO=bar nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_env_unset_flag() {
        assert_eq!(
            extract_command_name("env -u HOME nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_env_key_value_before_unset() {
        assert_eq!(
            extract_command_name("env FOO=bar -u HOME nvim"),
            Some("nvim".into())
        );
    }

    #[test]
    fn extract_command_prefixes() {
        assert_eq!(extract_command_name("command nvim"), Some("nvim".into()));
        assert_eq!(extract_command_name("noglob nvim"), Some("nvim".into()));
        assert_eq!(extract_command_name("builtin echo"), Some("echo".into()));
        assert_eq!(extract_command_name("time nvim"), Some("nvim".into()));
    }

    #[test]
    fn extract_empty_input() {
        assert_eq!(extract_command_name(""), None);
    }

    #[test]
    fn extract_only_prefixes() {
        assert_eq!(extract_command_name("sudo"), None);
    }

    #[test]
    fn extract_key_value_only() {
        assert_eq!(extract_command_name("FOO=bar"), None);
    }

    #[test]
    fn extract_basename_strips_path() {
        assert_eq!(extract_command_name("/usr/bin/nvim"), Some("nvim".into()));
    }

    #[test]
    fn opencode_process_detection_matches_real_binary_path() {
        assert!(is_opencode_process_command(
            "/opt/homebrew/Cellar/opencode/1.14.40/libexec/lib/node_modules/opencode-ai/node_modules/opencode-darwin-arm64/bin/opencode"
        ));
    }

    #[test]
    fn opencode_process_detection_rejects_text_mentions() {
        assert!(!is_opencode_process_command("vim opencode.md"));
        assert!(!is_opencode_process_command("cat opencode.log"));
        assert!(!is_opencode_process_command("echo opencode"));
    }

    // ─── detect_tui_app tests ────────────────────────────────────────────

    fn empty_tui_apps() -> std::collections::BTreeMap<String, crate::config::TuiAppConfig> {
        std::collections::BTreeMap::new()
    }

    #[test]
    fn detect_non_tui_returns_none() {
        assert!(detect_tui_app("ls", &[], &empty_tui_apps()).is_none());
    }

    #[test]
    fn detect_default_tui_command() {
        let result = detect_tui_app("lazygit", &[], &empty_tui_apps());
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().command_name, "lazygit");
        assert_eq!(result.as_ref().unwrap().source, TuiAppMatchSource::Builtin);
    }

    #[test]
    fn detect_sudo_default_tui() {
        let result = detect_tui_app("sudo -u alice lazygit", &[], &empty_tui_apps());
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().command_name, "lazygit");
    }

    #[test]
    fn detect_extra_commands() {
        let result = detect_tui_app("myapp", &["myapp".into()], &empty_tui_apps());
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().command_name, "myapp");
        assert_eq!(
            result.as_ref().unwrap().source,
            TuiAppMatchSource::UserConfig
        );
    }

    #[test]
    fn detect_app_config_exact_match() {
        let cfg = crate::config::TuiAppConfig {
            commands: vec!["custom-tui".into()],
            handoff: false,
            snapshot: vec![],
            after_exit: vec![],
            return_panel: crate::app::ReturnPanelTarget::None,
        };
        let mut apps = std::collections::BTreeMap::new();
        apps.insert("my-custom-app".into(), cfg);
        let result = detect_tui_app("custom-tui", &[], &apps);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().app_name, "my-custom-app");
        assert_eq!(
            result.as_ref().unwrap().source,
            TuiAppMatchSource::UserConfig
        );
    }

    #[test]
    fn detect_env_with_tui() {
        let result = detect_tui_app("env FOO=bar nvim", &[], &empty_tui_apps());
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().command_name, "nvim");
    }

    #[test]
    fn detect_env_unset_with_tui() {
        let result = detect_tui_app("env -u HOME lazygit", &[], &empty_tui_apps());
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().command_name, "lazygit");
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
