# Tide

Tide is a zsh-native shell workspace with command blocks and TUI handoff-return.

Tide 是一个基于 zsh 的 shell 工作环境，提供命令结果块和 TUI 应用交接返回机制。

## Positioning

Tide runs inside the user's existing terminal emulator. It is not a new terminal emulator and not a replacement shell. It is a PTY wrapper around real `zsh`.

Tide renders shell output through a terminal grid (like tmux), using `vt100` for parsing and grid maintenance. TUI applications are detected and given transparent passthrough.

The user runs `tide`, Tide starts real `zsh`, and then Tide adds two core capabilities above the shell:

1. Block-based execution
2. TUI handoff and return

## Core Feature 1: Block-Based Execution

Every command execution becomes a selectable and interactive `CommandBlock`.

A block records:

- command
- output
- exit code
- duration
- cwd
- timestamp
- git context

Users can select a block and act on it:

- copy command
- copy output
- rerun command
- explain error
- summarize output
- generate a fix command
- collapse or expand output
- save the block

AI-generated commands must be inserted into the zsh prompt by default. They must not auto-execute without the user pressing Enter.

## Core Feature 2: TUI Handoff-Return

Users can configure specific commands as TUI handoff apps, such as:

- `nvim`
- `vim`
- `lazygit`
- `opencode`
- `fzf`
- `less`

When one of these commands runs, Tide enters `TuiHandoff` mode. In this mode Tide gives the app full control of the terminal:

- no Tide overlay
- no Tide key interception
- no parsing of the TUI app's internal state
- transparent input and output forwarding

After the TUI app exits, Tide restores the shell workspace, creates a `TuiSession` block, and can show return context such as:

- exit status
- duration
- cwd
- git changes
- changed files
- suggested next commands

## Product Boundaries

Tide is:

- a zsh PTY wrapper
- a command lifecycle manager
- a block-based shell workspace
- a command result interaction layer
- a TUI app handoff-return runtime

Tide is not:

- a new terminal emulator
- a replacement for zsh
- a `tmux` / `zellij` style multiplexer
- a full IDE
- an AI agent-first product
- a simple AI command launcher

## Architecture

Tide uses terminal-grid rendering similar to tmux:

```
real terminal        ← Tide renders grid here
  |
TermRenderer         ← vt100 grid + diff rendering
  |
Osc777Parser         ← splits visible output from hook events
  |
PTY master
  |
zsh
  |
child commands / TUI apps
```

Shell mode: PTY → parse → grid → render with block frames
TUI mode: PTY → raw bytes → transparent passthrough (no parsing)

## Development Direction

Current priority: migrate from transparent passthrough to terminal-grid-based rendering using `vt100`. Shell output is parsed into a grid and rendered with block decorations. TUI apps get full transparent passthrough.

Upcoming: block mode as scrollback navigation (tmux copy-mode style), TUI handoff-return detection, return context, and eventually optional AI integrations.

The first block implementation keeps only the latest 10 blocks in memory for the current Tide session. Persistence is optional future work.

The two product lines are parallel:

```text
command -> block -> select -> interact

configured TUI command -> handoff -> exit -> return context
```

They share the same zsh lifecycle, PTY wrapper, state machine, and session model.
