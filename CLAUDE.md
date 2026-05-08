# CLAUDE.md

This file gives Claude Code / Claude agents repository-specific working instructions.

## Read First

Before editing code, read these documents (they define the target architecture):

- [docs/architecture.md](docs/architecture.md)
- [docs/block-layer.md](docs/block-layer.md)
- [docs/internal-api.md](docs/internal-api.md)
- [AGENTS.md](AGENTS.md)

## Commands

| Action | Command |
|--------|---------|
| Build | `cargo build` |
| Type-check | `cargo check` |
| Run all tests | `cargo test` |
| Run single test | `cargo test keeps_only_latest_blocks` |
| Format check | `cargo fmt --check` |
| Format fix | `cargo fmt` |
| Run Tide | `cargo run` |
| Debug block lifecycle | `TIDE_DEBUG_BLOCKS=1 cargo run` |

Before committing terminal behavior changes:

```sh
cargo fmt --check && cargo check && cargo test
```

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
| `config.rs` | TOML config loading, `BlockViewConfig`, `BlockLayoutConfig`, `RuntimeConfig`; `.default()` for all configs |

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

All state: `Arc<Mutex<RuntimeState>>`.

## Navigation Flow

- `Plain` → `Ctrl-B` → `Blocks` (Tail anchor, force render)
- `Blocks` → `j/k`/Up/Down → accumulated delta, rendered at frame cadence
- `Blocks` → `g` → `Top` anchor (force render)
- `Blocks` → `G` → `Tail` anchor (force render)
- `Blocks` → `Enter` → `Detail` (force render)
- `Detail` → `q`/`Esc` → `Blocks` (force render)
- `Blocks` → `q`/`Esc` → `Plain` (reset to default ViewState, force render)

## What Not To Build Now

- OpenCode, AI explanations/fix generation, natural-language command mode
- ReturnPanel, TUI handoff-return
- Database/JSONL persistence
- Complex styling/theme systems
- Complete ANSI/VT terminal emulation
- Capturing full-screen program internals as ShellLine data
