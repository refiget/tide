# Architecture

## Project Positioning

Tide is a multi-mode shell wrapper / terminal application that runs on top of real `zsh`.

It is not a terminal emulator and not a replacement for the user's terminal emulator or shell. Tide starts real `zsh` in a PTY, captures shell output and lifecycle markers, stores that information in internal buffers, and renders switchable layers back into the existing terminal.

The key idea is that Tide gives zsh a layer system:

- Plain / Normal View is ordinary zsh passthrough.
- Block View overlays structured command metadata on the same shell history.
- Detail View is a full-screen pager for a single block's output and metadata.

## Non-Goals

Current-stage non-goals:

- Do not build a full terminal emulator.
- Do not scrape the real terminal scrollback.
- Do not infer command boundaries from prompt regexes.
- Do not require users to change zsh themes.
- Do not build Block View as a separate list page.
- Do not build block details as popups or modals.
- Do not parse full-screen programs such as `vim`, `yazi`, `fzf`, `less`, `top`, `ssh`, or `lazygit` into ordinary shell lines.
- Do not add OpenCode integration.
- Do not add complex AI or natural-language workflows.
- Do not add database persistence.
- Do not build a complete TUI handoff-return system yet.

## Core Data Flow

Tide's Normal mode is pass-through display, but not pass-through state. Tide strips its own invisible markers, forwards visible PTY bytes to the real terminal, and captures sidecar history for later Block View rendering.

```text
Normal:
zsh PTY output
  -> Marker Parser
      -> visible bytes -> Real Terminal
      -> sidecar capture -> ShellBuffer + BlockStore

Block / Detail:
ShellBuffer + BlockStore + ViewState
  -> Compositor
  -> VisualLine
  -> Renderer
  -> Real Terminal
```

Responsibilities in this flow:

- `Marker Parser` splits visible shell bytes from invisible shell markers.
- Normal mode writes visible bytes to the terminal immediately.
- Sidecar capture stores best-effort plain text in `ShellBuffer`.
- `ShellBuffer` stores shell text lines.
- `BlockStore` stores structured command execution data.
- `Compositor` combines shell text, block data, and view state into visual lines for Block / Detail / Help views.
- `Renderer` draws visual lines only when Tide is in a reconstructed view.

## Module Layout

Current source layout (flat `src/` modules):

```text
src/
  main.rs          — thin entry point, loads config, starts PTY session
  agent_registry.rs— global shared agent records + jump stack persistence (file-backed)
  app.rs           — ViewState, ViewKind, InputMode, BlockViewport, ViewAnchor,
                     InputAccumulator, RenderState, AppEvent, CommandBlock,
                     BlockKind, BlockStatus, BlockAction, BlockFilter,
                     VisibleSource, HelpState, ConfirmState, ConfirmKind,
                     FooterSegment, BlockViewAction (22 variants),
                     DetailViewAction (15 variants)
  pty.rs           — PTY session, marker parser integration, input/output threads,
                     frame-rate-limited render loop, viewport math, navigation,
                     keyboard dispatch, copy/rerun/delete/help/confirm logic
  block.rs         — BlockStore (timeline + HashMap), block lifecycle, duration formatting
  buffer.rs        — ShellBuffer, ShellLine, ANSI escape sequence handling
  compositor.rs    — VisualLine enum, Compositor (ShellBuffer + BlockStore + ViewState → VisualLine),
                     visible range computation, tail scroll offset, block height estimation,
                     Detail View pager (build_detail_lines), footer segments
  renderer.rs      — terminal render function, border drawing, framed text, truncation,
                     Help overlay (render_help_overlay), Confirm overlay (render_confirm_overlay),
                     BlockSelectionStyle, BLOCK_HELP_ENTRIES / DETAIL_HELP_ENTRIES,
                     enter_block_render / leave_block_render
  shell_hooks.rs   — Osc777Parser, ShellHookEvent, ParsedPtyPart, zsh install script
  config.rs        — Config loading (TOML), BlockViewConfig, BlockLayoutConfig, RuntimeConfig,
                     keymap resolution (build_resolved_block_keymap, build_resolved_detail_keymap)
  format.rs        — compact_command, compact_cwd, TopLabel, build_top_label_parts,
                     build_top_label, CopyFormat, CopyPart, format_blocks
  index.rs         — BlockIndex, token inverted index for command search (substring, AND),
                     failed block tracking
  ansi.rs          — parse_ansi_lines, StyledText, StyledSpan, TextStyle,
                     styled_width, truncate_styled_to_width, styled_to_plain
  theme.rs         — Catppuccin Frappe color constants, Theme role-aliased colors
```

### app.rs

Owns top-level app state types: `ViewKind`, `InputMode`, `ViewState`, `BlockViewport`, `ViewAnchor`, `InputAccumulator`, `RenderState`, `AppEvent`, `CommandBlock`, `BlockKind`, `BlockStatus`, `BlockAction`, `BlockFilter`, `VisibleSource`, `HelpState`, `ConfirmState`, `ConfirmKind`, `FooterSegment`, `BlockViewAction` (22 variants), `DetailViewAction` (15 variants).

### pty.rs

Starts and manages the zsh PTY, runs input/output/resize threads, integrates the marker parser, captures shell output into `ShellBuffer` and `BlockStore`, handles keyboard input (view-mode switching and Block View navigation), and coordinates frame-rate-limited rendering.

Three-thread runtime: output thread (reads PTY, captures output, renders Block/Detail), input thread (reads stdin, dispatches keys, forwards to PTY, calls `maybe_flush_navigation_and_render`), resize thread (SIGWINCH → PTY + stored dimensions).

Block View uses a visual-line viewport: selection moves by block, but the viewport slices the complete visual layout by `line_offset`.

**Alternate screen lifecycle** — Block/Detail rendering happens in the alternate screen buffer, completely isolated from the main terminal display:
- `Ctrl-B` → input thread drops the state lock, locks stdout to enter alt screen (`enter_block_render`), re-acquires state, and sets view to Blocks. All subsequent Block/Detail renders write to the alt screen.
- `q`/`Esc` in Block View → `handle_block_view_byte` sets `needs_cleanup = true` (not `dirty`/`force_render`, to avoid rendering Plain view through the renderer). The input thread's post-byte-loop handler leaves the alt screen (`leave_block_render`) and resets cleanup flags. PTY output after cleanup goes to the restored main screen normally.
- Lock ordering: output thread always locks `(state) → (stdout)`. The input thread's Ctrl-B handler explicitly drops the state guard before locking stdout, then re-acquires state — never holding both simultaneously.

Input handler dispatches through configurable keymaps (`resolved_block_keymap`, `resolved_detail_keymap`) for single-byte actions. Esc sequences (`\x1b[A`, `\x1b[B`) are hardcoded.

Shared agent integration:
- Per-provider config in `RuntimeConfig.agents: HashMap<AgentProvider, AgentShareConfig>`.
- Detect provider runtime from command name or TTY process scan (`detect_and_register_agents`).
- Register running sessions into `agent_registry`; unregister on `precmd`.
- Inject synthetic shared blocks (`origin=Shared`, `synthetic=true`, `actions=JumpOnly`, `agent_ref=Some(AgentRef{provider,alias})`).
- `i` on a shared block reads `block.agent_ref`, looks up tmux target via registry, and jumps.
- Jump-back via file-backed `agent_registry` jump stack (`write_last_jump` / `pop_jump_for_target`).
- `Ctrl-B` first attempts jump-back; only enters Block View in shell-normal state.
- On shell `chpwd` and `precmd` cwd markers, Tide updates its own process cwd so tmux pane path tracking stays aligned. The zsh integration also emits OSC 7 cwd sequences, which Tide passes through unchanged.

### block.rs

Provides `BlockStore` with a `Vec<BlockId>` timeline and `HashMap<BlockId, CommandBlock>` lookup. Controls retention via `max_blocks`. Methods: `start_command`, `append_output`, `finish_command`, `block`, `block_mut`, `block_id_at`, `len`, `remove`, `set_cwd`. Enforces `max_output_bytes_per_block` with `output_truncated` flag.

### buffer.rs

Owns shell text storage via `ShellBuffer`. Supports `append` with ANSI escape sequence handling (cursor movement, erase). Provides `snapshot()` for rendering and `cursor_position()` for Plain View cursor placement. Must not contain block borders, metadata, or detail text.

### compositor.rs

Core of Block/Detail/Help View rendering. `build_visual_layout` produces a complete `VisualLayout` with `VisualLine` values and per-block spans; `build_visual_lines` slices that layout by `BlockViewport.line_offset` and content height. This is the single source of truth for viewport math and allows partial non-selected blocks at the top or bottom while keeping the selected block fully visible when possible.

Detail View mode: `build_detail_lines` produces a full-screen pager layout for the expanded block: top border, scrollable styled output lines with line cursor and visual selection, metadata lines, bottom border, and footer. Uses `detail_line_cursor` and `detail_visual_anchor` for navigation within output.

Help View mode: delegates to Block or Detail layout with selection suppressed (`unsel`), then the renderer overlays the help keybindings dialog.

See [block-layer.md](block-layer.md) for the block data model and visual generation rules.

### renderer.rs

Provides `render()` that draws `&[VisualLine]` to the terminal. Handles border characters (selected: `╭ ╮ ╰ ╯`, unselected: `┌ ┐ └ ┘`), framed text, titled borders, footer, cursor positioning, unicode-width-aware truncation, styled text rendering, and search match highlighting.

Uses `BlockSelectionStyle` to centralise selection appearance: `selected()` uses LAVENDER borders, `normal()` uses SURFACE2, `visual()` uses YELLOW borders. Extended to all 5 render paths (top border, body, detail line, bottom border, styled body lines).

Renders the **Help overlay** (`render_help_overlay`): a centered floating dialog showing keybinding entries from `BLOCK_HELP_ENTRIES` or `DETAIL_HELP_ENTRIES` with scroll, cursor highlight, and page counter.

Renders the **Confirm overlay** (`render_confirm_overlay`): a centered dialog for destructive operations (delete block(s), rerun multiple blocks) showing the action description, hint text, and [Y]es/(N)o actions.

Optimises flicker by tracking `HelpState::underlying_rendered`: on first Help frame both underlying view and overlay are rendered in a single flush; subsequent navigations redraw only the overlay.

Also exposes `enter_block_render()` / `leave_block_render()` for alternate screen lifecycle:
- `enter_block_render` — switches to the alternate screen buffer and hides the cursor. Called when entering Block View (Ctrl-B).
- `leave_block_render` — leaves the alternate screen, resets SGR attributes, and shows the cursor. Called in the cleanup path when returning to Plain view (q/Esc). **Order matters**: `LeaveAlternateScreen` must come first so that `ResetColor` and `Show` apply on the newly-restored main screen, not on the alt screen that is about to be discarded.

### shell_hooks.rs

Owns zsh hook definitions (`install_script()`), the `Osc777Parser` that strips invisible OSC 777 markers from PTY output, and marker parsing (`parse_block_marker`). Tide markers include command start/end and immediate cwd updates; non-Tide OSC sequences remain visible to the terminal.

### agent_registry.rs

File-backed global registry for shared agent navigation metadata and jump-back stack:
- Stores minimal provider records (`provider`, `alias`, status, tmux pane/window/target, timestamps).
- Stores jump stack (`from`, `to`, `from_zoomed`, timestamp), bounded in-memory/file model.
- Uses lock file + atomic write/rename pattern for cross-instance safety.

### config.rs

Loads TOML config from `~/.config/tide/config.toml` or `config/tide.toml`. Provides `BlockViewConfig`, `BlockLayoutConfig`, `RuntimeConfig`, keymap resolution, and defaults. See [config.md](config.md).

### format.rs

Provides `compact_command()` and `compact_cwd()` for truncating long text in top border labels. `build_top_label_parts()` and `build_top_label()` produce the structured and flat top border strings. `CopyPart` / `CopyFormat` / `format_blocks()` handle clipboard serialization in plaintext, markdown, shell transcript, and JSON formats.

### index.rs

`BlockIndex` maintains a token inverted index of command text. Supports substring token match with AND semantics. Also tracks failed blocks for the failed-only filter. Used by pty.rs to rebuild `VisibleSource::Filtered` on search/filter changes.

### ansi.rs

`parse_ansi_lines()` converts raw bytes into `Vec<StyledText>` with per-line `StyledSpan` sequences. `styled_to_plain()` extracts plain text. `styled_width()` / `truncate_styled_to_width()` handle display-width-aware truncation for styled content.

### theme.rs

Catppuccin Frappe color constants plus `Theme` role-aliased semantic colors used across all rendering (borders, status, metadata, help, icons, cursor, visual selection, search matches).

## Input Modes vs Display Layers

Input behavior and display rendering are related but separate concepts.

`InputMode` describes how keys are interpreted:

- `Shell`
- `BlockNav`
- `DetailNav`
- `NaturalLanguage`, future
- `OpenCode`, future
- `RawProgram`, future/reserved

`ViewKind` describes what is rendered:

- `Plain`
- `Blocks`
- `Detail`
- `Help`
- `Agent`, future
- `RawProgram`, future/reserved

Expected current combinations:

```text
Normal / Plain:
  InputMode::Shell
  ViewKind::Plain

Block View:
  InputMode::BlockNav
  ViewKind::Blocks

Detail View:
  InputMode::DetailNav
  ViewKind::Detail

Help overlay:
  InputMode::BlockNav / DetailNav
  ViewKind::Help

Full-screen programs in Normal mode:
  InputMode::Shell
  ViewKind::Plain
```

Future combinations may add agent views or explicit interactive metadata, but those should not distort the current Block Layer model.

## Full-Screen Program Compatibility

Some commands are full-screen or interactive terminal programs. They are not ordinary linear output and should not be parsed into shell text in the first phase.

Examples:

- `vim`
- `nvim`
- `vi`
- `yazi`
- `fzf`
- `less`
- `more`
- `top`
- `htop`
- `btop`
- `ssh`
- `lazygit`
- `lazydocker`
- `man`
- `tig`

These programs require direct access to keyboard input, cursor movement, alternate screen handling, raw mode behavior, and local redraws. Tide preserves that by making Normal mode transparent. No command-name whitelist is required for passthrough.

### Startup

On the zsh block-start marker:

1. Create an `ExecutionBlock`.
2. Record command, cwd, start time, and start line.
3. Keep Normal mode in transparent passthrough.

### Runtime

While any command is active in Normal mode:

- all ordinary key input goes directly to the PTY
- PTY output is written directly to the real terminal
- Tide may capture best-effort plain text on the side
- if alternate-screen control is observed, Tide may pause sidecar text capture until `block_end`
- the compositor is not run for Normal display
- the renderer is not run for Normal display

### Exit

On the zsh block-end marker:

1. Finish the active `ExecutionBlock`.
2. Record exit code, cwd, duration, and status.
3. Keep the block available for Block View.

The block-end marker is the primary boundary. Do not rely on prompt regexes.

## Current Stage Scope

Current implementation is the minimal Block Layer loop with the following in place:

- Starting real zsh in a PTY with `TIDE=1` session environment
- Parsing zsh lifecycle markers (`preexec` → block start, `precmd` → block end)
- Storing visible output in `ShellBuffer`
- Creating one `ExecutionBlock` per simple command with command, cwd, status, exit code, duration, and line range
- Preserving transparent Normal mode; full-screen programs work without a whitelist
- Rendering Block View with metadata borders (id, command, status, exit code, duration)
- Controlling visible block history through `BlockViewport` (`selected_index`, `line_offset`, `anchor`), separate from `BlockStore` retention (`max_blocks`)
- Truncating collapsed blocks with `preview_lines` and expanded blocks with `expanded_lines`
- Rendering Detail View as a full-screen pager with styled output, line cursor, visual selection, and metadata footer
- Rendering Detail View by inserting detail lines inside the selected block before the bottom border (inline mode)
- Navigation: `j`/`k` (accumulated and flushed at frame cadence), `g`, `G`, `Enter`, `q`, `Esc`, Up/Down arrows
- Copy operations: `c` (copy command), `o` (copy output), `y` (copy both) — copies to clipboard with flash message
- Visual mode: `v` to select a range of blocks (anchor → cursor), actions apply to range
- Delete: `d` with confirmation dialog (Y/N), single or multi-block
- Rerun: `r` pastes command into shell after alt-screen cleanup, with confirmation for multi-block
- Search: `/` opens search bar with live filter-as-you-type, `n`/`N` cycles next/prev results, Esc restores pre-search filter
- Scroll: `Ctrl-u` / `Ctrl-d` (half screen), `Ctrl-b` / `Ctrl-f` (full screen)
- Help overlay: `?` opens a centered keybinding reference dialog with scroll and cursor
- Confirm dialog: centered overlay for destructive operations (delete, rerun multi)
- Viewport anchoring: `Tail` (follow newest), `Top` (oldest), `Manual` (preserve position)
- Force render on view mode switches to prevent stale screen artifacts
- Delta accumulation clamping to prevent unbounded growth
- Config-gated `auto_follow_on_reach_bottom` for controlling `j`→Tail anchor behavior
- Block store retention limits, alternate-screen detection for capture suspension
- Frame-rate-limited rendering (16ms FRAME_DURATION)
- Output truncation (`max_output_bytes_per_block`, 1MB) with `output_truncated` flag in bottom border
- ANSI/VT styling capture via `ansi.rs` with styled text rendering for Block body and Detail View
- Configurable keymap overrides for Block View and Detail View actions
- Copy format support: plaintext, markdown, shell transcript, JSON — with multi-block serialization
- Visual range for Detail View output lines (`V`/`v` toggle anchor → cursor)
- Footer bar with flash messages, search buffer display, filter tags, and keybinding hint
- Styled top border with command, cwd, status marker, and search match highlighting
- **Alternate screen for Block/Detail views**: entering Block View (Ctrl-B) enters the alternate screen buffer via `EnterAlternateScreen`; leaving (q/Esc) leaves it. Block/Detail rendering never touches the main terminal display. On exit, SGR and cursor are reset on the restored main screen, and zsh integration provides an optional `^X^R` debug binding for manual `zle reset-prompt` if needed.

Next-stage items (not yet implemented):

- Return Panel
- TUI app return-panel rendering
- Persistence
- AI-assisted explanations

## Future Extensions

After the Block Layer loop is stable, Tide can grow in these directions:

- robust ANSI/VT handling
- optional interactive block metadata
- TUI handoff-return sessions
- return panels
- optional persistence
- optional AI-assisted block explanation and fix suggestions
- optional natural-language command composition

AI-generated commands must be inserted into the shell prompt by default, not auto-executed.
