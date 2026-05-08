# AGENTS.md

## Project Identity

Tide is a Rust project. The binary command is `tide`.

Tide is a multi-mode shell wrapper / terminal application that runs on top of real `zsh`.

It is not a terminal emulator and not a replacement for the user's terminal or shell. Tide starts and wraps real `zsh` inside the user's existing terminal, then adds a controlled input layer and a layered rendering system above that shell.

Simple mental model:

```text
Tide gives zsh a layer system.

Plain View  -> looks like ordinary zsh
Block View  -> overlays structured block metadata on the same shell history
Detail View -> expands the selected block inline
```

Chinese positioning:

Tide 是一个运行在 zsh 之上的多模式 shell wrapper / terminal application。它不是 terminal emulator，也不是替代系统终端，而是在现有终端中启动并包裹 zsh，为 zsh 增加一层可控输入层和渲染层。

## Current Priority

The current phase is the minimal Block Layer loop. Normal mode is transparent passthrough; Block and Detail modes are reconstructed from captured sidecar state:

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

Do not start with OpenCode, AI, persistence, a full natural-language mode, or a complete multi-mode product. The first goal is to make Plain / Blocks / Detail work from the same captured shell history while keeping Normal mode indistinguishable from ordinary zsh.

## Product Boundaries

Tide is:

- a zsh PTY wrapper
- a command lifecycle capture layer
- a shell text buffer
- a structured command block store
- a compositor that turns shell text plus block state into visual lines
- a renderer that draws those visual lines to the real terminal

Tide is not:

- a terminal emulator
- a replacement for zsh
- a replacement for the system terminal
- a terminal scrollback scraper
- a standalone block list application
- a popup UI around shell output
- an AI-first product

## Hard Rules

- Do not implement Tide as a terminal emulator.
- Do not read real terminal scrollback. Normal mode may passthrough visible PTY bytes, but Tide must capture its own sidecar history before Block View is entered.
- Do not make Block View an independent list page.
- Do not make Block UI a popup or modal.
- Do not depend on zsh prompt regexes to detect command boundaries.
- Do not require the user to switch to a fixed zsh theme.
- Do not put `selected`, `expanded`, or current view state into block data.
- Do not write block borders, metadata, or detail text into `ShellBuffer`.
- Do not depend on a RawProgram whitelist to preserve `vim`, `yazi`, `fzf`, `less`, or similar behavior. Normal mode is transparent, so these programs naturally own the terminal while they run.
- Do not try to capture or replay full-screen program internals in the first phase. If Tide has no linear captured text for a command, Block View should show a placeholder body line.
- Do not make the first implementation depend on OpenCode, AI, or a database.

## Layer Ownership

`ShellBuffer` owns only shell text:

- original shell text lines
- optional association from a shell line to a `BlockId`
- no block borders
- no block detail text
- no selected or expanded state

`BlockStore` owns only structured block data:

- command
- cwd
- stdout / stderr, or merged output in early versions
- exit code
- duration
- status
- start / end line information

`ViewState` owns display state:

- current `ViewKind`
- selected block
- expanded block
- scroll offset
- block viewport state, including selected index, viewport offset, and follow-tail

`Compositor` owns visual composition:

- reads `ShellBuffer`
- reads `BlockStore`
- reads `ViewState`
- emits `VisualLine`
- inserts block top/bottom metadata lines
- inserts detail lines for the expanded block

`Renderer` owns terminal drawing:

- takes `VisualLine`
- writes to the real terminal
- does not mutate block data
- does not parse command lifecycle

## Views

### Plain View

Plain View / Normal mode is transparent passthrough.

- Visible PTY bytes are written to the real terminal after Tide strips its own invisible markers
- Tide captures shell text and block lifecycle data on the side
- No block borders
- No block metadata
- No block detail lines
- No top or bottom spacer lines
- User experience should feel like ordinary zsh, including full-screen TUI programs

### Block View

Block View overlays Block Metadata Layer on the same shell history.

- Every command execution maps to one `ExecutionBlock`
- `BlockStore` history retention is separate from viewport visibility
- `BlockViewport` controls which blocks are currently visible and whether the view is anchored to Top, Tail, or Manual
- Collapsed blocks show only a configured output preview
- Top and bottom metadata lines are inserted around that block's output range
- Metadata shows block id, command, status, exit code, and duration
- The selected block is highlighted
- `j` / `k` or Up / Down moves selection
- `g` jumps to the oldest block
- `G` jumps to the newest block and resumes follow-tail
- `Enter` opens Detail View
- `q` / `Esc` returns to Plain View
- repeated `j` / `k` input should be coalesced and rendered at frame cadence

Block View is not a list page and not a popup. It is a new rendering layer over the same shell history.

### Detail View

Detail View expands the selected block inline.

- The selected block remains in shell history context
- Detail lines are inserted inside that block before the bottom border
- Detail shows command, cwd, exit code, duration, stdout, stderr, and actions
- `q` / `Esc` returns to Block View

### Full-Screen Programs

There is no first-phase RawProgram whitelist controlling terminal passthrough.

Normal mode already forwards input and visible PTY output directly, so commands such as `vim`, `nvim`, `yazi`, `fzf`, `less`, `top`, `ssh`, and `lazygit` do not need special handling to remain usable.

Tide still records their command lifecycle through zsh markers. If no linear text output is captured for such a command, Block View renders a placeholder body line such as:

```text
no captured text output
```

Future versions may add metadata that labels a block as interactive, but that metadata must not decide whether terminal passthrough is allowed.

## Command Boundaries

Do not infer command boundaries from prompt text.

Use shell integration markers emitted by zsh hooks, such as:

- `preexec` for block start
- `precmd` for block end
- `chpwd` for cwd changes

Markers should be invisible to the user and stripped from visible shell output by the capture/parser layer.

`preexec` starts the block and `precmd` ends the block. Do not rely on prompt regexes, alternate-screen detection, or command-name whitelists as the primary lifecycle boundary.

## Recommended Module Boundaries

The current repository may not yet match this layout. Move toward it gradually when code changes are needed.

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

## Engineering Rules

- Keep `main.rs` thin.
- Keep PTY, parser, buffer, block store, compositor, renderer, and input mapping separate.
- Prefer Rust `enum` plus `match` for state machines.
- Avoid premature traits or generic abstractions.
- Store block output as raw bytes first when possible; derive display text separately.
- Keep the first in-memory block store small.
- Keep Block Layer read-only until capture and rendering are stable.
- Update `docs/architecture.md`, `docs/block-layer.md`, `docs/internal-api.md`, and `docs/manual-testing.md` when terminal behavior or block architecture changes.
- Before committing terminal behavior changes, run:

```sh
cargo fmt --check
cargo check
cargo test
```

## Read First

Before changing code, read:

- [docs/architecture.md](docs/architecture.md)
- [docs/block-layer.md](docs/block-layer.md)
- [docs/internal-api.md](docs/internal-api.md)
- [docs/manual-testing.md](docs/manual-testing.md)
