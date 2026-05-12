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

## TUI Apps

TUI application definitions let Tide recognise full-screen programs and optionally snapshot their output or run cleanup commands after they exit.

```toml
[tui_apps]
[tui_apps.lazygit]
commands = ["lazygit", "lg"]
handoff = true
snapshot = []
after_exit = ["clear"]
return_panel = "none"
```

- `commands` — binary names or aliases that identify the app.
- `handoff` — whether Tide enters a passthrough handoff mode (not yet wired).
- `snapshot` — commands to run after the app exits to capture terminal state (not yet wired).
- `after_exit` — shell commands to run after the app exits (e.g. `"clear"`).
- `return_panel` — which panel to show after the app exits: `"none"`, `"plain"`, `"blocks"`, or `"detail"`. Default `"none"`.

## Legacy `raw_programs`

The current architecture does not require a RawProgram whitelist for passthrough.

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

Future versions may use this or a replacement setting as metadata for labeling interactive blocks. It must not be required for terminal passthrough.

## Shared Agent Registry

Tide supports sharing minimal agent navigation state between instances (current provider: `opencode`).

```toml
[opencode_share]
enabled = true
cwd = "basename"   # "full" | "basename" | "none"
command = true
```

- `enabled` — enable shared agent block sync and jump behavior.
- `cwd` — privacy level for shared cwd in registry:
  - `"full"` stores full cwd path
  - `"basename"` stores only last path component
  - `"none"` stores no cwd
- `command` — whether to share original command text (`false` stores generic `"opencode"`).

The shared registry stores navigation state only (alias, tmux target, status metadata). It does not store command output, prompt/reply content, or full session context.
