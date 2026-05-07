# AGENTS.md

## Project Identity

You are building a Rust project named Tide. The binary command is `tide`.

Tide is a zsh-native shell workspace with command blocks and TUI handoff-return.

Chinese positioning:

Tide 是一个基于 zsh 的 shell 工作环境，提供命令结果块和 TUI 应用交接返回机制。

Tide runs inside the user's existing terminal emulator. It is not a new terminal emulator and not a replacement for zsh. It is a PTY wrapper around real `zsh`. The user runs `tide`, Tide starts real `zsh`, and Tide provides two parallel core features above it.

## Two Parallel Product Lines

Tide has two equal core capabilities. Do not treat one as a wrapper around the other.

```text
command -> block -> select -> interact

configured TUI command -> handoff -> exit -> return context
```

They share the same zsh lifecycle, PTY wrapper, state machine, and session model, but they are parallel product values.

## Core Feature 1: Block-Based Execution

Every command execution becomes an independent `CommandBlock`.

A block contains:

- command
- output
- exit code
- duration
- cwd
- timestamp
- git context

Users can select a block and operate on it:

- copy command
- copy output
- rerun command
- explain output
- explain error
- generate fix command
- summarize block
- collapse block
- expand block
- save block
- delete block from the current session view
- create note from block
- inspect git changes after block
- insert suggested command into the zsh prompt

Important rule: AI-generated commands are inserted into the zsh prompt by default. They must not be auto-executed unless the user explicitly presses Enter.

## Core Feature 2: TUI Handoff-Return

Users can configure specific commands as TUI handoff apps, such as `nvim`, `vim`, `lazygit`, `opencode`, `fzf`, and `less`.

When a configured TUI command runs, Tide enters `TuiHandoff` mode. In `TuiHandoff`, Tide gives the application complete control of the terminal:

- do not draw Tide UI
- do not draw overlays
- do not steal shortcuts
- do not parse the TUI app's internal state
- forward all input to the PTY
- forward all output to the real terminal

When the TUI app exits, Tide returns to the shell workspace, creates a `TuiSession` block, and may show a return transition or return panel with:

- app name
- original command
- exit status
- duration
- cwd
- git branch
- changed files
- `git status --short`
- `git diff --stat`
- suggested next commands

If `return_panel = "none"`, return directly to `ShellIdle`.

## Non-Goals

Tide is not:

- a new terminal emulator
- a replacement for zsh
- a `tmux` / `zellij` style multiplexer
- a full IDE
- an AI agent-first product
- a simple AI command launcher

Do not start by implementing a full terminal emulator, complete ANSI/VT parser, complete IDE, or autonomous AI workflow.

## What Tide Is

Tide is:

- a zsh PTY wrapper
- a command lifecycle manager
- a block-based shell workspace
- a command result interaction layer
- a TUI app handoff-return runtime
- later, an optional AI-assisted interaction layer for blocks and return context

## Architecture

```text
real terminal
  |
tide wrapper
  |
PTY master
  |
zsh
  |
child commands / TUI apps
```

Primary responsibilities:

- start real `zsh`
- transparently forward user input to `zsh`
- read `zsh` and child process output
- receive command lifecycle events from zsh hooks, such as `preexec`, `precmd`, and `chpwd`
- model ordinary commands as `CommandBlock`
- model configured TUI handoff sessions as `TuiSession` blocks
- append command output to the active block
- record exit code, duration, cwd, timestamp, and git context when a command finishes
- support selecting and acting on historical blocks
- detect configured TUI handoff apps
- enter `TuiHandoff` when a configured TUI app runs
- enter `Returning` after handoff exits and run snapshot or after-exit jobs
- optionally show return transition or return panel
- later, integrate AI for block interaction and return context summarization

## Rendering Architecture

Tide uses terminal-grid-based rendering, similar to tmux. PTY output is parsed through `alacritty_terminal` to maintain an in-memory terminal grid, which Tide renders to the real terminal. This enables inline block decorations and future full block rendering.

### Two-Mode Design

**Shell Mode (Rendered)**

Default mode. Tide parses PTY output through `alacritty_terminal::Term`, maintains the grid, and renders it to the real terminal. Block header/footer frames are injected inline. Tide owns the display during this mode.

```
PTY output → Osc777Parser → alacritty_terminal::Term → grid → Tide renders → stdout
```

**TUI Mode (Transparent)**

When a command matches `tui_apps` config (nvim, lazygit, etc.), Tide yields the terminal. Raw PTY bytes are forwarded directly — no parsing, no rendering, no interference. Before entering: leave alternate screen. After exit: re-enter Tide rendering.

Mode switch on `ShellPreexec { command }`:
- Match against `tui_apps` commands → TUI mode (transparent)
- No match → Shell mode (rendered)

Return on `ShellPrecmd`.

### Rendering Pipeline

```
PTY output → Osc777Parser (splits visible/event)
  ├─ Visible bytes
  │   ├─ alacritty_terminal::Term.advance_bytes()   maintain grid
  │   ├─ BlockStore.append_output()                  sidecar capture
  │   └─ TermRenderer.render(stdout)                 diff-based screen update
  └─ OSC events
      ├─ Preexec → start block + render header
      ├─ Precmd  → finish block + render footer
      └─ Chpwd   → update cwd
```

### Component: `TermRenderer` (src/renderer.rs)

Wraps `vt100::Parser`:
- `process(&mut self, bytes: &[u8])` — feed bytes to terminal parser
- `render<W: Write>(&self, w: &mut W) -> io::Result<()>` — diff current screen vs last rendered, write only changed cells
- `resize(&mut self, rows: u16, cols: u16)` — handle SIGWINCH
- `mark_dirty()` — force full redraw (after mode switch)
- `total_lines()` — scrollback + visible rows

### Thread Model

Output thread merges rendering into the existing PTY read loop:

```
while running:
    read PTY → parser.push(bytes)
    for part in parsed:
        Visible → renderer.process() + block.append() + renderer.render()
        Event   → apply lifecycle + write decorations
```

No additional threads needed. `vt100::Parser` is single-threaded inside the output thread. The resize thread signals the output thread to call `renderer.resize()`.

### Dependencies

- `vt100` — pure Rust ANSI/VT parser, maintains screen grid with scrollback
- `ratatui` — used in BlockInteraction mode for TUI rendering
- `crossterm` — terminal raw mode, cursor control, alternate screen

## Core Data Model

### CommandBlock

`CommandBlock` represents one ordinary command execution or one session-like block.

Suggested fields:

```rust
struct CommandBlock {
    id: BlockId,
    command: String,
    cwd: PathBuf,
    started_at: DateTime,
    finished_at: Option<DateTime>,
    duration_ms: Option<u64>,
    exit_code: Option<i32>,
    output_raw: Vec<u8>,
    output_text: String,
    kind: BlockKind,
    status: BlockStatus,
    git_context: Option<GitContext>,
    suggestions: Vec<SuggestedAction>,
}
```

`BlockKind`:

- `NormalCommand`
- `FailedCommand`
- `TuiSession`
- `AiGenerated`
- `SystemEvent`

`BlockStatus`:

- `Running`
- `Success`
- `Failed`
- `Interrupted`
- `Unknown`

### TuiSession

`TuiSession` represents a handoff app session. It can also be represented as a `CommandBlock` with `kind = BlockKind::TuiSession`.

Suggested fields:

```rust
struct TuiSession {
    app_name: String,
    command: String,
    cwd_before: PathBuf,
    cwd_after: Option<PathBuf>,
    started_at: DateTime,
    finished_at: Option<DateTime>,
    duration_ms: Option<u64>,
    exit_code: Option<i32>,
    snapshot_before: Option<SessionSnapshot>,
    snapshot_after: Option<SessionSnapshot>,
    after_exit_results: Vec<AfterExitResult>,
}
```

`SessionSnapshot`:

- `cwd: PathBuf`
- `git_branch: Option<String>`
- `git_status_short: Option<String>`
- `git_diff_stat: Option<String>`
- `changed_files: Vec<String>`

`AfterExitResult`:

- `command: String`
- `exit_code: i32`
- `output_text: String`

## State Machine

Use a clear Rust `enum` plus `match`. Do not over-abstract early with traits or generics.

`AppMode`:

- `ShellIdle`: zsh is waiting for input at a normal shell prompt. Block interaction may be entered.
- `CommandRunning`: an ordinary command is running. Tide appends output to the active `CommandBlock`.
- `TuiHandoff`: the command matches a configured TUI app. Tide fully yields the terminal.
- `Returning`: a TUI app or command just exited. Tide may show a short transition and run after-exit jobs.
- `BlockInteraction`: the user browses and operates on command blocks.
- `ReturnPanel`: Tide displays context after a TUI session exits.

`AppEvent`:

- `KeyInput(Vec<u8>)`
- `PtyOutput(Vec<u8>)`
- `ShellPreexec { command: String }`
- `ShellPrecmd { exit_code: i32 }`
- `CwdChanged { cwd: String }`
- `CommandStarted { block_id: BlockId, command: String }`
- `CommandOutput { block_id: BlockId, bytes: Vec<u8> }`
- `CommandFinished { block_id: BlockId, exit_code: i32 }`
- `TuiAppMatched { command: String, app_name: String }`
- `TuiAppExited { command: String, exit_code: i32 }`
- `BlockSelected { block_id: BlockId }`
- `BlockActionRequested { block_id: BlockId, action: BlockAction }`
- `ReturnStarted { block_id: BlockId }`
- `ReturnFinished { block_id: BlockId }`
- `Tick`
- `Resize { cols: u16, rows: u16 }`
- `Shutdown`

## zsh Lifecycle Hooks

Tide should inject or generate a zsh hook script that users can source from `.zshrc`.

The hook script emits invisible OSC events that Tide parses from PTY output and does not display.

Example:

```zsh
autoload -Uz add-zsh-hook

_tide_preexec() {
  print -n "\e]777;tide;preexec;$(printf %q "$1")\a"
}

_tide_precmd() {
  print -n "\e]777;tide;precmd;$?\a"
}

_tide_chpwd() {
  print -n "\e]777;tide;cwd;$PWD\a"
}

add-zsh-hook preexec _tide_preexec
add-zsh-hook precmd _tide_precmd
add-zsh-hook chpwd _tide_chpwd
```

Parsing rules:

- identify these OSC events while reading PTY output
- strip them from user-visible output
- convert them into `AppEvent`
- do not attempt a full ANSI/VT parser in the first phase

Alternate screen detection may be used as an auxiliary signal:

- `ESC [ ? 1049 h`
- `ESC [ ? 1049 l`
- `ESC [ ? 1047 h/l`
- `ESC [ ? 1048 h/l`

Do not rely only on alternate screen. Primary lifecycle boundaries come from zsh `preexec` / `precmd` or process exit.

## Ordinary Command Block Lifecycle

1. Tide starts in `ShellIdle`.
2. The user enters a command and presses Enter.
3. zsh `preexec` emits an OSC event.
4. Tide receives `ShellPreexec { command }`.
5. Tide checks whether the command matches a configured TUI handoff app.
6. If it does not match, Tide creates a `CommandBlock` with:
   - `kind = NormalCommand`
   - `status = Running`
   - command
   - cwd
   - `started_at = now`
   - empty `output_raw`
   - empty `output_text`
7. Tide enters `CommandRunning`.
8. PTY output during `CommandRunning` is appended to the current block.
9. zsh `precmd` emits an OSC event.
10. Tide receives `ShellPrecmd { exit_code }`.
11. Tide completes the block:
   - `finished_at = now`
   - computed `duration_ms`
   - recorded `exit_code`
   - `status = Success` or `Failed`
   - `output_text = cleaned ANSI text`
12. Tide returns to `ShellIdle`.
13. The user may enter `BlockInteraction` and operate on the block.

## TUI Handoff Lifecycle

1. Tide receives `ShellPreexec { command }`.
2. Tide checks whether the command matches `tui_apps` config.
3. If it matches, Tide creates a `CommandBlock`:
   - `kind = TuiSession`
   - `status = Running`
   - command
   - cwd
   - `started_at = now`
4. Tide creates `TuiSession` metadata.
5. If configured, record `snapshot_before`.
6. Tide enters `TuiHandoff`.
7. During `TuiHandoff`, do not draw Tide UI or intercept TUI behavior.
8. zsh `precmd` or process exit confirms the TUI app has exited.
9. Tide completes the `TuiSession` block.
10. If configured, record `snapshot_after`.
11. Run `after_exit` commands.
12. Enter `Returning` or `ReturnPanel`.
13. Return to `ShellIdle`.

## Configuration

Example `config/tide.toml`:

```toml
[shell]
program = "zsh"

[ui.transitions]
enabled = true
duration_ms = 220
fps = 30
skip_if_fast_under_ms = 80
reduced_motion = false

[blocks]
max_blocks = 10
max_output_bytes_per_block = 1048576
strip_ansi_for_text = true
persist_session = false

[tui_apps.nvim]
commands = ["nvim", "vim"]
handoff = true
snapshot = ["git", "cwd"]
return_panel = "changed-files"

[tui_apps.lazygit]
commands = ["lazygit"]
handoff = true
snapshot = ["git"]
after_exit = ["git status --short"]
return_panel = "git"

[tui_apps.opencode]
commands = ["opencode"]
handoff = true
snapshot = ["git", "cwd"]
after_exit = ["git status --short", "git diff --stat"]
return_panel = "summary"

[tui_apps.fzf]
commands = ["fzf"]
handoff = true
return_panel = "none"
```

First command matching should be conservative. Match only `argv[0]`, such as `nvim`, `vim`, `lazygit`, `opencode`, `fzf`, and `less`. Add glob or regex later.

## UI Model

Tide uses a two-mode terminal rendering model:

- **Shell Mode** (default): PTY output parsed through `alacritty_terminal`, Tide renders the grid with inline block frames. User sees normal shell + block decorations.
- **TUI Handoff Mode**: Configured TUI apps get full terminal control. Transparent byte forwarding, no parsing, no overlay.
- **BlockInteraction Mode**: `Ctrl-X Ctrl-B` enters alternate-screen TUI for browsing blocks. j/k selection, highlighted active block, read-only for now.

Early visual rule: block frames use simple line borders (`┌─ ───┐` / `└─ ───┘`). Keep structural and readable. No decorative styling or animations yet.

## BlockInteraction UI

Suggested shortcut: `Ctrl-X Ctrl-B`.

List view:

```text
Tide Blocks
> [12] cargo build        failed   2.4s
  [11] git status         success  0.1s
  [10] opencode           session  6m18s
```

Selected block view:

```text
Block #12
command: cargo build
cwd: ~/project
exit: 101
duration: 2.4s

Output
error[E0432]: unresolved import ...

Actions
[e] explain error
[f] generate fix command
[r] rerun
[c] copy command
[o] copy output
[s] summarize
```

Key behavior:

- `Esc`: leave `BlockInteraction`, return to `ShellIdle`
- `Enter`: run default action
- `j` / `k` or arrow keys: select block
- `e`: explain error
- `f`: generate fix command
- `r`: insert original command into zsh prompt
- `c`: copy command
- `o`: copy output
- `s`: summarize block

## Return Transition

Return transitions are only for handoff boundaries. They must not pollute ordinary shell usage.

Rules:

- short, restrained, and skippable
- skip if recovery takes less than 80 ms
- show a short transition for 80-500 ms
- show real task status after 500 ms
- allow `Esc` to skip if recovery takes more than 2 s
- never block the PTY main loop

## ReturnPanel

After a TUI exits, the panel may show:

```text
Returned from opencode
exit 0 · 6m 18s · ~/project

Changed files:
  M src/main.rs
  M Cargo.toml

Suggested next:
  cargo test
  cargo fmt
  git diff
  ask AI to summarize changes
```

User actions:

- `Enter`: insert selected suggested command into the zsh prompt, but do not auto-execute
- `Esc`: close `ReturnPanel`, return to ordinary zsh
- `Ctrl-C`: cancel return jobs, return to ordinary zsh

## AI Scope

AI is not a first-phase goal.

Later, an AI adapter may support:

- explain selected block
- explain last error
- suggest next command for selected block
- summarize block output
- summarize git diff
- summarize what changed after an `opencode` or `nvim` session
- generate shell command from natural language

Do not make the first version depend on AI. Do not introduce a complex agent system early.

## Milestones

### Milestone 1: Transparent zsh Wrapper

Goal:

- `cargo run` starts `tide`
- `tide` starts real `zsh`
- user input reaches zsh normally
- zsh output is displayed as-is
- `Ctrl-C` works
- `Ctrl-D` / `exit` works
- terminal resize works
- terminal state is restored on exit

Do not implement AI, animation, `BlockInteraction`, or `ReturnPanel` in this milestone.

### Milestone 2: zsh Lifecycle Hooks and Command Block Capture

Goal:

- generate Tide zsh hook script
- identify `preexec`, `precmd`, and `chpwd` OSC events
- hide OSC events from visible output
- track current command, exit code, and cwd in `AppState`
- create one `CommandBlock` per ordinary command
- append command output while running
- save exit code, duration, and cleaned output when finished
- show blocks in debug logs

### Milestone 3: Basic Block List UI

Goal:

- enter `BlockInteraction` with a shortcut
- show recent command blocks
- select blocks with keyboard
- show command, exit code, duration, and output text
- return to ordinary zsh with `Esc`
- no AI yet

### Milestone 4: Block Actions

Goal:

- copy command
- copy output
- rerun command by inserting it into the zsh prompt
- collapse or expand output
- delete block from current session view

### Milestone 5: TUI Handoff-Return Detection

Goal:

- read `config/tide.toml`
- recognize configured commands such as `nvim`, `vim`, `lazygit`, `opencode`, `fzf`, and `less`
- enter `TuiHandoff` on match
- generate `TuiSession` block on exit
- do not interfere with TUI input or output
- support `return_panel = "none"` by returning directly to `ShellIdle`

### Milestone 6: Return Transition and ReturnPanel

Goal:

- show `Returning` transition after TUI exit
- run `after_exit` commands asynchronously
- display `ReturnPanel`
- support `Esc` to close
- support `Enter` to insert suggested command

### Milestone 7: AI Adapter

Goal:

- optional integration with `opencode` or an LLM provider
- explain selected block
- summarize block
- suggest next command
- keep AI-generated commands insert-only by default

## Suggested First Implementation Step

Milestone 1 has been bootstrapped. The current near-term work is Milestone 2 hardening: make zsh lifecycle hooks and block capture reliable before polishing Block Mode UI.

Next implementation target:

1. Move hook installation out of the user-visible PTY input stream.

   Current implementation writes the hook script into the running PTY after spawning zsh. This is acceptable for the early prototype but should be replaced with a cleaner startup-time mechanism.

   Preferred direction:

   - generate a temporary Tide hook file for the current process
   - start zsh in a way that sources that hook before interactive use
   - preserve the user's normal zsh configuration and prompt behavior
   - avoid adding hook installation commands to shell history
   - avoid visible hook script text during startup
   - delete temporary hook files on exit when possible

2. Keep hook payloads encoded.

   Continue using encoded OSC 777 payloads so commands containing semicolons, newlines, BEL, or other control-sensitive characters do not corrupt parser boundaries.

3. Preserve transparent-first behavior.

   Hook installation must not make Tide feel like a replacement shell. Ordinary startup, prompts, command output, and TUI passthrough should still behave like normal zsh.

4. Keep Block Mode read-only for now.

   Do not add copy, rerun, AI, save, delete, or action-menu behavior until command lifecycle capture is stable.

Acceptance criteria:

- `cargo fmt --check`, `cargo check`, and `cargo test` pass
- parser tests still cover split events, multiple events in one PTY chunk, and encoded payloads
- `cargo run` starts zsh without visibly printing the hook script
- ordinary commands are captured as blocks
- `false` is captured with a failed status and non-zero exit code
- `Ctrl-X Ctrl-B` still opens the read-only latest-10 Block Mode
- `Esc` or `q` returns from Block Mode to the transparent shell
- terminal state is restored after exit

Historical bootstrap steps for a fresh repository:

1. Create the Rust project:

   ```sh
   cargo init --bin --name tide
   ```

2. Add dependencies:

   - `anyhow`
   - `thiserror`
   - `tracing`
   - `tracing-subscriber`
   - `serde` with `derive`
   - `toml`
   - `tokio` with `full`
   - `ratatui`
   - `crossterm`
   - `portable-pty`
   - `strip-ansi-escapes`
   - `arboard` or `copypasta` for later clipboard support

3. Implement:

   - `Config` and loading from `config/tide.toml`
   - `AppMode`
   - `AppEvent`
   - `CommandBlock`
   - `TuiSession`
   - `App`
   - `pty::spawn_shell`
   - transparent input/output passthrough
   - terminal restoration on exit

## Development Principles

- Stabilize PTY behavior before building UI.
- Build block capture before AI.
- Use `alacritty_terminal` for terminal parsing — do not write a custom ANSI/VT parser.
- Tide renders shell output (tmux-style), TUI apps get full transparent passthrough.
- Treat block-based execution and TUI handoff-return as equal core features.
- Never steal control during `TuiHandoff`.
- Do not break the user's existing zsh configuration.
- Always restore terminal state after errors.
- AI-generated commands must be inserted, not executed.
- Use Rust `enum` plus `match` for the state machine.
- Avoid premature trait or generic abstraction.
- Keep `main.rs` thin.
- Use `anyhow::Result` initially for error handling.
- Derive `Debug`, `Clone`, and `Deserialize` where appropriate.
- Store block output as raw bytes first, then derive stripped text.
- Keep the first in-memory BlockStore small. The current default is the latest 10 blocks only.
- Enforce `max_output_bytes_per_block` to avoid memory growth.
- Persistence is optional and must never be on the PTY hot path.
- Add unit tests for event parsing.
- Commit after each milestone when working in a git repository.
- Maintain [docs/manual-testing.md](./docs/manual-testing.md) as terminal behavior evolves.
- When adding or changing terminal behavior, zsh lifecycle handling, block interaction, TUI handoff-return, or AI command insertion, update the manual testing checklist in the same change.
- Before committing terminal behavior changes, run `cargo fmt --check && cargo check && cargo test`.

## Current Priority

Migrate from transparent passthrough to terminal-grid rendering.

Next: implement `TermRenderer` using `alacritty_terminal`, replacing the transparent output path with grid-based rendering while preserving block decorations and TUI handoff transparency.

Do not implement:

- AI
- animation
- ReturnPanel
- block actions (copy, rerun, explain, save)
- database persistence
