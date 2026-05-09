# Tide

Tide is a zsh-based layered shell wrapper with structured block history, filter/search, and ANSI-styled rendering.

It runs inside the user's existing terminal, starts real `zsh`, transparently shows Normal mode output, captures shell executions into structured blocks, and redraws that captured history in Block and Detail views:

- **Plain** — transparent zsh passthrough with sidecar capture
- **Blocks** — overlays structured command metadata with filters and search
- **Detail** — full-screen pager for deep inspection of a single block with ANSI-colored output

Tide is not a terminal emulator and not a replacement for zsh. Normal mode is passthrough; Block and Detail modes are reconstructed views based on Tide's own captured `ShellBuffer` and `BlockStore`.

## Architecture

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

## Project Status

The current phase implements the minimal Block Layer loop:

- **Startup** — Tide starts real `zsh`, sets `TIDE=1`, and runs interactively
- **Capture** — zsh lifecycle markers (`preexec`/`precmd`) create and finish `ExecutionBlock` entries, visible output is captured alongside
- **Plain View** — ordinary key input and PTY output pass through transparently; Tide only intercepts `Ctrl-B`
- **Block View** — navigable overlay of shell history with block metadata (borders, id, command, status, exit code, duration), ANSI-styled output, failed-only filter, command text search
- **Detail View** — full-screen pager with line cursor, ANSI-colored output, and semantic metadata colors
- **Input batching** — repeated `j`/`k` navigation input is accumulated and flushed at frame cadence (16ms)
- **Viewport anchoring** — `Tail` (follow newest), `Top` (oldest), or `Manual` (preserve position)
- **Force render on switch** — view changes always trigger an immediate full redraw
- **Theme system** — Catppuccin Frappe colors for borders, selection, cursor, footer, metadata

## Requirements

- Rust toolchain
- zsh (or your configured shell)

## Quick Start

```sh
cargo run
```

After Tide starts, run normal commands. Press `Ctrl-B` to enter Block View.

## Navigation

### Block View

| Key | Action |
|-----|--------|
| `j` / `Down` | Next block |
| `k` / `Up` | Previous block |
| `g` | Jump to oldest block |
| `G` | Jump to newest block, re-enter Tail anchor |
| `Enter` | Toggle inline expansion (show/hide full output + metadata) |
| `i` | Enter Detail View for selected block |
| `f` | Toggle failed-only filter |
| `/` | Open command search bar (substring token match, AND semantics) |
| `y` | Copy output to clipboard |
| `Y` | Copy command to clipboard |
| `r` | Rerun selected command (exits to Plain view) |
| `q` / `Esc` | Return to Plain View |

### Detail View

| Key | Action |
|-----|--------|
| `j` / `Down` | Move cursor down (auto-scrolls) |
| `k` / `Up` | Move cursor up (auto-scrolls) |
| `g` | Jump to output top |
| `G` | Jump to output bottom |
| `yc` | Copy command to clipboard |
| `yo` | Copy output to clipboard |
| `yb` | Copy block (command + output) to clipboard |
| `r` | Rerun command (exits to Plain view) |
| `q` / `Esc` | Return to Block View |

## Configuration

See [docs/config.md](docs/config.md) and `config/tide.toml.example`.

## Documentation

- [Architecture](docs/architecture.md)
- [Block Layer](docs/block-layer.md)
- [Internal API](docs/internal-api.md)
- [Configuration](docs/config.md)
- [Zsh Integration](docs/zsh-integration.md)
- [Full-Screen Program Compatibility](docs/raw-program.md)
- [Manual Testing](docs/manual-testing.md)
- [AGENTS.md](AGENTS.md)
