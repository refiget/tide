# Tide: Project Context & Guidelines

Tide is a Rust-based, zsh-powered layered shell wrapper that provides structured block history, filtering, and ANSI-styled rendering. It operates by wrapping a real `zsh` session in a PTY, capturing lifecycle markers (via OSC 777), and reconstructing shell history into navigable views.

## Project Overview

- **Core Technology:** Rust, Tokio (async runtime), Ratatui (TUI rendering), Crossterm (terminal backend), Portable-PTY.
- **Architecture:** 
    - **Normal Mode:** Transparent passthrough to `zsh` with sidecar capture of visible bytes and invisible markers.
    - **Block View:** A navigable overlay of command history with structured metadata (borders, execution status, duration).
    - **Detail View:** A full-screen pager for deep inspection of individual blocks, supporting ANSI colors and visual selection.
- **Key Concept:** Tide uses a "layer" system. It doesn't replace the terminal or shell but overlays them with metadata and interactive history.

## Building and Running

- **Run:** `cargo run`
- **Build:** `cargo build`
- **Test:** `cargo test` (Tests are located inline within the source modules).
- **Zsh Integration:** Requires the zsh hooks defined in `shell/zsh-integration.zsh` to be active (Tide handles this by injecting them or the user can manual install).

## Module Map

- `src/main.rs`: Entry point; initializes logging and starts the PTY session.
- `src/app.rs`: Manages application state, view modes (`ViewKind`), and input modes (`InputMode`).
- `src/pty.rs`: Orchestrates the PTY, input/output threads, and the main event/render loop.
- `src/block.rs`: Implements `BlockStore` for managing the timeline of command executions.
- `src/buffer.rs`: Handles raw shell text storage and ANSI sequence processing via `ShellBuffer`.
- `src/compositor.rs`: Logic for converting raw buffers and block data into visual lines for rendering.
- `src/renderer.rs`: Terminal-level rendering, including borders, styled text, and overlays (Help/Confirm).
- `src/shell_hooks.rs`: Parser for OSC 777 markers and shell integration scripts.
- `src/ansi.rs`: Utilities for parsing and manipulating ANSI-styled text.
- `src/config.rs`: TOML configuration loading and keymap resolution.
- `src/theme.rs`: Semantic color definitions using the Catppuccin Frappe palette.

## Development Conventions

- **Concurrency:** Uses `tokio` for async tasks. Be mindful of lock ordering (typically `state` -> `stdout`).
- **Error Handling:** Use `anyhow::Result` for general errors and `thiserror` for defined error types.
- **Logging:** Use `tracing` macros (`debug!`, `info!`, etc.). Logs are written to stderr.
- **Testing:** Add unit tests within the same module file using the `#[cfg(test)]` block.
- **Rendering:** All reconstructed views (Block/Detail) render into the **alternate screen** to preserve the main shell's scrollback.
- **Style:** Adhere to standard Rust idioms. Truncation and width calculations must be `unicode-width` aware.
- **Markers:** Tide relies on `preexec` and `precmd` hooks sending OSC 777 markers to identify command boundaries.

## Documentation References

- [Architecture](docs/architecture.md): Deep dive into the design and data flow.
- [Block Layer](docs/block-layer.md): Data model and visual generation rules.
- [Internal API](docs/internal-api.md): Component-level interactions.
- [Zsh Integration](docs/zsh-integration.md): Details on how Tide interacts with the shell.
