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

## Module Responsibilities

Recommended long-term module layout:

```text
src/
  app/
    state.rs
    runtime.rs
    command.rs

  pty/
    session.rs

  shell_integration/
    zsh.rs
    marker.rs

  capture/
    parser.rs
    command_capture.rs

  buffer/
    shell_buffer.rs

  block/
    block.rs
    store.rs
    layout.rs

  render/
    compositor.rs
    visual_line.rs
    renderer.rs
    styles.rs

  input/
    keymap.rs
```

### app

Owns top-level app state, including input mode, view kind, selected block, expanded block, scroll offset, and runtime coordination.

### pty

Starts and manages the zsh PTY. It should move bytes between real stdin/stdout and the PTY, but should not own block layout or rendering policy.

### shell_integration

Owns zsh hook generation and shell marker definitions. Command lifecycle boundaries should come from markers such as `preexec`, `precmd`, and `chpwd`.

Tide must preserve the user's native zsh configuration. The runtime starts an interactive zsh and sets `TIDE=1` and `TIDE_SESSION_ID`. Users source Tide's integration from their own `.zshrc`; if they do not, Tide runs in degraded mode without command block boundaries.

### capture

Consumes PTY output and shell markers. It updates `ShellBuffer` and `BlockStore`.

### buffer

Owns shell text storage. It must not contain block borders, block metadata lines, detail text, selected state, or expanded state.

### block

Owns structured command execution records and optional layout records.

### render

Owns visual composition and terminal drawing.

The compositor builds `VisualLine` values from `ShellBuffer + BlockStore + ViewState`. The renderer only draws those visual lines.

### input

Maps key events into app commands such as entering Block View, moving selection, expanding detail, returning, or forwarding bytes to zsh.

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

Current implementation work should focus on:

- starting real zsh in a PTY
- parsing zsh lifecycle markers
- storing visible output in `ShellBuffer`
- creating one `ExecutionBlock` per simple command
- recording command, cwd, status, exit code, duration, and line range
- preserving transparent Normal mode without RawProgram detection
- rendering Block View by adding metadata lines around block ranges
- controlling visible block history through `BlockViewport`, separate from `BlockStore` retention
- truncating collapsed blocks with `preview_lines` and expanded blocks with `expanded_lines`
- rendering Detail View by inserting detail lines inside the selected block
- supporting simple navigation keys

Complex terminal behavior may be simplified in this phase if the layer boundaries remain correct.

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
