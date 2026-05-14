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

### next planned work

- Next implementation should focus on zsh hook installation stability before UI polish.
- Replace the current prototype behavior that writes hook setup into the live PTY input stream.
- Preferred next direction:
  - generate a per-process temporary Tide hook file
  - start zsh so it sources the hook before user interaction
  - preserve the user's normal zsh configuration
  - avoid visible hook setup text and avoid polluting shell history
  - clean up temporary hook files when possible
- Keep Block Mode read-only and latest-10 in-memory until lifecycle capture is stable.
- Do not add block actions, database persistence, AI, ReturnPanel, or UI polish in the next step.

### hook installation via ZDOTDIR temp file

- Replaced PTY-stream hook injection with a ZDOTDIR-based temp file approach.
- Added `TempHookFiles` struct in `shell_hooks.rs`:
  - Creates a per-process temp directory (`/tmp/tide-<pid>/`)
  - Writes `tide-hooks.zsh` (the hook script with hex-encoded OSC 777 events)
  - Writes `.zshenv` that restores ZDOTDIR and sources the original `.zshenv`
  - Writes `.zshrc` that restores ZDOTDIR, sources the original `.zshrc`, and sources `tide-hooks.zsh`
  - Cleans up the temp directory on `Drop` (RAII)
- Modified `pty.rs::run_shell()`:
  - Creates `TempHookFiles` before spawning zsh
  - Sets `ZDOTDIR` env var on the `CommandBuilder` to point to the temp directory
  - Removed the old `hook_install_command()` function and PTY write at startup
- Benefits:
  - No visible hook script text during startup
  - No shell history pollution from hook installation
  - User's normal zsh configuration is preserved (chained via the temp `.zshrc`/`.zshenv`)
  - Clean teardown on Tide exit
- Added tests: temp file creation, .zshrc/.zshenv content, hook file content, Drop cleanup, single-quote escaping

### block frame decorations

- Added inline block frames around command output in the shell stream:
  - On `Preexec`: writes header `┌─ #N · command ────┐` to screen
  - On `Precmd`: writes footer `└─ #N · status · exit N · X.Xs ────┘` to screen
  - Frames adapt to terminal width, command names truncated to fit
- Improved Block Mode UI:
  - j/k navigation with reverse-video highlight on selected block
  - Read-only — no action keys

### terminal grid rendering

- Added `vt100` and `unicode-width` dependencies (switched from `alacritty_terminal` due to macOS compile issue)
- Created `src/renderer.rs` — `TermRenderer` wrapping `alacritty_terminal::Term`:
  - Maintains terminal grid with 10000-line scrollback
  - Diff-based rendering: only writes changed cells to screen via cursor positioning
  - `process(bytes)` feeds PTY output to terminal parser
  - `render(writer)` outputs grid changes
  - `resize(rows, cols)` handles terminal resize
  - `mark_dirty()` forces full redraw
- Rewired `pty.rs` output thread:
  - Replaced transparent stdout writes with `renderer.process()` + `renderer.render()`
  - Block header/footer decorations now fed as bytes to renderer (become grid cells)
  - PTY visible output flowing through renderer rather than directly to stdout
- Updated AGENTS.md, README.md, CLAUDE.md to reflect rendering architecture
- Design: Tide renders shell output (tmux-style), TUI apps will get transparent passthrough

### pty.rs code-level refactor

- Executed a major code-level refactoring on `src/pty.rs` to address structural complexity and god-object anti-patterns.
- **Module Extraction:** Extracted tmux integration logic into `src/tmux.rs` and agent logic into `src/agent_logic.rs`. This reduces the scope of the main PTY loop, making future development and bug tracking significantly easier.
- **Lock Ordering Standardization:** Investigated the lock ordering of `RuntimeState` and `stdout`. Determined that the codebase correctly follows a `stdout` -> `state` lock order (or uses lock-and-drop sequential patterns). Added explicit module-level documentation to `pty.rs` formalizing this rule to prevent future deadlocks.
- **Cleanup:** Resolved 20+ unused variable/mutability warnings across `compositor.rs` and `pty.rs` via `cargo fix`.
