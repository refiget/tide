# CLAUDE.md

This file gives Claude Code / Claude agents repository-specific working instructions.

## Read First

Before editing code, read these documents:

- [docs/architecture.md](docs/architecture.md)
- [docs/block-layer.md](docs/block-layer.md)
- [docs/internal-api.md](docs/internal-api.md)
- [AGENTS.md](AGENTS.md)

These documents define the target architecture. Keep changes consistent with them.

## Commands

| Action | Command |
| --- | --- |
| Build | `cargo build` |
| Type-check | `cargo check` |
| Run tests | `cargo test` |
| Format check | `cargo fmt --check` |
| Format fix | `cargo fmt` |
| Run Tide | `cargo run` |
| Run with block debug | `TIDE_DEBUG_BLOCKS=1 cargo run` |

Before committing terminal behavior changes, run:

```sh
cargo fmt --check
cargo check
cargo test
```

## Current Development Stage

Current target: implement transparent Normal mode plus Block/Detail redraw from captured sidecar state.

The required user-visible loop is:

- run `tide`
- Tide starts real `zsh`
- simple commands like `ls`, `pwd`, `echo hello`, and `false` run normally
- command lifecycle markers create `ExecutionBlock` entries
- Plain / Normal mode passthrough shows real zsh output directly
- Block View overlays block metadata on the same shell history
- Detail View expands the selected block inline
- full-screen interactive commands work naturally because Normal mode is transparent

Required current view support:

- `ViewKind::Plain`
- `ViewKind::Blocks`
- `ViewKind::Detail`
- `ViewKind::RawProgram` is reserved for future metadata only; current passthrough does not depend on it

Required current input support:

- Plain View: ordinary key input goes to zsh
- `Ctrl-B`: enter Block View
- Block View: `j` / `k` or Up / Down moves selected block
- Block View: `g` jumps to the oldest block
- Block View: `G` jumps to the newest block and resumes follow-tail
- Block View: `Enter` enters Detail View
- Detail View: `q` / `Esc` returns to Block View
- Block View: `q` / `Esc` returns to Plain View
- In Normal mode, all ordinary input goes directly to the PTY. Tide only intercepts the Block View shortcut.

## Claude Working Rules

- Read existing modules before adding new structures.
- Check whether a responsibility already exists before creating a similarly named type.
- Keep module boundaries clear.
- Do not put PTY, parser, renderer, block store, and input handling into one giant file.
- Prefer small, coherent changes.
- After each implemented loop, update the relevant docs.
- If existing code conflicts with the target architecture, do a modest refactor instead of hard-patching features into the wrong layer.
- Do not introduce OpenCode, AI, database persistence, complex theme configuration, or a full natural-language mode unless the docs explicitly call for it.
- Do not turn Block View into an alternate-screen list page.
- Do not use popups for block detail.
- Do not couple `BlockStore` retention to the number of blocks visible on screen.
- Do not write block borders or detail lines into `ShellBuffer`.
- Do not store selected or expanded state in `ExecutionBlock`.
- Do not require a command-name whitelist for `vim`, `yazi`, `fzf`, `less`, `top`, `ssh`, or similar programs to work.
- Do not try to emulate or replay alternate-screen program internals.
- If a command has no captured linear text, render a placeholder in Block View instead of trying to classify or parse the program.

## Target Data Flow

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

Key implication: Normal mode is transparent, but Block View must not read real terminal scrollback. It renders only from Tide's captured `ShellBuffer` and `BlockStore`.

## Target Responsibilities

- PTY/session layer: starts zsh and moves bytes.
- Shell integration layer: installs zsh hooks and emits/parses markers.
- Capture layer: turns visible output and markers into buffer/store mutations.
- Buffer layer: stores shell text lines only.
- Block layer: stores structured command execution data only.
- App state: stores input mode, view kind, selection, expansion, and block viewport state.
- Compositor: produces `VisualLine` from `ShellBuffer + BlockStore + ViewState`.
- Renderer: draws `VisualLine` to the real terminal.
- Input/keymap: maps key events to app commands.

`BlockStore.max_blocks` is retention. `BlockViewport` is view position and anchor (`Top`, `Tail`, `Manual`). `BlockViewConfig.preview_lines` and `expanded_lines` control output truncation.

## Full-Screen Program Compatibility

Do not add a first-phase RawProgram whitelist. Normal mode is passthrough, so full-screen and interactive commands naturally receive terminal input and output without Tide-specific detection.

Examples:

- `vim`, `nvim`, `vi`
- `yazi`
- `fzf`
- `less`, `more`, `man`
- `top`, `htop`, `btop`
- `ssh`
- `lazygit`, `lazydocker`, `tig`

The only command lifecycle source is zsh integration:

1. `block_start` creates an `ExecutionBlock`.
2. Visible output is forwarded to the terminal and captured as best-effort plain text.
3. `block_end` finishes the `ExecutionBlock`.
4. If no linear text was captured for the block, Block View renders `no captured text output`.

Do not try to store full-screen program screen state in `ShellBuffer`.

## What Not To Build Now

- OpenCode integration
- AI explanations or fix generation
- Natural-language command mode
- ReturnPanel
- TUI handoff-return
- Database or JSONL persistence
- Complex styling/theme systems
- Complete ANSI/VT terminal emulation
- Capturing full-screen program internals as ShellLine data

The current implementation may use simplified ANSI handling as long as the architecture keeps shell text, block data, visual composition, and rendering separate.
