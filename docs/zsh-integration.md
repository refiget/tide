# Zsh Integration

Tide must preserve the user's native zsh environment.

Tide starts the user's configured shell as an interactive shell and sets:

```text
TIDE=1
TIDE_SESSION_ID=<session id>
```

Tide does not replace the prompt, does not clear zsh hook arrays, and does not require `zsh -f` or `--no-rcs`.

## User Installation

Users should source Tide's integration from their own `.zshrc`:

```zsh
source ~/.tide/zsh-integration.zsh
```

If the integration is not installed, Tide enters degraded mode: shell output still renders, but command blocks cannot be accurately captured.

## Integration Script

The integration only emits invisible OSC markers.

```zsh
autoload -Uz add-zsh-hook

_tide_escape_osc() {
  printf '%s' "$1" | command od -An -tx1 -v | command tr -d ' \n'
}

_tide_preexec() {
  local cmd="$1"
  cmd="$(_tide_escape_osc "$cmd")"
  printf '\033]777;block_start;cmd=hex:%s\a' "$cmd"
}

_tide_emit_cwd() {
  local cwd="$PWD"
  cwd="$(_tide_escape_osc "$cwd")"
  printf '\033]777;cwd;cwd=hex:%s\a' "$cwd"
  printf '\033]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
}

_tide_precmd() {
  local ec=$?
  local cwd="$PWD"
  cwd="$(_tide_escape_osc "$cwd")"
  _tide_emit_cwd
  printf '\033]777;block_end;exit=%d;cwd=hex:%s\a' "$ec" "$cwd"
}

add-zsh-hook preexec _tide_preexec
add-zsh-hook precmd _tide_precmd
add-zsh-hook chpwd _tide_emit_cwd
```

`_tide_emit_cwd` exists because Tide is the process tmux sees for the pane.
When the inner zsh changes directory, Tide must update its own process cwd so
tmux `split-window -c "#{pane_current_path}"` and `new-window -c
"#{pane_current_path}"` inherit the expected directory. The `chpwd` hook emits
this immediately after `cd`; `precmd` repeats it as a final synchronization
point before command completion.

The OSC 777 `cwd` marker is stripped by Tide and used internally. The OSC 7
`file://...` marker is passed through for terminals/tmux versions that track
working directories via standard terminal cwd reporting.

## Prompt Redraw Widget

Tide registers an internal prompt redraw widget for debugging and future use:

```zsh
_tide_redraw_prompt() {
  zle reset-prompt
  zle -R
}
zle -N _tide_redraw_prompt 2>/dev/null
bindkey '^X^R' _tide_redraw_prompt 2>/dev/null
```

The widget is bound to `Ctrl-X Ctrl-R` (`^X^R`, an unusual sequence unlikely
to conflict with user bindings).  It is **not** automatically triggered during
normal operation — the alternate screen exit alone restores the main screen
correctly.  Users and developers can invoke it manually for debugging.

**This is an internal binding — do not depend on it in user configuration.**

## Rules

- Do not output visible UI from zsh hooks.
- Do not insert top or bottom spacer lines.
- Do not modify `PROMPT`, `RPROMPT`, `PS1`, or theme state.
- Do not overwrite existing `preexec` or `precmd` functions.
- Use `add-zsh-hook` to append hooks.
- Rust must strip markers before text enters `ShellBuffer`.
- Non-Tide OSC sequences such as OSC 7 must pass through unchanged.
- Markers must never appear in Normal View.
