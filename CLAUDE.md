# CLAUDE.md

Follow the project guidance in [AGENTS.md](./AGENTS.md).

Short version:

Tide is a zsh-native shell workspace with command blocks and TUI handoff-return.

The two parallel core product lines are:

```text
command -> block -> select -> interact

configured TUI command -> handoff -> exit -> return context
```

Current implementation priority is Milestone 1: build a stable transparent zsh PTY wrapper in Rust. Do not start with AI, animation, ReturnPanel, or a full BlockInteraction UI.
