# Architecture

## Project Positioning

Tide is a multi-mode shell wrapper / terminal application that runs on top of real `zsh`.

It is not a terminal emulator and not a replacement for the user's terminal emulator or shell. Tide starts real `zsh` in a PTY, captures shell output and lifecycle markers, stores that information in internal buffers, and renders switchable layers back into the existing terminal.

The key idea is that Tide gives zsh a layer system:

- Plain / Normal View is ordinary zsh passthrough.
- Block View overlays structured command metadata on the same shell history.
- Detail View expands the selected block inline.

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
- `Compositor` combines shell text, block data, and view state into visual lines for Block / Detail views.
- `Renderer` draws visual lines only when Tide is in a reconstructed view.

## Module Layout

Current source layout (flat `src/` modules):

```text
src/
  main.rs          — thin entry point, loads config, starts PTY session
  app.rs           — ViewState, BlockViewport, ViewAnchor, InputAccumulator,
                     RenderState, AppEvent, CommandBlock, BlockKind, BlockStatus
  pty.rs           — PTY session, marker parser integration, input/output threads,
                     frame-rate-limited render loop, viewport math, navigation
  block.rs         — BlockStore (timeline + HashMap), block lifecycle, duration formatting
  buffer.rs        — ShellBuffer, ShellLine, ANSI escape sequence handling
  compositor.rs    — VisualLine enum, compositor (ShellBuffer + BlockStore + ViewState → VisualLine),
                     visible range computation, tail scroll offset, block height estimation
  renderer.rs      — terminal render function, border drawing, framed text, truncation
  shell_hooks.rs   — Osc777Parser, ShellHookEvent, ParsedPtyPart, zsh install script
  config.rs        — Config loading (TOML), BlockViewConfig, BlockLayoutConfig, RuntimeConfig
```

### app.rs

Owns top-level app state types: `ViewKind`, `InputMode`, `ViewState`, `BlockViewport`, `ViewAnchor`, `InputAccumulator`, `RenderState`, `AppEvent`, `CommandBlock`, `BlockKind`, `BlockStatus`, `BlockAction`.

### pty.rs

Starts and manages the zsh PTY, runs input and output threads, integrates the marker parser, captures shell output into `ShellBuffer` and `BlockStore`, handles keyboard input (including view-mode switching and Block View navigation), and coordinates frame-rate-limited rendering. Viewport math (visible range, tail scroll offset, scroll margin) was recently moved into the `Compositor` for a single source of truth on block height.

### block.rs

Provides `BlockStore` with a `Vec<BlockId>` timeline and `HashMap<BlockId, CommandBlock>` lookup. Controls retention via `max_blocks`. Methods: `start_command`, `append_output`, `finish_command`, `block`, `block_id_at`, `len`.

### buffer.rs

Owns shell text storage via `ShellBuffer`. Supports `append` with ANSI escape sequence handling (cursor movement, erase). Provides `snapshot()` for rendering and `cursor_position()` for Plain View cursor placement. Must not contain block borders, metadata, or detail text.

### compositor.rs

Core of Block/Detail View rendering. `build_visual_lines` produces `Vec<VisualLine>` from `ShellBuffer + BlockStore + ViewState`. Also provides `compute_visible_range`, `compute_tail_scroll_offset`, and `compute_scroll_offset_ending_at` — these are the single source of truth for viewport math, using `build_one_block_lines().len()` instead of estimating height from `output_text`.

### renderer.rs

Provides the `render()` function that draws `&[VisualLine]` to the real terminal via crossterm. Handles border characters (selected: `╭ ╮ ╰ ╯`, unselected: `┌ ┐ └ ┘`), framed text, titled borders, and cursor positioning.

### shell_hooks.rs

Owns zsh hook definitions (`install_script()`), the `Osc777Parser` that strips invisible OSC 777 markers from PTY output, and marker parsing (`parse_block_marker`).

### config.rs

Loads TOML config from `~/.config/tide/config.toml` or `config/tide.toml`. Provides `BlockViewConfig`, `BlockLayoutConfig`, `RuntimeConfig`, and defaults. See [config.md](config.md).

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
- Controlling visible block history through `BlockViewport` (selected_index, scroll_offset, anchor), separate from `BlockStore` retention (`max_blocks`)
- Truncating collapsed blocks with `preview_lines` and expanded blocks with `expanded_lines`
- Rendering Detail View by inserting detail lines inside the selected block before the bottom border
- Navigation: `j`/`k` (accumulated and flushed at frame cadence), `g`, `G`, `Enter`, `q`, `Esc`, Up/Down arrows
- Viewport anchoring: `Tail` (follow newest), `Top` (oldest), `Manual` (preserve position)
- Force render on view mode switches to prevent stale screen artifacts
- Delta accumulation clamping to prevent unbounded growth
- Config-gated `auto_follow_on_reach_bottom` for controlling `j`→Tail anchor behavior
- Block store retention limits, alternate-screen detection for capture suspension
- Frame-rate-limited rendering (16ms FRAME_DURATION)

Next-stage items (not yet implemented):

- Return Panel
- Block actions (copy, rerun, delete)
- Persistence
- AI-assisted explanations

## Future Extensions

After the Block Layer loop is stable, Tide can grow in these directions:

- robust ANSI/VT handling
- optional interactive block metadata
- TUI handoff-return sessions
- return panels
- block actions such as copy, rerun, delete, collapse, and expand
- optional persistence
- optional AI-assisted block explanation and fix suggestions
- optional natural-language command composition

AI-generated commands must be inserted into the shell prompt by default, not auto-executed.
