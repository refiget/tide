# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Read First

Before editing code, read these documents (they define the target architecture):

- [docs/architecture.md](docs/architecture.md)
- [docs/block-layer.md](docs/block-layer.md)
- [docs/internal-api.md](docs/internal-api.md)
- [docs/zsh-integration.md](docs/zsh-integration.md)
- [docs/config.md](docs/config.md)
- [docs/manual-testing.md](docs/manual-testing.md)
- [AGENTS.md](AGENTS.md)

## Commands

| Action | Command |
|--------|---------|
| Build | `cargo build` |
| Type-check | `cargo check` |
| Run all tests | `cargo test` |
| Run tests in a module | `cargo test -- compositor` |
| Run single test | `cargo test tail_offset_is_zero` |
| Run with stdout | `cargo test -- --nocapture` |
| Format check | `cargo fmt --check` |
| Format fix | `cargo fmt` |
| Run Tide | `cargo run` |
| Debug block lifecycle | `TIDE_DEBUG_BLOCKS=1 cargo run` |

Before committing terminal behavior changes:

```sh
cargo fmt --check && cargo check && cargo test
```

## Test Locations (~30 tests)

| Module | Count | What's tested |
|--------|-------|---------------|
| `pty.rs` | 8 | View transitions, force-render flags, viewport clamping, boundary navigation |
| `compositor.rs` | 16 | Visual layout, viewport math, anchors (Top/Tail/Manual), span invariants, edge cases |
| `block.rs` | 4 | Retention cap, prev/next navigation, unbounded history, output truncation flag |
| `shell_hooks.rs` | 7 | OSC 777 marker stripping, split-event handling, normal output passthrough, hex decoding |
| `renderer.rs` | 2 | Framed text width with wide/unicode chars |
| `config.rs` | 2 | Runtime config defaults, legacy field handling |

## Notable Code Conventions

- `src/app.rs` and `src/config.rs` open with `#![allow(dead_code)]` — many types are forward-looking / not fully wired yet
- `COMPOSITOR_TIMESTAMP_DURATION_MS` in `compositor.rs` gates a timestamp-display debug path
- `FRAME_DURATION` (16ms) in `pty.rs` controls render cadence
- `CommandBlock.output_truncated` is set when `max_output_bytes_per_block` is hit; surfaces as `"· truncated"` in the bottom border label and as a detail line
- Prefer `enum + match` for state machines; avoid premature traits or generic abstractions

## Architecture (flat `src/` modules)

| Module | Responsibility |
|--------|---------------|
| `main.rs` | Entry point — loads config, starts PTY session |
| `app.rs` | Types: `BlockId`, `ViewKind`, `InputMode`, `ViewState`, `BlockViewport`, `ViewAnchor`, `CommandBlock/ExecutionBlock`, `InputAccumulator`, `RenderState`, `BlockKind`, `BlockStatus`, `AppEvent` |
| `pty.rs` | PTY session, 3-thread runtime (output reader, input reader, resize handler), `Osc777Parser` integration, frame-limited render loop, keyboard dispatch, navigation, `TerminalGuard` |
| `block.rs` | `BlockStore` — `Vec<BlockId>` timeline + `HashMap<BlockId, CommandBlock>` lookup, retention cap, output byte cap |
| `buffer.rs` | `ShellBuffer` — text storage with ANSI escape handling (CSI cursor/erase, OSC strings, CR, backspace, tab) |
| `shell_hooks.rs` | `Osc777Parser` — strips invisible OSC 777 markers from PTY output, emits `ShellHookEvent::Preexec`/`Precmd`; zsh `preexec`/`precmd` hook install script |
| `compositor.rs` | `Compositor` + `VisualLine` enum (ShellText, BlockBodyLine, BlockTopBorder, BlockBottomBorder, BlockDetailLine, Footer) — builds `VisualLayout` from `ShellBuffer + BlockStore + ViewState`; viewport math |
| `renderer.rs` | Terminal drawing via crossterm — border chars, framed text, truncation, footer, cursor, `truncate_to_width` |
| `config.rs` | TOML config loading (local > XDG > legacy > defaults), `BlockViewConfig`, `BlockLayoutConfig`, `RuntimeConfig`; `.default()` for all configs |

## Key Design Rules

- **ShellBuffer stores only shell text** — no block borders, metadata, detail lines, or selection state
- **BlockStore stores only structured block data** — no view state
- **ViewState owns display state** — selected block, expanded block, viewport, anchor
- **Compositor is the single source of truth** for viewport math; visual layout drives height calculations
- **Normal mode is transparent passthrough** — full-screen programs (vim, fzf, less, ssh, etc.) work without a whitelist
- **Command boundaries from zsh hooks** (`preexec`/`precmd`), not prompt regexes
- **Frame-rate-limited rendering** — 16ms FRAME_DURATION, force render on view switches
- **Input batching** — `j`/`k` deltas accumulated via `InputAccumulator`, flushed at frame cadence
- **Block store retention** (`max_blocks`) is separate from viewport visibility

## Three-Thread Runtime (pty.rs)

1. **Output thread** — reads PTY master, runs `Osc777Parser`, captures visible output to `ShellBuffer + BlockStore`, writes to real terminal (Plain view) or triggers `render_runtime` (Block/Detail view)
2. **Input thread** — reads stdin, dispatches view-mode keys, forwards remaining bytes to PTY writer, calls `maybe_flush_navigation_and_render`
3. **Resize thread** — listens for `SIGWINCH`, resizes PTY, updates stored dimensions

All state: `Arc<Mutex<RuntimeState>>`. Lock ordering: output thread locks `(state) -> (stdout)`; input thread drops state lock before locking stdout (avoids deadlock on Ctrl-B from Plain mode).

## Alternate Screen Lifecycle

- `Ctrl-B` from Plain: input thread drops state lock, enters alt screen, re-acquires state lock
- `q`/`Esc` from Blocks/Detail: sets `needs_cleanup` flag (not `dirty`/`force_render`) — post-byte-loop handler exits alt screen, resets SGR, shows cursor
- `RenderState.needs_cleanup` is a separate path from `dirty`/`force_render` to avoid races between output thread writes and alt-screen cleanup

## Navigation Flow

- `Plain` → `Ctrl-B` → `Blocks` (Tail anchor, force render)
- `Blocks` → `j`/`k`/Up/Down → accumulated delta, rendered at frame cadence
- `Blocks` → `g` → `Top` anchor (force render)
- `Blocks` → `G` → `Tail` anchor (force render)
- `Blocks` → `Enter` → `Detail` (force render)
- `Detail` → bare `\x1b` or `q` → `Blocks` (force render); multi-byte escape sequences (arrow keys etc.) are consumed without triggering exit
- `Blocks` → `q`/`Esc` → `Plain` (reset to default ViewState, force render)

## Config Search Order

1. `config/tide.toml` (local development override)
2. `$XDG_CONFIG_HOME/tide/config.toml`
3. `$HOME/.config/tide/config.toml`
4. `Config::default()` if none exist

See `config/tide.toml.example` for all available options.

## What Not To Build Now

- OpenCode, AI explanations/fix generation, natural-language command mode
- ReturnPanel, TUI handoff-return
- Database/JSONL persistence
- Complex styling/theme systems
- Complete ANSI/VT terminal emulation
- Capturing full-screen program internals as ShellLine data
