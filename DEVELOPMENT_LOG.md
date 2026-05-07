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

### manual testing policy

- Added `docs/manual-testing.md` as the maintained manual test checklist for terminal behavior.
- Documented the Milestone 1 flow for testing zsh passthrough, command output, interactive input, `Ctrl-C`, `Ctrl-D` / `exit`, resize propagation, TUI passthrough smoke tests, and terminal recovery.
- Updated `AGENTS.md` so future agents must maintain the manual testing checklist whenever terminal behavior changes.
- Rewrote `docs/manual-testing.md` in Chinese to match the project's primary documentation style.

### transparent-first product direction

- Clarified that transparent passthrough is Tide's foundation, not a temporary bootstrap phase.
- Updated the product guidance so block features are sidecar capture and opt-in interaction rather than a default replacement for the live shell surface.
- Reframed future UI work around boundary-aware enhancement:
  - zsh lifecycle boundaries
  - explicit BlockInteraction shortcuts
  - configured TUI handoff-return exits
- Replaced language that implied Tide should naturally grow into a complete block-rendered shell by default.

### block mode prototype

- Began the first block capture prototype with an in-memory `BlockStore`.
- Set the default block history limit to the latest 10 blocks for the current Tide session.
- Added OSC 777 zsh hook parsing for `preexec`, `precmd`, and `chpwd`.
- Added a read-only alternate-screen Block Mode entered with `Ctrl-X Ctrl-B`.
- Kept persistence as optional future work and outside the PTY hot path.

### hook and capture hardening

- Changed OSC 777 hook payloads to `hex:` encoding so commands with semicolons, newlines, or control-sensitive characters do not break event parsing.
- Kept parser support for legacy plain payloads to avoid making the parser brittle.
- Added parser tests for split events, multiple events in one PTY chunk, semicolon/newline payloads, and ordinary output delay behavior.
- Added optional `TIDE_DEBUG_BLOCKS=1` output to inspect completed block status, exit code, duration, command, and captured output size.
- Updated the Chinese manual testing guide with block capture debug and hook/parser regression checks.
