# Tide

Tide is a zsh-based layered shell wrapper.

It runs inside the user's existing terminal, starts real `zsh`, transparently shows Normal mode output, captures shell executions into structured blocks, and redraws that captured history in Block and Detail views:

- **Plain** — transparent zsh passthrough with sidecar capture
- **Blocks** — overlays structured command metadata on the same shell history
- **Detail** — expands the selected block inline with execution details

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
- **Block View** — navigable overlay of shell history with block metadata (borders, id, command, status, exit code, duration)
- **Detail View** — inline expansion of the selected block's execution details
- **Input batching** — repeated `j`/`k` navigation input is accumulated and flushed at frame cadence (16ms)
- **Viewport anchoring** — `Tail` (follow newest), `Top` (oldest), or `Manual` (preserve position)
- **Force render on switch** — view changes always trigger an immediate full redraw

## Requirements

- Rust toolchain
- zsh (or your configured shell)

## Quick Start

```sh
cargo run
```

After Tide starts, run normal commands. Press `Ctrl-B` to enter Block View.

## Navigation

| Key | View | Action |
|-----|------|--------|
| `Ctrl-B` | Plain | Enter Block View |
| `j` / `Down` | Blocks | Next block |
| `k` / `Up` | Blocks | Previous block |
| `g` | Blocks | Jump to oldest block |
| `G` | Blocks | Jump to newest block, re-enter Tail anchor |
| `Enter` | Blocks | Enter Detail View for selected block |
| `q` / `Esc` | Detail | Return to Block View |
| `q` / `Esc` | Blocks | Return to Plain View |

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
