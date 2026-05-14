# Configuration

Tide reads configuration from:

```text
~/.config/tide/config.toml
```

For development, a repository-local file is also supported:

```text
config/tide.toml
```

If no config file exists, Tide uses defaults.

## Shell

```toml
[shell]
program = "zsh"
```

- `program` — the shell binary Tide launches. Defaults to `"zsh"`.

## UI

```toml
[ui]
[ui.transitions]
enabled = true
duration_ms = 220
fps = 30
skip_if_fast_under_ms = 80
reduced_motion = false
```

- `transitions.enabled` — enable view-transition animations.
- `transitions.duration_ms` — animation duration in milliseconds.
- `transitions.fps` — animation frame rate.
- `transitions.skip_if_fast_under_ms` — skip the enter animation when the previous view was displayed for less than this many ms. Avoids flashing on quick round-trips.
- `transitions.reduced_motion` — disable all animation unconditionally.

## Blocks

```toml
[blocks]
max_blocks = 1000
max_output_bytes_per_block = 1048576
strip_ansi_for_text = true
persist_session = false
```

- `max_blocks` — maximum number of execution blocks kept in memory. The default is `1000`.
- `max_output_bytes_per_block` — cap output storage per block (in bytes). If a command produces more output than this, the surplus is discarded and a `"· truncated"` marker appears in the bottom border. Default `1048576` (1 MB).
- `strip_ansi_for_text` — strip ANSI escape sequences when formatting output as plain text. The original ANSI is still stored and used for rendered display.
- `persist_session` — save/restore blocks across Tide restarts. Not yet wired.

## History

`BlockStore` history retention is separate from the number of blocks visible on screen.

```toml
[history]
max_blocks = 1000
```

`max_blocks` controls how many execution blocks Tide keeps in memory. The default is `1000`.

## Block View

Block View has its own viewport and preview limits:

```toml
[block_view]
preview_lines = 4
expanded_lines = 15
follow_tail = true
block_gap = 0
scroll_margin_blocks = 2
scroll_margin_lines = 2
auto_follow_on_reach_bottom = false
horizontal_margin = 1
body_padding = 1
show_footer = true
copy_format = "plaintext"
```

- `preview_lines` limits body lines for collapsed blocks. Default `4`.
- `expanded_lines` limits body lines for the selected expanded block. Default `15`.
- `follow_tail` starts Block View with tail anchoring enabled. Default `true`.
- `block_gap` inserts blank visual lines between blocks in Block View. Default `0`.
- `scroll_margin_lines` keeps a small visual-line margin around the selected block during keyboard navigation. Default `2`.
- `scroll_margin_blocks` is legacy compatibility for the old block-index viewport and should not be used for new logic. Default `2`.
- `auto_follow_on_reach_bottom` controls whether pressing `j` onto the newest block re-enters Tail anchor. Default `false`.
- `horizontal_margin` keeps block borders away from terminal edges. Default `1`.
- `body_padding` controls inner body padding. Default `1`.
- `show_footer` displays Block View shortcuts on the last line. Default `true`.
- `copy_format` — clipboard serialization format. Default `"plaintext"`.

  | Value | Description |
  |-------|-------------|
  | `"plaintext"` | Plain text with blocks separated by `\n\n---\n\n` |
  | `"markdown"` | Markdown fenced code blocks per block |
  | `"shell_transcript"` | Shell session transcript format with prompt markers |
  | `"json"` | Structured JSON `block_export.v1` (single block or array), including metadata, truncation flags, and derived `views` |

## Block Layout

Normal View does not show block borders or spacer lines. It may apply horizontal padding as a display strategy.

```toml
[block_layout]
horizontal_padding = 1
show_padding_in_plain = true
```

- `horizontal_padding` — horizontal margin applied to block content in Normal View. Default `1`.
- `show_padding_in_plain` — whether to show horizontal padding in Plain View. Default `true`.

Do not use top padding, bottom padding, or reserved spacer lines for current Block Layer design.

## Keymap

Key bindings can be overridden per view. The default bindings are listed in the help overlay (`?`).

```toml
[keymap]
[keymap.blocks]
nav_down = "j"
nav_up = "k"
expand = "enter"
detail_view = "i"

[keymap.detail]
nav_down = "j"
nav_up = "k"
quit = "q"
```

### Block View actions

| Action | Default | Description |
|--------|---------|-------------|
| `nav_down` | `j` | Move selection down |
| `nav_up` | `k` | Move selection up |
| `nav_top` | `g` | Jump to first block |
| `nav_bottom` | `G` | Jump to last block |
| `scroll_half_down` | `Ctrl-D` | Scroll viewport down by half a screen |
| `scroll_half_up` | `Ctrl-U` | Scroll viewport up by half a screen |
| `scroll_full_down` | `Ctrl-F` | Scroll viewport down by one screen |
| `scroll_full_up` | `Ctrl-B` | Scroll viewport up by one screen |
| `expand` | `Enter` | Toggle inline block expansion |
| `detail_view` | `i` | Enter full-screen Detail View pager |
| `toggle_failed_filter` | `f` | Show/hide failed blocks only |
| `open_search` | `/` | Open search bar |
| `search_next` | `n` | Jump to next search match |
| `search_prev` | `N` | Jump to previous search match |
| `copy_command` | `c` | Copy command text |
| `copy_output` | `o` | Copy output text |
| `copy_both` | `y` | Copy command and output |
| `rerun` | `r` | Rerun the selected command |
| `delete` | `d` | Delete block |
| `visual_mode` | `v` | Toggle visual selection mode |
| `help` | `?` | Toggle help overlay |
| `quit` | `q` / `Esc` | Leave Block/Detail View |

### Detail View actions

| Action | Default | Description |
|--------|---------|-------------|
| `nav_down` | `j` | Scroll down |
| `nav_up` | `k` | Scroll up |
| `nav_top` | `g` | Jump to top |
| `nav_bottom` | `G` | Jump to bottom |
| `scroll_half_down` | — | Scroll viewport down by half a screen |
| `scroll_half_up` | — | Scroll viewport up by half a screen |
| `scroll_full_down` | — | Scroll viewport down by one screen |
| `scroll_full_up` | — | Scroll viewport up by one screen |
| `copy_command` | `c` | Copy command text |
| `copy_output` | `o` | Copy output text |
| `copy_both` | `y` | Copy command and output |
| `rerun` | `r` | Rerun the selected command |
| `visual_mode` | `v` / `V` | Toggle visual selection mode |
| `help` | `?` | Toggle help overlay |
| `quit` | `q` / `Esc` | Return to Block View |

## Classification

Classification is an exception list. Commands that do not match `tui`, `repl`, or `agent` are normal commands and Tide captures their linear output.

```toml
[classification.tui]
commands = ["vim", "nvim", "less", "fzf", "lazygit", "top"]

[classification.repl]
commands = ["python", "python3", "node", "psql", "sqlite3"]

[classification.agent]
commands = ["codex", "claude", "opencode", "aider"]
```

- `classification.tui.commands` — full-screen or terminal-UI programs. Tide suspends capture and shows a TUI placeholder block.
- `classification.repl.commands` — interactive shells, language REPLs, debuggers, and database consoles. Tide suspends capture and shows a REPL placeholder block.
- `classification.agent.commands` — coding agent CLIs. Tide suspends capture and treats the command as an agent-like interactive session.

The default config contains common command lists. To remove a default category, set its command list to an empty array:

```toml
[classification.repl]
commands = []
```

Legacy `[tui_apps]` and `[tui.apps]` entries are still parsed and treated as TUI classification for compatibility, but the classification table is the preferred interface.

## Legacy `raw_programs`

The current architecture does not require a RawProgram whitelist for passthrough or normal rendering.

Normal mode forwards visible PTY bytes directly to the real terminal, so full-screen commands such as these work without detection:

- `vim`
- `nvim`
- `vi`
- `yazi`
- `fzf`
- `less`
- `more`
- `top`
- `htop`
- `btop`
- `ssh`
- `lazygit`
- `lazydocker`
- `man`
- `tig`

Older config files may still contain `raw_programs`. Tide may parse the field for compatibility, but it is not used to decide whether `vim`, `yazi`, `fzf`, `less`, `ssh`, or similar programs get passthrough.

Legacy example:

```toml
raw_programs = [
  "my-tui-app",
  "lg",
  "v",
]
```

Use `[classification.tui]`, `[classification.repl]`, or `[classification.agent]` to control how interactive commands are labeled and captured.

## Shared Agent Registry

Tide supports sharing minimal agent navigation state between instances. Each agent provider is configured independently under `[agents.<provider>]`.

```toml
[agents.opencode]
enabled = true
cwd = "basename"                               # "full" | "basename" | "none"
command = true
start_aliases = ["opencode", "oc"]             # shell commands/aliases that launch this agent
process_prefixes = ["opencode", "opencode-"]   # TTY process-scan prefixes
display_name = "opencode"                      # label shown in Block View
```

All fields have sensible provider defaults and can be omitted. The minimal config is just:

```toml
[agents.opencode]
enabled = true
```

To add your own shell aliases:

```toml
[agents.opencode]
enabled = true
start_aliases = ["opencode", "oc", "ai"]
```

### Fields

- `enabled` — enable shared block sync and tmux jump behavior for this provider.
- `cwd` — privacy level for the cwd stored in the registry:
  - `"full"` — full path
  - `"basename"` — last path component only (default)
  - `"none"` — no cwd stored
- `command` — share the original command text; `false` stores only the display name.
- `start_aliases` — shell command names or aliases that trigger detection on `preexec` (e.g. `["opencode", "oc"]`). When empty, the provider default is used. The legacy key `command_match` is accepted as an alias.
- `process_prefixes` — binary name prefixes used for TTY process scanning (handles versioned / platform-suffixed binaries). When empty, the provider default is used.
- `display_name` — label used in Block View for this agent's blocks (e.g. `[a] opencode ~/path`). When empty, the provider name is used.

### Adding a new provider

Add a new `[agents.<name>]` section. The provider name must match a variant in `AgentProvider` (currently: `opencode`).

### Backward compatibility

The legacy `[opencode_share]` section is still accepted and maps to `[agents.opencode]`. It is ignored when `[agents.opencode]` is present.

The shared registry stores navigation state only (alias, tmux target, status metadata). It does not store command output, prompt/reply content, or full session context.

## Agent Live Events

When Tide is running inside tmux, it injects two environment variables into the shell:

| Variable | Value | Purpose |
|----------|-------|---------|
| `TIDE_AGENT_EVENTS_DIR` | `~/.tide/agents` | Base directory for per-pane event files |
| `TIDE_TMUX_PANE` | e.g. `%117` | Tmux pane ID of the Tide shell |

Agent plugins write real-time status events to:

```
$TIDE_AGENT_EVENTS_DIR/$TIDE_TMUX_PANE/events.jsonl
```

For example: `~/.tide/agents/%117/events.jsonl`

Both variables must be present for agent live status to work. In non-tmux environments they are not set and agent monitoring is silently disabled.

### File layout

```
~/.tide/agents/{pane_id}/
  events.jsonl   — real-time event stream (append-only)
  history.json   — last 5 conversation turns (rewritten after each reply)
```

### events.jsonl format

Each event is a single JSON line. Required fields: `type` and `at_ms` (Unix milliseconds).

```jsonl
{"type":"started","at_ms":1715000000000,"cwd":"/projects/myapp"}
{"type":"thinking","at_ms":1715000001000}
{"type":"tool_call","at_ms":1715000002000,"tool_name":"bash","command":"cargo test"}
{"type":"tool_result","at_ms":1715000003000,"tool_name":"bash","exit_code":0}
{"type":"reply","at_ms":1715000004000,"text":"All tests passed."}
{"type":"idle","at_ms":1715000005000}
```

| Event type | Block View label | Key fields |
|------------|-----------------|------------|
| `started` | _(none)_ | `cwd`, `model` |
| `thinking` | `· thinking` | — |
| `tool_call` | `· tool` or `· running: <cmd>` | `tool_name`, `command` (bash/exec tools) |
| `tool_result` | _(no label change)_ | `tool_name`, `exit_code` |
| `reply` | `· replying` | `text` |
| `question` | `· question` | `text` |
| `request` | `· request` | `summary` |
| `idle` | _(none)_ | — |
| `exit` | `· exited` | `code` |
| `error` | `· error` | — |

### history.json format

Written by the plugin after every assistant reply. Contains the last 5 conversation turns, newest last.

```json
{
  "records": [
    {
      "at_ms": 1715000000000,
      "user_message": "fix the failing tests",
      "tool_calls": [
        { "tool_name": "bash", "command": "cargo test" },
        { "tool_name": "bash", "command": "cargo fix --allow-dirty" }
      ],
      "reply_summary": "I ran the tests and applied the fixes…"
    }
  ]
}
```

Each record maps to an `AgentHistoryRecord` in Tide's `AgentLiveSnapshot.recent_history`.

### Update trigger

Tide polls event file mtimes every 500 ms while Block View is open. A sync and re-render fires only when a mtime changes — no updates happen when no events are written.

### Reading strategy

Tide reads only the last 64 KB of `events.jsonl` (max 200 lines), scanning newest-to-oldest. Partial lines at the seek boundary are silently skipped. `history.json` is read in full (always small: ≤5 records).
