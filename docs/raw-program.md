# Full-Screen Program Compatibility

Tide does not need a first-phase RawProgram whitelist.

Normal mode is transparent passthrough:

```text
PTY visible bytes -> real terminal
PTY visible bytes -> sidecar plain-text capture
```

Because Tide does not redraw Normal mode, full-screen and interactive terminal programs naturally keep control of the terminal while they run.

Examples:

- `vim`, `nvim`, `vi`
- `yazi`
- `fzf`
- `less`, `more`, `man`
- `top`, `htop`, `btop`
- `ssh`
- `lazygit`, `lazydocker`, `tig`

## Lifecycle

Tide still records the command lifecycle through zsh integration:

1. `block_start` creates an `ExecutionBlock`.
2. Visible PTY bytes are forwarded to the real terminal.
3. Tide captures best-effort plain text on the side.
4. `block_end` finishes the block with exit code, cwd, duration, and status.

Do not use command-name detection to decide whether passthrough is allowed.

If Tide sees alternate-screen control sequences while a command is running, it may pause sidecar text capture for the rest of that command. This prevents full-screen UI bytes from polluting `ShellBuffer`. The terminal output is still forwarded normally.

## Capture Rules

Do not try to emulate or replay full-screen program internals in the first phase.

If Tide captures no linear text for a command, Block View renders a placeholder body line:

```text
│ no captured text output │
```

Detail View may show the command, cwd, exit code, duration, and status. It should not claim that Tide captured a full-screen program's internal screen state.

## Future Metadata

Future versions may label a block as interactive based on heuristics, shell metadata, or user configuration. That label is only metadata for Block View. It must not be required for `vim`, `yazi`, `fzf`, `less`, `ssh`, or similar programs to work in Normal mode.
