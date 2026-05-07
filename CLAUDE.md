# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

| Action | Command |
|--------|---------|
| Build | `cargo build` |
| Type-check | `cargo check` |
| Run tests | `cargo test` |
| Format check | `cargo fmt --check` |
| Format fix | `cargo fmt` |
| Run Tide | `cargo run` |
| Run with block debug | `TIDE_DEBUG_BLOCKS=1 cargo run` |

Before committing: all three of `cargo fmt --check`, `cargo check`, and `cargo test` must pass.

## Architecture

Tide is a Rust PTY wrapper around real `zsh` that renders shell output through a terminal grid (tmux-style). The current implementation uses `vt100` for grid parsing and diff-based screen rendering.

```
real terminal ← Tide renders grid here
  |
TermRenderer ← vt100 grid + diff rendering
  |
Osc777Parser ← splits visible output from hook events
  |
PTY master
  |
zsh → child commands / TUIs
```

### Components

**`src/main.rs`** — Thin entry point: init tracing, load config, call `pty::run_shell()`.

**`src/config.rs`** — Loads optional `config/tide.toml`. All fields have `#[serde(default)]`.

**`src/app.rs`** — Pure data model. `AppMode` (6-state enum), `CommandBlock`, `AppEvent`, `BlockAction`, `TuiSession`.

**`src/renderer.rs`** — `TermRenderer`: wraps `vt100::Term`. Maintains terminal grid with 10000-line scrollback. Diff-based rendering — only writes changed cells to screen. `process(bytes)` feeds bytes to the terminal parser, `render(writer)` outputs grid changes.

**`src/pty.rs`** — Core shell runner. Three `std::thread` threads:
- **Output thread**: PTY → `Osc777Parser` → `renderer.process(visible)` + `BlockStore.append_output()` + block decorations → `renderer.render(stdout)`
- **Input thread**: stdin → forwards to PTY writer, intercepts `Ctrl-X Ctrl-B` for block mode
- **Resize thread**: SIGWINCH → PTY resize

State shared via `Arc<Mutex<BlockStore>>`. `TermRenderer` lives exclusively in the output thread.

**`src/shell_hooks.rs`** — `Osc777Parser`: streaming parser for OSC 777 escape sequences. `TempHookFiles`: creates per-process ZDOTDIR temp dir with hook scripts. zsh sources hooks at startup via ZDOTDIR env var.

**`src/block.rs`** — `BlockStore`: `VecDeque<CommandBlock>` ring buffer (max 10 blocks, 1 MiB output cap per block).

**`src/ui.rs`** — `run_block_mode()`: crossterm alternate screen, block list with borders, j/k selection with reverse-video highlight, Esc/q to exit. Read-only.

### Data flow

```
stdin → input thread → PTY master → zsh
zsh output → PTY master → output thread → Osc777Parser
  ├─ Visible → renderer.process() + BlockStore.append_output()
  ├─ Preexec → header bytes → renderer.process()
  └─ Precmd  → footer bytes → renderer.process()
                      ↓
            renderer.render(stdout)  ← diff to screen
Ctrl-X Ctrl-B → ui::run_block_mode() → reads BlockStore
```

### Two modes

- **Shell mode**: PTY output parsed through TermRenderer, grid rendered to screen with inline block frames
- **TUI mode** (future): configured TUI apps get transparent passthrough, no parsing

## What NOT to build yet

- AI / LLM features
- Animation or decorative styling
- ReturnPanel
- Block actions (copy, rerun, explain, save)
- TUI handoff detection
- Database or file persistence

Block mode should remain **read-only** for now.

## Key design rules

- Use `vt100` for terminal parsing — do not write a custom ANSI/VT parser.
- Tide renders shell output; TUI apps get full transparent passthrough.
- Use `enum` + `match` for the state machine, no premature trait/generic abstraction.
- Keep `main.rs` thin — it delegates, never owns logic.
- Always restore terminal state on exit or error (RAII `TerminalGuard`).
- Never steal control during TuiHandoff — full I/O forwarding, no key interception, no overlay.
- AI commands are insert-only — inserted into zsh prompt, never auto-executed.
- Store block output as raw bytes first, then derive stripped text.

Read [AGENTS.md](./AGENTS.md) for detailed product vision, rendering architecture spec, and milestone planning.

## Active development

Current: migrating from transparent passthrough to `vt100`-based rendering. Next: implement TUI handoff detection so configured apps (nvim, lazygit, etc.) get transparent passthrough while shell output is rendered.
