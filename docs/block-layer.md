# Block Layer

## What A Block Means

An `ExecutionBlock` is the structured record for one shell command execution.

The shell still behaves like zsh, but Tide captures command lifecycle boundaries and associates output lines with a block. This lets Tide render the same shell history in multiple ways:

- Plain View: transparent zsh passthrough with sidecar capture
- Block View: shell text plus block metadata
- Detail View: shell text plus metadata plus inline details for the selected block

Blocks are not a separate page. They are metadata over the original shell history.

## ExecutionBlock Fields

An `ExecutionBlock` should carry:

- `id: BlockId`
- `command: String`
- `cwd: Option<PathBuf>`
- `stdout: String`
- `stderr: String`
- `exit_code: Option<i32>`
- `duration: Option<Duration>`
- `status: BlockStatus`
- `kind: ExecutionKind`
- `start_line: usize`
- `end_line: usize`
- `created_at: SystemTime`

Early implementations may merge stdout and stderr into one text stream, but the model should leave room to separate them later.

`ExecutionKind` may later distinguish normal commands from interactive metadata:

```rust
pub enum ExecutionKind {
    Normal,
    RawProgram,
}
```

## ShellLine And BlockId

`ShellBuffer` stores shell text lines:

```rust
pub struct ShellLine {
    pub text: String,
    pub block_id: Option<BlockId>,
}
```

`ShellLine.block_id` tells the compositor which block, if any, owns that shell line. This relationship is display metadata, not a rendered border.

Do not store block border text in `ShellBuffer`.

Do not store block border text in `ShellBuffer`.

Full-screen program output is not specially classified in the first phase. Normal mode is transparent for every command. If Tide cannot capture useful linear text for a command, Block View shows a placeholder body line.

## BlockLayout

`BlockLayout` describes where a block appears in shell history:

```rust
pub struct BlockLayout {
    pub block_id: BlockId,
    pub start_line: usize,
    pub end_line: usize,
}
```

The first version may keep `start_line` and `end_line` directly on `ExecutionBlock`. A separate `BlockLayout` type becomes useful when layout needs to diverge from execution data, such as collapsed output, filtered history, or virtualized rendering.

## Plain View Generation

Plain View / Normal mode primarily renders by passthrough:

```text
PTY visible bytes -> real terminal
PTY visible bytes -> sidecar capture
```

When Tide needs to restore a captured plain view after leaving Block View, it may render shell text:

```text
ShellBuffer.lines
  -> VisualLine::ShellText
```

Plain View must not show:

- block borders
- block ids
- command metadata
- detail lines
- selection state

The user should feel like they are using ordinary zsh. Normal mode should not continuously redraw through the Block renderer.

Plain View may apply horizontal display padding, controlled by:

```toml
[block_layout]
horizontal_padding = 1
show_padding_in_plain = true
```

This padding is not block UI and is not written into `ShellBuffer`.

## Block View Generation

Block View overlays block metadata on the same shell history.

`BlockStore` history and on-screen visibility are separate:

- `BlockStore.max_blocks` controls retention.
- `BlockViewport.selected_index` controls selected history index.
- `BlockViewport.scroll_offset` controls first visible block.
- `BlockViewConfig.preview_lines` controls collapsed output height.
- `BlockViewConfig.expanded_lines` controls expanded output height.
- `BlockViewConfig.block_gap` controls blank visual lines between blocks.
- `BlockViewConfig.scroll_margin_blocks` keeps navigation from pinning the selected block to the edge.

For each block:

```text
insert VisualLine::BlockTopBorder
insert VisualLine::BlockBodyLine values for lines belonging to the block
insert VisualLine::BlockBottomBorder
```

The top border should show at least:

- block id
- command

The bottom border should show at least:

- block id
- status
- exit code
- duration

The selected block should be visibly highlighted. Do not rely only on color; using a distinct border shape such as `╭ ╮ ╰ ╯` is acceptable.

Collapsed blocks show at most `preview_lines` body lines. If more output exists, append:

```text
... N more lines, press Enter to expand
```

## Detail View Generation

Detail View starts with the same generation rules as Block View.

If the current block is the expanded block:

```text
insert VisualLine::BlockDetailLine values
after the block shell text
before VisualLine::BlockBottomBorder
```

Detail lines should include:

- command
- cwd
- exit code
- duration
- status
- stdout summary
- stderr summary
- actions

Detail View is inline. It is not a popup.

The selected expanded block shows at most `expanded_lines` body lines before detail metadata. If more output exists, append:

```text
... N more lines
```

## Commands With No Captured Text

Full-screen or interactive programs such as these may not produce useful linear text for Block View:

- `vim`
- `nvim`
- `yazi`
- `fzf`
- `less`
- `top`
- `htop`
- `ssh`
- `lazygit`
- `man`

Tide does not need a whitelist for these commands to work. Normal mode is already transparent passthrough.

If a block has no captured body lines, Block View should show a placeholder:

```text
% › yazi
┌─ #8 · yazi ─────────────────────────────────────┐
│ no captured text output                          │
└─ #8 · ok · exit 0 · 1m32s ──────────────────────┘
```

Selected example:

```text
% › vim src/main.rs
╭─ #12 · vim src/main.rs ─────────────────────────╮
│ no captured text output                          │
╰─ #12 · ok · exit 0 · 2m14s ─────────────────────╯
```

Detail View for a block without captured text still shows execution metadata:

```text
Detail
command: yazi
cwd: ~/Projects/demo
exit code: 0
duration: 1m32s
status: ok
actions:
explain | fix | rerun | copy
```

## Selection And Expansion State

Selection and expansion are view state, not block data.

Store these in `ViewState`:

- `selected_block: Option<BlockId>`
- `expanded_block: Option<BlockId>`
- `view: ViewKind`
- `scroll_offset: usize`

Store block viewport state in `BlockViewport`:

- `selected_index: usize`
- `scroll_offset: usize`
- `anchor: ViewAnchor`

`ViewAnchor::Tail` bottom-aligns the visible block region. `ViewAnchor::Manual` preserves viewport position while the selected block remains visible. `ViewAnchor::Top` is used after `g`.

Do not write selected or expanded flags into `ExecutionBlock`.

## Visual Examples

### Plain View

```text
% › ls
api_tracker  Documents  Downloads  Projects
```

### Block View

```text
% › ls
┌─ #1 · ls ───────────────────────────────────────┐
│ api_tracker  Documents  Downloads  Projects     │
└─ #1 · ok · exit 0 · 28ms ───────────────────────┘
```

### Selected Block View

```text
% › cargo build
╭─ #2 · cargo build ──────────────────────────────╮
│ error[E0432]: unresolved import `foo`            │
╰─ #2 · failed · exit 101 · 2.3s ─────────────────╯
```

### Detail View

```text
% › cargo build
╭─ #2 · cargo build ──────────────────────────────╮
│ error[E0432]: unresolved import `foo`            │
│                                                  │
│ Detail                                           │
│ cwd: ~/Projects/demo                             │
│ exit code: 101                                   │
│ duration: 2.3s                                   │
│ status: failed                                   │
│                                                  │
│ actions: explain | fix | rerun | copy            │
╰─ #2 · failed · exit 101 · 2.3s ─────────────────╯
```

## Current Interaction Contract

Plain View:

- ordinary key input goes to zsh
- `Ctrl-B` enters Block View
- full-screen programs work without a whitelist because Normal mode is passthrough

Block View:

- `j` / Down selects next block
- `k` / Up selects previous block
- `G` jumps to the newest block and restores follow-tail
- `g` jumps to the oldest block and disables follow-tail
- `Enter` enters Detail View
- `q` / `Esc` returns to Plain View

Fast repeated `j` / `k` input should be coalesced before viewport adjustment and render. The viewport should only move when the selected block leaves the visible range or crosses the configured scroll margin.

Detail View:

- `q` / `Esc` returns to Block View
