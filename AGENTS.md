# AGENTS.md

## Project Identity

Tide wraps real `zsh` in a PTY, captures shell output + lifecycle markers, and renders switchable display layers:

- **Plain View** — transparent zsh passthrough
- **Block View** — structured block metadata overlaid on shell history
- **Detail View** — full-screen pager for one block (entered via `i`, NOT Enter)

Tide is **not** a terminal emulator, zsh replacement, scrollback scraper, or AI-first product.

## Commands

| Action | Command |
|--------|---------|
| Build | `cargo build` |
| Run | `cargo run` |
| Type-check | `cargo check` |
| Test all (~145 across 9 modules) | `cargo test` |
| Test module | `cargo test -- compositor` |
| Single test | `cargo test tail_offset_is_zero` |
| Format check | `cargo fmt --check` |
| Debug block capture | `TIDE_DEBUG_BLOCKS=1 cargo run` |

Pre-commit (terminal behavior changes):
```sh
cargo fmt --check && cargo check && cargo test
```

## Architecture (flat `src/`)

All 12 modules declared in `main.rs` — no `mod.rs`/`lib.rs`. `app.rs` and `config.rs` open with `#![allow(dead_code)]` (forward-looking types).

| Module | Role |
|--------|------|
| `main.rs` | Entry: `Config::load()` → `pty::run_shell()` |
| `app.rs` | `ViewKind`, `ViewState`, `CommandBlock`, `BlockViewport`, ... |
| `pty.rs` | 3-thread runtime (output/input/resize), frame-limited render loop, keyboard dispatch |
| `block.rs` | `BlockStore` — `Vec<BlockId>` timeline + `HashMap` lookup, retention cap |
| `buffer.rs` | `ShellBuffer` — text storage, ANSI escape handling |
| `compositor.rs` | `ShellBuffer + BlockStore + ViewState` → `VisualLayout`; viewport math, Detail pager |
| `renderer.rs` | Crossterm drawing — borders, styled spans, Help overlay, `BlockSelectionStyle` |
| `config.rs` | TOML config loading, `BlockViewConfig`, `BlockLayoutConfig`, `KeymapConfig`, keymap resolution |
| `format.rs` | `compact_command()`, `compact_cwd()`, `build_top_label()`, `CopyFormat`/`CopyPart`/`format_blocks()` |
| `index.rs` | Token inverted index for command search (substring, AND) |
| `ansi.rs` | `parse_ansi_lines()` — raw bytes → `StyledText` spans |
| `theme.rs` | Catppuccin Frappe color constants |
| `shell_hooks.rs` | `Osc777Parser` — strips OSC 777 markers, emits `ShellHookEvent` |
| `app.rs` | `BlockViewAction` (22 variants) / `DetailViewAction` (15 variants) — remappable key action enums |

## Hard Rules

- Do not implement Tide as a terminal emulator.
- Do not read real terminal scrollback.
- Do not make Block View an independent list page or popup.
- Do not depend on zsh prompt regexes for command boundaries.
- Do not put view state (`selected`, `expanded`) into `CommandBlock`.
- Do not write block borders/metadata/detail text into `ShellBuffer`.
- Do not use a RawProgram whitelist — Normal mode is transparent passthrough.
- Do not capture full-screen program internals in the first phase.
- Do not depend on OpenCode, AI, or a database in the first phase.

## Key Terminology Distinction

| Concept | Trigger | View | Scope |
|---------|---------|------|-------|
| Block expansion | `Enter` | `ViewKind::Blocks` (inline) | Per-block toggle |
| Detail View | `i` | `ViewKind::Detail` (full-screen pager) | Single block |

**Enter NEVER enters Detail View.** It toggles inline expansion within Block View.

## Key Design Decisions

- **Normal mode is transparent passthrough** — full-screen TUI programs (vim, fzf, less, ssh, etc.) work without a whitelist. If no linear output is captured, Block View shows `"no captured text output"`.
- **Command boundaries from zsh hooks** (`preexec`/`precmd` via OSC 777), not prompt regexes. Integration script at `shell/zsh-integration.zsh` — user sources from `.zshrc`, not injected by Tide. Without it, Tide runs in degraded mode (no block capture).
- **Three-thread runtime** — output thread (PTY → capture + render), input thread (stdin → dispatch), resize thread (SIGWINCH). Shared state: `Arc<Mutex<RuntimeState>>`. Lock ordering: output locks `state → stdout`; input drops state before locking stdout (avoids deadlock on Ctrl-B).
- **Alternate screen** — Block/Detail rendering uses alt screen buffer, isolated from main terminal. `Ctrl-B` enters (input drops state lock → locks stdout → alt screen → re-acquires state). `q`/`Esc` sets `needs_cleanup` flag (separate from `dirty`/`force_render` to avoid race between output thread writes and alt-screen cleanup).
- **Frame-limited rendering** — 16ms `FRAME_DURATION` in `pty.rs`. Force render on view switches. `j`/`k` deltas accumulated via `InputAccumulator`, flushed at frame cadence.
- **Block viewport** — scrolls by visual line (`line_offset`), selection moves by block. Anchors: `Top`, `Tail`, `Manual`.
- **Default block preview** — `preview_lines` (4) of output, no metadata. **Expanded** — all output lines (capped at `expanded_lines` = 15) + detail lines (command, cwd, exit, duration, actions).
- **Block selection style** — `BlockSelectionStyle` in `renderer.rs` centralises appearance for all 5 render functions; edit `::selected()`/`::normal()`. Selected borders: LAVENDER, round (╭╮╰╯). Normal: SURFACE2.
- **BlockIndex** indexes command text only (substring token match, AND semantics), not output text.
- **Output truncation** — `max_output_bytes_per_block` (1MB); `output_truncated` flag surfaces as `"· truncated"` in bottom border label.
- **Config search order** — `config/tide.toml` (local override) → `$XDG_CONFIG_HOME/tide/config.toml` → `~/.config/tide/config.toml` → `Config::default()`.
- **`tui_apps` / `raw_programs`** are config fields defined but not yet wired into runtime behavior.

## What Not To Build Now

- OpenCode/AI integration, natural-language command mode
- Database/JSONL persistence
- Regex/glob search (current: substring token match only)
- Case-sensitive search, search history in search bar, indexing block output text
- Full-screen program internals capture as `ShellLine` data
- TUI handoff-return, ReturnPanel

## Read Before Changing Code

- [docs/architecture.md](docs/architecture.md) — data flow, non-goals, module descriptions
- [docs/block-layer.md](docs/block-layer.md) — block data model, visual generation rules
- [docs/internal-api.md](docs/internal-api.md) — every struct, method, ownership rules
- [docs/zsh-integration.md](docs/zsh-integration.md) — marker protocol, user setup
- [docs/config.md](docs/config.md) — user-facing config reference
- [docs/manual-testing.md](docs/manual-testing.md) — interactive test checklist
