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
preview_lines = 6
expanded_lines = 30
follow_tail = true
block_gap = 0
```

- `preview_lines` limits body lines for collapsed blocks.
- `expanded_lines` limits body lines for the selected expanded block.
- `follow_tail` starts Block View with tail anchoring enabled.
- `block_gap` inserts blank visual lines between blocks in Block View.

## Block Layout

Normal View does not show block borders or spacer lines. It may apply horizontal padding as a display strategy.

```toml
[block_layout]
horizontal_padding = 1
show_padding_in_plain = true
```

Do not use top padding, bottom padding, or reserved spacer lines for current Block Layer design.

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
