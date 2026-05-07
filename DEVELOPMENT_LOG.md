# Development Log

This log records notable project changes so future agents can quickly understand what changed and why.

## 2026-05-07

### init

- Created the initial repository documentation.
- Added `AGENTS.md` with Tide's product direction, architecture, milestones, and engineering rules.
- Added `CLAUDE.md` as a Claude-specific entry point that delegates to `AGENTS.md`.
- Rewrote `readme.md` with the project positioning:
  - zsh-native shell workspace
  - command blocks
  - TUI handoff-return
- Clarified that Tide has two parallel product lines:
  - `command -> block -> select -> interact`
  - `configured TUI command -> handoff -> exit -> return context`
- Recorded the early UI rule: blocks should use simple line borders first.

### current working direction

- Begin with Milestone 1.
- Build a Rust binary named `tide`.
- Implement a stable transparent zsh PTY wrapper.
- Defer AI, animation, ReturnPanel, and full BlockInteraction UI.

### milestone 1 bootstrap

- Initialized the Rust binary project with Cargo.
- Added the first dependency set for config loading, tracing, PTY handling, terminal control, future TUI rendering, ANSI stripping, and clipboard support.
- Added a default-aware `Config` loader that reads `config/tide.toml` when present and otherwise uses conservative defaults.
- Added initial state and session model types:
  - `AppMode`
  - `AppEvent`
  - `CommandBlock`
  - `TuiSession`
  - block actions and support structs
- Added a transparent zsh PTY runner:
  - enables raw terminal mode
  - starts real `zsh`
  - forwards stdin to the PTY
  - forwards PTY output to stdout
  - applies initial terminal size
  - handles `SIGWINCH` resize events
  - restores terminal raw mode on exit
- Added `config/tide.toml.example` for the intended configuration shape.
