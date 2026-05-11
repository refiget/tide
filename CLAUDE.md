# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Working Mode

This project uses **direct implementation** — Claude reads the codebase, makes changes, and runs `cargo check && cargo test` to verify. No intermediate prompt generation step.

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

## Test Locations (~126 tests)

| Module | Count | What's tested |
|--------|-------|---------------|
| `ansi.rs` | 14 | SGR parsing, 256-color, truecolor, OSC/CSI ignoring, multiline, \r/\r\n, truncation |
| `pty.rs` | 25 | View transitions, force-render flags, viewport clamping, boundary navigation, Detail scroll, clipboard copy, live search, CopyFormat |
| `compositor.rs` | 22 | Visual layout, viewport math, anchors (Top/Tail/Manual), span invariants, footer, Detail layout, flash messages |
| `block.rs` | 4 | Retention cap, prev/next navigation, unbounded history, output truncation flag |
| `shell_hooks.rs` | 8 | OSC 777 marker stripping, split-event handling, normal output passthrough, hex decoding |
| `renderer.rs` | 5 | Framed text width with wide/unicode chars, titled border width, search highlight spans |
| `config.rs` | 4 | Runtime config defaults, legacy field handling, CopyFormat deserialization |
| `format.rs` | 43 | compact_command, compact_cwd, build_top_label, CopyFormat (plaintext/markdown/transcript/json, multi-record) |
| `index.rs` | 1 | Token inverted index query (substring + AND semantics) |

## Key Terminology (Critical — Do Not Confuse)

### Block Expansion (Enter in Block View)
- **What it is**: A per-block in-place toggle within Block View. Pressing Enter on a selected block shows/hides full output lines + metadata (command, cwd, exit, duration, actions).
- **View**: Stays in `ViewKind::Blocks`. No view switch.
- **State**: `ViewState.expanded_block: Option<BlockId>` — the block currently expanded, or `None`.
- **Rendering**: `build_one_block_lines()` checks `expanded_block == Some(block_id)` to decide whether to show all output lines (capped at `expanded_lines`, default 15) and append `detail_lines()`.
- **Footer**: Shows Block View footer (`Block #N/total  j/k ...`).
- **Navigation**: `j`/`k` navigate between blocks normally; expanded state follows selection (Enter, `j`/`k`, `g`, `G`).

### Detail View (i from Block View)
- **What it is**: A full-screen pager mode for deep inspection of one block. Entry via `i` from Block View.
- **View**: `ViewKind::Detail` — separate view, leaves Block View.
- **Rendering**: `build_detail_lines()` generates single-block full-screen layout with ANSI-styled output and metadata (command, cwd, exit, duration, actions).
- **Footer**: Shows pager-style footer (`Detail #N  ↑↓ scroll ...`).
- **Navigation**: `j`/`k` scroll within the block's output with a highlighted cursor line; auto-scrolls when cursor leaves visible area.

> **Rule**: Enter NEVER enters Detail View. Enter toggles block expansion in Block View only.

## Notable Code Conventions

- `src/app.rs` and `src/config.rs` open with `#![allow(dead_code)]` — many types are forward-looking / not fully wired yet
- `COMPOSITOR_TIMESTAMP_DURATION_MS` in `compositor.rs` gates a timestamp-display debug path
- `FRAME_DURATION` (16ms) in `pty.rs` controls render cadence
- `CommandBlock.output_truncated` is set when `max_output_bytes_per_block` is hit; surfaces as `"· truncated"` in the bottom border label and as a detail line
- Prefer `enum + match` for state machines; avoid premature traits or generic abstractions
- ANSI output is parsed by `ansi::parse_ansi_lines()` into `StyledText` spans; rendered by `render_styled_framed_text()`
- Navigation functions use `view.visible.ids(blocks)` instead of `blocks.timeline` directly (supports filters)
- `build_detail_lines` in compositor renders Detail View (not `build_detail_layout`)

## Renderer Maintenance Groups

### Group A — Block Selection Style (sync all together)
All block selection visual changes go through `BlockSelectionStyle` in `renderer.rs`. Edit only `BlockSelectionStyle::selected()` and `::normal()` — the 5 render functions consuming it update automatically:

| Function | What it renders |
|----------|----------------|
| `render_top_border` | ╭─ #N cmd ~/path ─╮ |
| `render_border` | ╰──────────────────╯ |
| `render_framed_text` | plain-text body lines │...│ |
| `render_styled_framed_text` | ANSI body lines │...│ |
| `render_block_detail_line` | expanded metadata lines │...│ |

Current style: border color LAVENDER (selected) / SURFACE2 (normal), no body background, always round corners ╭╯.

### Group B — Help Overlay
Changes to Help appearance touch: `render_help_overlay` + `BLOCK_HELP_ENTRIES` / `DETAIL_HELP_ENTRIES` in `renderer.rs`, `ViewKind::Help` handler in `pty.rs`, and `ViewState.help: Option<HelpState>` in `app.rs`.

## Architecture (flat `src/` modules)

| Module | Responsibility |
|--------|---------------|
| `main.rs` | Entry point — loads config, starts PTY session |
| `app.rs` | Types: `BlockId`, `ViewKind`, `InputMode`, `ViewState`, `HelpState`, `BlockViewport`, `ViewAnchor`, `VisibleSource`, `BlockFilter`, `FooterSegment`, `CommandBlock/ExecutionBlock`, `InputAccumulator`, `RenderState`, `BlockKind`, `BlockStatus`, `BlockAction`, `AppEvent` |
| `pty.rs` | PTY session, 3-thread runtime (output reader, input reader, resize handler), `Osc777Parser` integration, frame-limited render loop, keyboard dispatch, navigation, `TerminalGuard` |
| `block.rs` | `BlockStore` — `Vec<BlockId>` timeline + `HashMap<BlockId, CommandBlock>` lookup, retention cap, output byte cap |
| `buffer.rs` | `ShellBuffer` — text storage with ANSI escape handling (CSI cursor/erase, OSC strings, CR, backspace, tab) |
| `compositor.rs` | `Compositor` + `VisualLine` enum (Empty, ShellText, BlockBodyLine, StyledBlockBodyLine, BlockTopBorder, BlockBottomBorder, BlockDetailLine, DetailTopBorder, DetailBottomBorder, StyledDetailBodyLine, Footer) — builds `VisualLayout` from `ShellBuffer + BlockStore + ViewState`; viewport math; Detail View pager |
| `renderer.rs` | Terminal drawing via crossterm — `BlockSelectionStyle` (centralised selection palette), border chars, framed text, styled span rendering, Help overlay, theme-aware colors, footer, cursor, `truncate_to_width` |
| `config.rs` | TOML config loading (local > XDG > legacy > defaults), `BlockViewConfig`, `BlockLayoutConfig`, `RuntimeConfig`; `.default()` for all configs |
| `format.rs` | `compact_command()`, `compact_cwd()`, `build_top_label()` — ANSI stripping, whitespace normalization, unicode-aware truncation, top border label formatting |
| `index.rs` | `BlockIndex` — `failed: Vec<BlockId>` index + `tokens: HashMap<String, Vec<BlockId>>` inverted index for command search |
| `ansi.rs` | `parse_ansi_lines()` — parses raw PTY bytes into `Vec<StyledText>` with per-span `TextStyle` (fg/bg/bold/italic/underline/reverse), handles SGR/OSC/CSI |
| `theme.rs` | Catppuccin Frappe color constants for borders, selection, cursor, footer, metadata labels |
| `shell_hooks.rs` | `Osc777Parser` — strips invisible OSC 777 markers from PTY output, emits `ShellHookEvent::Preexec`/`Precmd`; zsh `preexec`/`precmd` hook install script |

## Key Design Rules

- **ShellBuffer stores only shell text** — no block borders, metadata, detail lines, or selection state
- **BlockStore stores only structured block data** — no view state
- **ViewState owns display state** — selected block, expanded block, viewport, anchor, filter, visible, detail_line_cursor, search_buffer, `help: Option<HelpState>` (non-None while Help overlay is open)
- **Compositor is the single source of truth** for viewport math; visual layout drives height calculations
- **Normal mode is transparent passthrough** — full-screen programs (vim, fzf, less, ssh, etc.) work without a whitelist
- **Command boundaries from zsh hooks** (`preexec`/`precmd`), not prompt regexes
- **Frame-rate-limited rendering** — 16ms FRAME_DURATION, force render on view switches
- **Input batching** — `j`/`k` deltas accumulated via `InputAccumulator`, flushed at frame cadence
- **Block store retention** (`max_blocks`) is separate from viewport visibility
- **Filter/navigation via VisibleSource** — all navigation functions iterate `view.visible.ids(blocks)` instead of `blocks.timeline` directly
- **ANSI rendering from output_raw** — body lines are parsed from `block.output_raw` via `parse_ansi_lines` for color/style preservation

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
- `Blocks` → `Enter` → toggle `expanded_block` (inline expand/collapse, stays in Blocks, force render)
- `Blocks` → `i` → `Detail` (full-screen pager, force render)
- `Blocks` → `f` → toggle failed-only filter (rebuild visible, force render)
- `Blocks` → `/` → open search bar → type query → `Enter` apply, `Esc` cancel
- `Blocks` → `y`/`Y` → copy output/command to clipboard (with flash message)
- `Blocks` → `r` → rerun (exit to Plain, paste command to PTY)
- `Blocks` → `?` → `Help` overlay (underlying Blocks view rendered behind; `j`/`k`/`g`/`G` scroll list, `?`/`q`/`Esc` close)
- `Blocks` → `q`/`Esc` → `Plain` (reset to default ViewState, force render)
- `Detail` → `j`/`k` → move cursor line (auto-scrolls)
- `Detail` → `g`/`G` → jump to top/bottom
- `Detail` → `yc`/`yo`/`yb` → copy command/output/block
- `Detail` → `r` → rerun (exit to Plain, paste command to PTY)
- `Detail` → bare `\x1b` or `q` → `Blocks` (force render); multi-byte escape sequences (arrow keys etc.) are consumed without triggering exit
- `Detail` → `?` → `Help` overlay (underlying Detail view rendered behind; same navigation as Blocks Help)

## Config Search Order

1. `config/tide.toml` (local development override)
2. `$XDG_CONFIG_HOME/tide/config.toml`
3. `$HOME/.config/tide/config.toml`
4. `Config::default()` if none exist

See `config/tide.toml.example` for all available options.

## Config Defaults

- `history.max_blocks = 1000`
- `block_view.preview_lines = 4`
- `block_view.expanded_lines = 15`
- `block_view.follow_tail = true`
- `block_view.block_gap = 0`
- `block_view.scroll_margin_lines = 2`
- `block_view.auto_follow_on_reach_bottom = false`
- `block_view.horizontal_margin = 1`
- `block_view.body_padding = 1`
- `block_view.show_footer = true`
- `block_layout.horizontal_padding = 1`
- `block_layout.show_padding_in_plain = true`

## What Not To Build Now

- OpenCode, AI explanations/fix generation, natural-language command mode
- ReturnPanel, TUI handoff-return
- Database/JSONL persistence
- Regex or glob search (current: substring token match only)
- Search history / up-arrow recall in search bar
- Case-sensitive search mode
- Indexing of block output text (command text only)
- / key in Detail View
- Capturing full-screen program internals as ShellLine data
