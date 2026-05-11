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
- `BlockViewport.line_offset` controls the first visible visual line in the complete Block View layout.
- `BlockViewport.scroll_offset` is a deprecated compatibility field from the old block-index viewport model.
- `BlockViewConfig.preview_lines` controls collapsed output height.
- `BlockViewConfig.expanded_lines` controls expanded output height.
- `BlockViewConfig.block_gap` controls blank visual lines between blocks.
- `BlockViewConfig.scroll_margin_lines` keeps navigation from pinning the selected block to the edge.
- `BlockViewConfig.scroll_margin_blocks` is legacy and should not drive new viewport logic.
- `BlockViewConfig.auto_follow_on_reach_bottom` controls whether pressing `j` onto the newest block re-enters Tail anchor (default `false`).
- `BlockViewConfig.horizontal_margin` keeps borders away from terminal edges.
- `BlockViewConfig.body_padding` controls inner body padding.
- `BlockViewConfig.show_footer` reserves a compact shortcut footer.

Compositor first builds a complete `VisualLayout`:

```rust
pub struct VisualLayout {
    pub lines: Vec<VisualLine>,
    pub spans: Vec<BlockVisualSpan>,
    pub total_height: usize,
}
```

Each `BlockVisualSpan` records a block's `[start_line, end_line)` range inside that complete visual document. The viewport then slices `layout.lines` by `line_offset`, so the top and bottom of the screen may show partial non-selected blocks. This is intentional: the viewport scrolls by visual lines, while selection still moves by block.

For each block in the full layout:

```text
insert VisualLine::BlockTopBorder
insert VisualLine::BlockBodyLine values for lines belonging to the block
insert VisualLine::BlockBottomBorder
```

The top border should stay compact:

- block id
- command
- failed marker `✗` when applicable
- running marker `…` when applicable

The bottom border should stay compact:

- status
- exit code
- duration

The selected block should be visibly highlighted without reversing the whole body. Use `╭ ╮ ╰ ╯` and border/label emphasis; keep body text normally readable.

The selected block must be fully visible whenever its visual height fits in the content area. If navigation would leave the selected block partially clipped, `ensure_selected_block_fully_visible` adjusts `line_offset` before rendering. Partial blocks above or below the selected block are allowed and are not a separate data state.

Collapsed blocks show at most `preview_lines` body lines. If more output exists, append:

```text
... N more lines, Enter to expand
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
- status (merged with exit code)
- duration

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
┌─ [8] · yazi ─────────────────────────────────────┐
│ no captured text output                          │
└─ [8] · 󰄬 ok · 1m32s ────────────────────────────┘
```

Selected example:

```text
% › vim src/main.rs
╭─ [12] · vim src/main.rs ──────────────────────────╮
│ no captured text output                            │
╰─ [12] · 󰄬 ok · 2m14s ────────────────────────────╯
```

Detail View for a block without captured text still shows execution metadata:

```text
󰋼 Detail
󰘧 command: yazi
󰉋 cwd: ~/Projects/demo
󰄬 status: ok
󰔟 duration: 1m32s
```

## Selection And Expansion State

Selection and expansion are view state, not block data.

Store these in `ViewState`:

- `view: ViewKind`
- `selected_block: Option<BlockId>`
- `expanded_block: Option<BlockId>`
- `scroll_offset: usize` legacy top-level field
- `block_viewport: BlockViewport`
- `detail_line_cursor: usize`
- `filter: BlockFilter`
- `visible: VisibleSource`
- `search_buffer: Option<String>`
- `pre_search_query: String`
- `help: Option<HelpState>`
- `confirm: Option<ConfirmState>`
- `visual_anchor: Option<BlockId>`
- `detail_visual_anchor: Option<usize>`

Store block viewport state in `BlockViewport`:

- `selected_index: usize`
- `line_offset: usize`
- `scroll_offset: usize` deprecated block-index offset
- `anchor: ViewAnchor`

`ViewAnchor::Tail` follows the end of the visual layout. `ViewAnchor::Manual` preserves the current `line_offset` unless the selected block would become partial. `ViewAnchor::Top` is used after `g`.

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
┌─ [1] · ls ───────────────────────────────────────┐
│ api_tracker  Documents  Downloads  Projects       │
└─ [1] · 󰄬 ok · 28ms ──────────────────────────────┘
```

### Selected Block View

```text
% › cargo build
╭─ [2] · cargo build ──────────────────────────────╮
│ error[E0432]: unresolved import `foo`            │
╰─ [2] · 󰅙 fail · exit 101 · 2.3s ────────────────╯
```

### Detail View

```text
% › cargo build
╭─ [2] · cargo build ──────────────────────────────╮
│ error[E0432]: unresolved import `foo`            │
│                                                    │
│ 󰋼 Detail ──────────────────────────────────────── │
│ 󰘧 command: cargo build                            │
│ 󰉋 cwd: ~/Projects/demo                            │
│ 󰅙 status: fail · exit 101                         │
│ 󰔟 duration: 2.3s                                   │
╰─ [2] · 󰅙 fail · exit 101 · 2.3s ────────────────╯
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
- `Enter` expands / collapses the selected block
- `i` enters Detail View for the selected block
- `c` copies the command text
- `o` copies the output text
- `y` copies both command and output text
- `v` toggles visual line selection mode (anchors at cursor block)
- `d` deletes the selected block(s) with a confirmation dialog
- `r` reruns the command (single block) or shows confirmation (multi-block visual range)
- `f` toggles the failed-only filter
- `/` opens the search bar for live substring filtering
- `n` / `N` jumps to the next / previous search result
- `?` opens the Help overlay
- `Esc` / `q` when in visual mode exits visual mode first; second press leaves Block View
- `Ctrl-u` / `Ctrl-d` scrolls half a screen up / down
- `Ctrl-b` / `Ctrl-f` scrolls a full screen up / down

Fast repeated `j` / `k` input is accumulated via `InputAccumulator.pending_block_delta` and flushed at frame cadence (16ms `FRAME_DURATION`). The accumulated delta is clamped to `[-limit, limit]` where `limit = min(blocks.len(), 500)` to prevent unbounded growth. Navigation at block boundaries is a no-op only when there is no pending delta; accumulated delta is clamped on flush.

View mode switches (enter Block View, return to Plain, enter/exit Detail, g/G jumps) set `RenderState.force_render = true`, which bypasses frame-rate limiting and forces an immediate redraw, preventing stale screen artifacts.

`BlockViewConfig.auto_follow_on_reach_bottom` (default `false`) gates whether a `j`-driven arrival at the newest block changes the anchor to `Tail`. When `false` (the default), the anchor stays `Manual` — only the explicit `G` key re-enters Tail mode.

Detail View:

- `j` / `k` scroll the output lines
- `g` / `G` jump to top / bottom
- `c` copies the command text
- `o` copies output text (respects visual line selection when active)
- `y` copies both command and output
- `v` / `V` toggles visual line selection
- `r` reruns the command
- `?` opens the Help overlay
- `q` / `Esc` returns to Block View

## Clipboard Copy

The copy system uses `CopyPart` and `CopyFormat` from `format.rs`:

```rust
pub enum CopyPart { Command, Output, Both }

pub enum CopyFormat { Plaintext, Markdown, ShellTranscript, Json }

pub fn format_blocks(blocks: &[&CommandBlock], part: CopyPart, fmt: CopyFormat) -> String;
```

- `c` copies command text via `CopyPart::Command`
- `o` copies output text via `CopyPart::Output`
- `y` copies both via `CopyPart::Both`
- In Block View, visual range copies all blocks in the selection
- In Detail View, `o` respects `detail_visual_anchor` — copies only the selected range of output lines
- `CopyFormat` is configurable via `[block_view] copy_format = "markdown"` in `tide.toml`

## Footer

Block View footer shows `Keybindings: ?` by default. During a live search it shows the search buffer with `/` prefix and apply/cancel hints. When a filter is active it shows the filter tags. Flash messages (e.g. `"copied command"`) temporarily replace the footer for ~1.5 seconds.

Detail View footer shows `cursor/total` when output exceeds the visible area, followed by `Keybindings: ?`.

The footer is rendered from `FooterSegment` values (Label, Key, Plain, Spacer, Sep). `show_footer` config controls whether it is reserved.
