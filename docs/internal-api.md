# Internal API

This document describes Tide's internal Rust interfaces. It is not an HTTP API.

The exact implementation can evolve, but code should preserve these ownership boundaries:

- `ShellBuffer` stores shell text.
- `BlockStore` stores structured execution data.
- `ViewState` stores selected / expanded / current view state, search state, help/confirm overlays, visual selection anchors, and filtering.
- `Compositor` builds visual lines and computes visible ranges.
- `Renderer` draws visual lines using theme colors (Catppuccin Frappe).
- `Osc777Parser` strips shell markers and emits lifecycle events.
- `ansi::parse_ansi_lines` parses raw PTY bytes into per-line styled spans.
- `BlockIndex` provides incremental indexes (failed blocks, token inverted) for filtering/search.

## RuntimeState

```rust
struct RuntimeState {
    shell: ShellBuffer,
    blocks: BlockStore,
    view: ViewState,
    input_accumulator: InputAccumulator,
    render_state: RenderState,
    config: RuntimeConfig,
    capture_suspended: bool,
    rows: u16,
    cols: u16,
    index: BlockIndex,
}
```

The central mutable state. Wrapped in `Arc<Mutex<RuntimeState>>` and shared between the input thread (mutates view state, triggers render) and the output thread (updates shell/block state from PTY output, renders).

## BlockId

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u64);
```

`BlockId` identifies one execution block. Display via `{}` prints the inner `u64`. In the UI, block IDs are formatted as `[N]` instead of `#N`.

## ShellBuffer

```rust
#[derive(Debug, Clone, Default)]
pub struct ShellBuffer {
    pub lines: Vec<ShellLine>,
    current_line: String,
    current_col: usize,
    current_block_id: Option<BlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellLine {
    pub text: String,
    pub block_id: Option<BlockId>,
}
```

### Methods

```rust
impl ShellBuffer {
    pub fn new() -> Self;
    pub fn append(&mut self, bytes: &[u8], block_id: Option<BlockId>);
    pub fn line_count(&self) -> usize;
    pub fn snapshot(&self) -> Vec<ShellLine>;
    pub fn cursor_position(&self) -> (usize, usize);
}
```

`append` handles:
- ANSI escape sequences (CSI cursor movement, erase in display/line, OSC strings)
- Carriage return (`\r` → col 0)
- Newline (`\n` → push current line)
- Backspace (`\x08` → remove preceding char)
- Tab (→ 4 spaces)
- Control characters (silently dropped)

Block borders, metadata, detail text, selected state, and expanded state are not part of `ShellBuffer`.

## CommandBlock (ExecutionBlock)

```rust
#[derive(Debug, Clone)]
pub struct CommandBlock {
    pub id: BlockId,
    pub command: String,
    pub cwd: PathBuf,
    pub started_at: SystemTime,
    pub finished_at: Option<SystemTime>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub output_raw: Vec<u8>,
    pub output_text: String,
    pub kind: BlockKind,
    pub status: BlockStatus,
    pub git_context: Option<GitContext>,
    pub suggestions: Vec<SuggestedAction>,
    pub start_line: usize,
    pub end_line: usize,
    pub output_truncated: bool,
}
```

`start_line` and `end_line` describe where the block appears in `ShellBuffer` (indices into `ShellBuffer.lines`).

`output_raw` stores raw bytes captured during command execution. `output_text` is derived from `output_raw` by stripping ANSI escape sequences (via `strip_ansi_escapes`).

`output_truncated` is set `true` when the accumulated output exceeds `max_output_bytes_per_block`. Once set, further output is silently dropped for that block.

## BlockKind

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    NormalCommand,
    FailedCommand,
    TuiSession,
    RawProgram,
    AiGenerated,
    SystemEvent,
}
```

`RawProgram` is set when an alternate-screen switch is detected during command execution. `FailedCommand` is promoted from `NormalCommand` when the exit code is non-zero. `AiGenerated` and `SystemEvent` are future/reserved.

## BlockStatus

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockStatus {
    Running,
    Success,
    Failed,
    Interrupted,
    Unknown,
}
```

Set to `Running` on `start_command`. Set to `Success` or `Failed` on `finish_command` based on exit code.

## BlockStore

```rust
#[derive(Debug)]
pub struct BlockStore {
    next_id: u64,
    pub timeline: Vec<BlockId>,
    pub executions: HashMap<BlockId, CommandBlock>,
    pub max_blocks: Option<usize>,
    active_block_id: Option<BlockId>,
    current_cwd: PathBuf,
    max_output_bytes_per_block: usize,
}
```

### Methods

```rust
impl BlockStore {
    pub fn new(current_cwd: PathBuf, max_blocks: Option<usize>, max_output_bytes_per_block: usize) -> Self;

    pub fn start_command(&mut self, command: String, start_line: usize, kind: BlockKind) -> BlockId;
    pub fn append_output(&mut self, bytes: &[u8]);
    pub fn finish_command(&mut self, exit_code: i32, end_line: usize);
    pub fn active_block_id(&self) -> Option<BlockId>;
    pub fn block(&self, id: BlockId) -> Option<&CommandBlock>;
    pub fn block_mut(&mut self, id: BlockId) -> Option<&mut CommandBlock>;
    pub fn block_id_at(&self, index: usize) -> Option<BlockId>;
    pub fn remove(&mut self, id: BlockId);
    pub fn next_block(&self, id: BlockId) -> Option<BlockId>;
    pub fn prev_block(&self, id: BlockId) -> Option<BlockId>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn set_cwd(&mut self, cwd: String);
}
```

`timeline` preserves execution order as `Vec<BlockId>`. `executions` provides lookup by id via `HashMap`.

`max_blocks` controls retention only. When the timeline exceeds `max_blocks`, the oldest block is evicted. `max_blocks = None` means unbounded history.

`append_output` is capped at `max_output_bytes_per_block`. Once the cap is reached, `output_truncated` is set and further output is silently dropped.

`finish_command` converts `output_raw` to `output_text` by stripping ANSI escapes via `strip_ansi_escapes`.

`remove` deletes a block from both `timeline` and `executions`. `next_block` and `prev_block` navigate relative to a given `BlockId` in the `timeline`.

## RuntimeConfig

```rust
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub block_layout: BlockLayoutConfig,
    pub block_view: BlockViewConfig,
    pub max_blocks: Option<usize>,
}
```

Built from the user-facing `Config` struct via `build_runtime_config`, which extracts only the rendering-relevant fields and discards raw_programs etc.

## BlockAction

```rust
#[derive(Debug, Clone)]
pub enum BlockAction {
    CopyCommand,
    CopyOutput,
    CopyBlock,
    RerunCommand,
    ExplainOutput,
    ExplainError,
    GenerateFixCommand,
    SummarizeBlock,
    CollapseBlock,
    ExpandBlock,
    SaveBlock,
    DeleteFromSessionView,
    CreateNote,
    InspectGitChanges,
    InsertSuggestedCommand(String),
}
```

Dispatched by `perform_block_action()` in the PTY input handler. `CopyCommand`, `CopyOutput`, and `CopyBlock` are wired to clipboard via `format_blocks()` and `write_to_clipboard()`. Block View uses `copy_blocks()` which calls `format_blocks()` directly. Detail View uses `detail_copy_output()` for output (respecting visual line selection) and `format_blocks()` for command/combined. The remaining variants are future/reserved for AI integration.

## ViewKind

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Plain,
    Blocks,
    Detail,
    Help,
    Agent,
    RawProgram,
}
```

`Agent` and `RawProgram` are future-facing/reserved. Current Normal passthrough uses `ViewKind::Plain`.

Detail View is a full-screen pager mode for deep inspection of a single block, entered via `i` from Block View (not Enter; Enter toggles inline block expansion).

`Help` is an overlay mode that renders a keybinding reference over the current Block or Detail view. Entered via `?`, exited via `?`, `q`, or `Esc`. Within Help, `j`/`k`/`g`/`G` navigate the help entries; `q`/`?`/`Esc` return to the previous view.

## ViewState

```rust
#[derive(Debug, Clone)]
pub struct ViewState {
    pub view: ViewKind,
    pub selected_block: Option<BlockId>,
    pub expanded_block: Option<BlockId>,
    pub scroll_offset: usize,
    pub block_viewport: BlockViewport,
    pub detail_line_cursor: usize,
    pub filter: BlockFilter,
    pub visible: VisibleSource,
    pub search_buffer: Option<String>,
    pub pre_search_query: String,
    pub help: Option<HelpState>,
    pub confirm: Option<ConfirmState>,
    pub visual_anchor: Option<BlockId>,
    pub detail_visual_anchor: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct BlockViewport {
    pub selected_index: usize,
    pub line_offset: usize,
    /// Deprecated: old block-index offset. New rendering uses line_offset.
    pub scroll_offset: usize,
    pub anchor: ViewAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAnchor {
    Top,
    Tail,
    Manual,
}

#[derive(Debug, Clone)]
pub struct BlockFilter {
    pub failed_only: bool,
    pub command_query: String,  // empty = inactive
}

impl BlockFilter {
    pub fn is_active(&self) -> bool {
        self.failed_only || !self.command_query.is_empty()
    }
}

/// The set of BlockIds currently visible in Block View.
/// Navigation always uses this; never indexes into BlockStore.timeline directly.
#[derive(Debug, Clone)]
pub enum VisibleSource {
    AllTimeline,
    Filtered(Vec<BlockId>),
}
```

`selected_block` and `expanded_block` belong to `ViewState`.

`detail_line_cursor` is a 0-indexed cursor line within Detail View output, used by the full-screen pager when `ViewKind::Detail` is active.

`filter` controls which blocks are visible (failed-only toggle, command query string).

`visible` is the current visible set — either `AllTimeline` (no filter) or `Filtered(Vec<BlockId>)` (pre-computed intersection of active filters). All navigation functions use `view.visible.ids(blocks)` instead of `blocks.timeline` directly.

`search_buffer` is `Some(String)` while the user is typing a search query in the inline search bar; `None` otherwise.

`pre_search_query` saves the `filter.command_query` before the search bar opens, so `Esc` can restore it.

`help` is `Some(HelpState)` while the Help overlay is open; `None` otherwise.

`confirm` is `Some(ConfirmState)` while a confirmation dialog (delete, rerun) is open; `None` otherwise.

`visual_anchor` is the anchor `BlockId` for Block View visual selection mode (v mode). `None` = not in visual mode. When set, the range from this block through `selected_block` is highlighted with `VISUAL_BORDER_FG` / `VISUAL_LINE_BG`.

`detail_visual_anchor` is the anchor line index for Detail View visual line selection (v/V mode). `None` = not in visual mode. When set, the range from this line through `detail_line_cursor` is highlighted with `VISUAL_LINE_BG`.

`BlockViewport` controls which portion of block history is visible. It does not store block content.

- `line_offset` is the primary viewport offset. It is the first visible visual line in the full `VisualLayout`.
- `scroll_offset` is deprecated compatibility from the old block-index viewport model.
- `ViewAnchor::Tail` follows the end of the visual layout; new blocks shift the viewport only while Tail is active.
- `ViewAnchor::Top` displays from the first visual line.
- `ViewAnchor::Manual` preserves the current `line_offset` unless the selected block would become partially clipped.

Default for both `ViewState` and `BlockViewport` is `Plain` view + `Tail` anchor.

## HelpState

```rust
#[derive(Debug, Clone)]
pub struct HelpState {
    pub cursor: usize,
    pub scroll: usize,
    /// The view that was active when Help was opened; restored on close.
    pub return_view: ViewKind,
    /// Set to true after the underlying view has been rendered once with
    /// selection suppressed. While false, render() does a full underlying
    /// render + overlay. Once true, only the overlay is redrawn (no flicker).
    pub underlying_rendered: bool,
}

impl HelpState {
    pub fn open(return_view: ViewKind) -> Self;
}
```

The Help overlay displays either `BLOCK_HELP_ENTRIES` or `DETAIL_HELP_ENTRIES` (from `renderer.rs`) in a centered floating box. Navigation: `j`/`k` cursor movement, `g`/`G` jump to start/end, `?`/`q`/`Esc` close and restore `return_view`.

## ConfirmState / ConfirmKind

```rust
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub kind: ConfirmKind,
    /// All block ids this action covers. Always at least one element.
    pub block_ids: Vec<BlockId>,
}

impl ConfirmState {
    pub fn single(kind: ConfirmKind, id: BlockId) -> Self;
    pub fn multi(kind: ConfirmKind, ids: Vec<BlockId>) -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmKind {
    DeleteBlock,
    DeleteBlocks,
    RerunBlocks,
}
```

The confirm dialog is drawn as a centered floating box with the action description, a warning, and `[Y]es` / `(N)o` prompt. `y`/`Y`/`Enter` confirms; any other key dismisses (no-op). Drawn via `render_confirm_overlay()` in `renderer.rs`.

## InputMode

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Shell,
    BlockNav,
    DetailNav,
    NaturalLanguage,
    OpenCode,
    RawProgram,
}
```

`InputMode` is defined in `app.rs` but currently unused in the main loop; view-mode dispatch is done directly on `ViewKind` in `handle_view_key_sequence`.

## InputAccumulator

```rust
#[derive(Debug, Clone, Default)]
pub struct InputAccumulator {
    pub pending_block_delta: isize,
    pub last_input_at: Option<Instant>,
}
```

Accumulates `j`/`k` delta for frame-rate-limited flush. The delta is clamped to `[-limit, limit]` where `limit = min(blocks.len(), 500)` to prevent unbounded growth during rapid keypresses.

## RenderState

```rust
#[derive(Debug, Clone)]
pub struct RenderState {
    pub dirty: bool,
    pub force_render: bool,
    pub last_render_at: Instant,
    pub needs_cleanup: bool,
    pub flash_message: Option<(String, Instant)>,
    pub last_rendered_rows: usize,
    pub pending_paste: Option<String>,
}
```

`dirty` is set `true` whenever navigation or view state changes. `force_render` is set `true` on view mode switches to bypass frame-rate limiting and guarantee an immediate redraw.

`needs_cleanup` is set instead of `dirty`/`force_render` when leaving Block/Detail view (q/Esc). The input thread's cleanup handler leaves the alternate screen, resets SGR, and shows the cursor — without rendering Plain view through the normal render path. `flush_render_state` returns early when `needs_cleanup` is set.

`flash_message` holds a transient message (e.g. "copied output") and its `Instant`. The compositor reads it and shows it in the footer for 1500ms before expiring it.

`last_rendered_rows` tracks the number of rows drawn in the previous frame, used by the renderer to clear stale tail lines when the new frame is shorter.

`pending_paste` is set by the `r` (rerun) key handler. After the alt-screen cleanup, the input loop extracts it and writes the command text into the PTY, effectively pasting the command for re-execution.

## VisualLine

```rust
#[derive(Debug, Clone)]
pub enum VisualLine {
    Empty,
    ShellText { text: String, block_id: Option<BlockId> },
    BlockBodyLine { text: String, block_id: BlockId, selected: bool, in_visual: bool },
    BlockTopBorder { block_id: BlockId, selected: bool, in_visual: bool, label: TopLabel, match_query: String },
    BlockBottomBorder { block_id: BlockId, selected: bool, in_visual: bool, label: String },
    BlockDetailLine { block_id: BlockId, text: String, selected: bool, in_visual: bool, in_detail_view: bool },
    DetailTopBorder { block_id: BlockId, label: String },
    DetailBottomBorder { block_id: BlockId, label: String },
    StyledDetailBodyLine { block_id: BlockId, styled: StyledText, plain_text: String, is_cursor: bool, is_visual: bool },
    StyledBlockBodyLine { block_id: BlockId, styled: StyledText, plain_text: String, selected: bool, in_visual: bool },
    Footer { segments: Vec<FooterSegment> },
}
```

Block borders and details are visual lines. They are not stored in `ShellBuffer`.

`selected: bool` on borders, detail lines, and styled body lines highlights the entire selected block (LAVENDER border, no body background fill).

`in_visual: bool` indicates the block is within the visual selection range. When true, the renderer uses `VISUAL_BORDER_FG` (YELLOW) for borders instead of LAVENDER/SURFACE2, and `VISUAL_LINE_BG` for body backgrounds.

`BlockTopBorder.label` is a `TopLabel` struct (from `format.rs`) containing structured fields (`id_marker`, `command`, `cwd`, `status`) for styled rendering of the top border label with per-segment foreground colors, status-based command coloring, and search match highlighting.

`match_query: String` in `BlockTopBorder` carries the active search query (from `search_buffer` or `filter.command_query`). The renderer uses `search_tokens()` / `highlight_spans()` to highlight matching substrings in the command text with `SEARCH_MATCH_FG` (YELLOW).

`BlockDetailLine.in_detail_view: bool` controls border color behavior — when `true`, the renderer uses `DETAIL_BORDER_FG` (LAVENDER) with no background; when `false` (inline expansion in Block View), it follows the block selection style.

Detail View uses the `Detail*` variants: borders use `DETAIL_BORDER_FG` (LAVENDER) without background highlight. `StyledDetailBodyLine` carries ANSI-colored content (`StyledText` spans) with `is_cursor: bool` for the active cursor line and `is_visual: bool` for visual line selection highlighting. When `is_cursor` is true, the line gets `CURSOR_BG` background; when `is_visual` is true, it gets `VISUAL_LINE_BG` background.

Footer has `segments: Vec<FooterSegment>` instead of a plain `String`. Each segment is typed (`Label`, `Key`, `Sep`, `Plain`, `Spacer`) and rendered with distinct foreground colors.

## FooterSegment

```rust
#[derive(Debug, Clone)]
pub enum FooterSegment {
    Label(String),
    Key(String),
    Sep,
    Plain(String),
    Spacer,
}

impl FooterSegment {
    pub fn flatten(segments: &[FooterSegment]) -> String;
}
```

`FooterSegment` moved from `compositor.rs` to `app.rs`. `Spacer` fills remaining width with spaces before the next segment group. `Key` segments are rendered with `FOOTER_KEY_FG`, `Sep` with `FOOTER_SEP_FG`, `Label`/`Plain` with `FOOTER_FG`.

## VisibleBlockRange

```rust
#[derive(Debug, Clone, Copy)]
pub struct VisibleBlockRange {
    pub start: usize,
    pub end: usize,
    pub top_padding_lines: usize,
}
```

Returned by `Compositor::compute_visible_range`. `start` and `end` are inclusive block indices. `top_padding_lines` is the number of `VisualLine::Empty` lines the compositor will insert at the top for bottom-alignment (non-Top anchors).

`VisibleBlockRange` is retained for tests and diagnostics. The render path uses `VisualLayout + line_offset` directly.

## VisualLayout

```rust
#[derive(Debug, Clone)]
pub struct VisualLayout {
    pub lines: Vec<VisualLine>,
    pub spans: Vec<BlockVisualSpan>,
    pub total_height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockVisualSpan {
    pub block_id: BlockId,
    pub block_index: usize,
    pub start_line: usize,
    pub end_line: usize,
}
```

`VisualLayout` is the single layout source for Block View. It contains the complete visual document plus block spans with exclusive `end_line` values. The viewport slices `layout.lines[line_offset..line_offset + content_height]`, allowing partial non-selected blocks at the top or bottom. Selection still moves by block index, and the viewport is adjusted so the selected block is fully visible when possible.

Detail View does not use `VisualLayout` — it generates its own full-screen layout directly via `Compositor::build_detail_lines()`.

## BlockSelectionStyle

```rust
struct BlockSelectionStyle {
    border_fg: Color,
    body_bg: Option<Color>,
    text_fg: Color,
}

impl BlockSelectionStyle {
    fn selected() -> Self;   // BORDER_SELECTED_FG (LAVENDER), no bg, TEXT
    fn normal() -> Self;     // BORDER_NORMAL_FG (SURFACE2), no bg, SUBTEXT1
    fn visual() -> Self;     // VISUAL_BORDER_FG (YELLOW), no bg, SUBTEXT1
    fn from_bool(selected: bool) -> Self;
    fn from_state(selected: bool, in_visual: bool) -> Self;
}
```

Centralised in `renderer.rs`. All Group-A render functions take this instead of a bare `selected: bool`. `from_state` prioritises visual range over selection — when `in_visual` is true, visual() wins even for the cursor block.

## Compositor API

```rust
pub struct Compositor;

impl Compositor {
    pub fn build_visual_lines(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        width: u16,
        height: u16,
        layout: &BlockLayoutConfig,
        block_view: &BlockViewConfig,
        flash_message: Option<&str>,
        home: Option<&Path>,
    ) -> Vec<VisualLine>;

    pub fn build_visual_layout(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        width: u16,
        block_view: &BlockViewConfig,
        home: Option<&Path>,
    ) -> VisualLayout;

    pub fn slice_visible_lines(
        layout: &VisualLayout,
        view: &ViewState,
        content_height: usize,
    ) -> Vec<VisualLine>;

    pub fn compute_visible_range(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> VisibleBlockRange;

    pub fn compute_tail_scroll_offset(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> usize;

    pub fn compute_scroll_offset_ending_at(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        selected_index: usize,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> usize;
}
```

The compositor is responsible for turning `ShellBuffer + BlockStore + ViewState` into renderable `VisualLine` values, AND is the single source of truth for viewport math (visual layout, visible range, tail line offset). It also contains the Detail View layout builder:

```rust
// Private, called from build_visual_lines when ViewKind::Detail:
fn build_detail_lines(
    _shell: &ShellBuffer,
    blocks: &BlockStore,
    view: &ViewState,
    width: u16,
    height: u16,
    block_view: &BlockViewConfig,
    flash_message: Option<&str>,
    home: Option<&Path>,
) -> Vec<VisualLine>;
```

`build_detail_lines` constructs a full-screen single-block pager with:
- `DetailTopBorder` / `DetailBottomBorder` (LAVENDER foreground, no background highlight)
- `StyledDetailBodyLine` (ANSI-colored output from `parse_ansi_lines`, with `is_cursor: bool` for line cursor highlighting using CURSOR_BG and `is_visual: bool` for visual selection using VISUAL_LINE_BG)
- Metadata lines (`BlockDetailLine`) rendered with semantic coloring: merged "status: fail · exit N" format, Nerd Font icons per field (󰘧 command, 󰉋 cwd, 󰄬/󰅙/󰔟 status, 󰔟 duration, 󰘳 actions)
- Vertical centering of short output
- Auto-scroll support via `block_viewport.line_offset`

Helper functions in `compositor.rs`:

```rust
fn get_block_styled_output_lines(block: &CommandBlock) -> Vec<StyledText>;
fn detail_footer_segments(block: &CommandBlock, view: &ViewState, total_lines: usize, inner_height: usize, flash_message: Option<&str>) -> Vec<FooterSegment>;
fn detail_lines(block: &CommandBlock, selected: bool, in_visual: bool, in_detail_view: bool) -> Vec<VisualLine>;
fn bottom_label(block: &CommandBlock) -> String;
fn footer_segments(blocks: &BlockStore, view: &ViewState, flash_message: Option<&str>) -> Vec<FooterSegment>;
fn format_ago(t: SystemTime) -> String;
```

`get_block_styled_output_lines` extracts the block's output as ANSI-parsed `StyledText` lines from `block.output_raw` via `parse_ansi_lines()`. Returns the appropriate placeholder for `RawProgram` or empty-output blocks.

`detail_lines` builds metadata for a block with merged exit+status line (e.g. "status: fail · exit 1"), empty separator rows, and truncated-output notices.

`bottom_label` builds the bottom border label with Nerd Font icons per status (󰄬 ok, 󰅙 fail · exit N, 󰔟 running/cancelled), duration, truncated marker, and `format_ago` relative timestamp.

`format_ago` returns human-readable relative times ("5s ago", "3m ago", "2h ago").

`detail_footer_segments` builds the Detail View footer with scroll position and keybinding hint.

`footer_segments` builds the Block View footer: search bar UI when typing, flash message when active, filter tags when filter is active, or default keybinding hint.

### Block View composition

Plain View generation (`build_visual_lines` with `ViewKind::Plain`):

```text
ShellBuffer.lines -> VisualLine::ShellText
```

Block View generation (`ViewKind::Blocks`):

```text
for each block (iterating view.visible.ids(blocks)):
  BlockTopBorder (selected: true for selected block, in_visual: true for visual range)
  Body lines:
    - RawProgram → placeholder text
    - output_raw empty + shell_lines exist → plain BlockBodyLine from shell_lines
    - output_raw non-empty → StyledBlockBodyLine from parse_ansi_lines(&block.output_raw)
  (if expanded_block == Some(id): BlockDetailLine values)
  BlockBottomBorder (selected: true for selected block, in_visual: true for visual range)

then slice by BlockViewport.line_offset and content height

Footer:
  - search bar when typing: "/query▌ | Apply: Enter | Cancel: Esc"
  - flash message when active
  - filter tags when filter is active
  - otherwise: "Keybindings: ?"
```

`expanded_block` is a per-block inline toggle that stays in `ViewKind::Blocks`. When set, the block shows all output lines (capped at `expanded_lines`, default 15) plus detail metadata (command, cwd, status, duration, actions). When set, the selected block is expanded; navigation via `j`/`k`/`g`/`G`/Enter automatically moves the expanded state to the newly selected block.

When a filter is active (`BlockFilter.failed_only` or `block_filter.command_query`), the compositor iterates `view.visible.ids(blocks)` instead of the full `blocks.timeline`. The `VisibleSource` is rebuilt by `rebuild_visible()` in `pty.rs` whenever the filter state changes.

Visual selection: when `view.visual_anchor` is set, the compositor computes a visual range (min/max block indices between anchor and selected_block) before the loop. Each block's `in_visual` flag is set based on this range. The renderer uses `BlockSelectionStyle::visual()` (YELLOW borders) for blocks in the visual range.

### Detail View composition

```text
for the single expanded block:
  DetailTopBorder (LAVENDER fg, no bg)
  Body: StyledDetailBodyLine × N (ANSI-styled via parse_ansi_lines, is_cursor highlights line, is_visual for visual selection)
  Metadata: BlockDetailLine × N (command, cwd, status, duration, actions — Nerd Font icons, semantic colors)
  DetailBottomBorder (LAVENDER fg, no bg)

then pad to fill screen (short output vertically centered)

Footer:
  "N/M | Keybindings: ?" (when output overflows)
  flash message when active
```

Detail View is a full-screen pager entered via `i` from Block View. The cursor (`detail_line_cursor`) moves independently with `j`/`k`; the viewport auto-scrolls when the cursor leaves the visible area. `g`/`G` jump to top/bottom.

Visual line selection: when `view.detail_visual_anchor` is set, lines between the anchor and `detail_line_cursor` get `is_visual = true` and are rendered with `VISUAL_LINE_BG`. Toggled by `v`/`V` in Detail View.

Metadata is rendered by `render_block_detail_line()` in `renderer.rs`, which applies semantic colors with Nerd Font icons:
- "Detail" header: bold MAUVE, 󰋼 icon
- Status (merged exit+status): 󰄬 GREEN (ok), 󰅙 RED (fail · exit N), 󰔟 YELLOW (running)
- CWD: 󰉋 TEAL (icon), BLUE (path)
- Command: 󰘧 BLUE
- Duration: 󰔟 YELLOW
- Actions: 󰘳 LAVENDER, bold MAUVE keys (c 󰆏 copy command, o 󰉆 copy output, y 󰈚 copy both, r 󰑓 rerun)

Height calculations come from the generated visual layout for Block View, or from direct arithmetic for Detail View accounting for metadata line count (`detail_inner_height = rows - 4 - meta_count`).

## Renderer API

```rust
pub fn render<W: Write>(
    w: &mut W,
    visual_lines: &[VisualLine],
    view: &ViewState,
    cursor: Option<(usize, usize)>,
    layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
    rows: u16,
    cols: u16,
    last_rendered_rows: usize,
) -> io::Result<(usize, bool)>;

pub fn enter_block_render<W: Write>(w: &mut W) -> io::Result<()>;

pub fn leave_block_render<W: Write>(w: &mut W) -> io::Result<()>;
```

`render` draws visual lines to the terminal via crossterm. It returns `(rendered_rows, drew_underlying)`.
- `rendered_rows`: number of lines actually rendered
- `drew_underlying`: `true` when the underlying Block/Detail view was re-rendered (as opposed to reusing previous frame's pixels). The caller sets `HelpState::underlying_rendered = true` so subsequent Help navigations can skip the underlying re-render (avoiding full-screen flicker on j/k).

Rendering flow:
1. If `view == Help` and `underlying_rendered` is already true, only redraw the floating Help box.
2. Otherwise, render the visual lines (with selection suppressed for Help's first render), then draw Help or Confirm overlay on top.
3. Draws each visible line, stopping at terminal height.
4. Clears stale tail rows from the previous frame that fall outside the new frame's range (using `last_rendered_rows`).
5. For Plain view, positions the cursor via `MoveTo` and `Show`. For Block/Detail/Help views, hides the cursor.
6. When `view.confirm` is `Some`, draws `render_confirm_overlay` after the main content.
7. Returns the count of rendered rows.

The caller (`render_runtime` in `pty.rs`) stores the returned count in `RenderState.last_rendered_rows` for the next frame.

Crossterm `queue!` is used for batching; all drawing commands are flushed atomically at the end to avoid intermediate blank frames.

### Rendering by variant

All borders, text, and highlights use theme colors from `theme.rs` (Catppuccin Frappe). All borders use round characters (`╭╮╰╯`). No body background fill on any variant.

| Variant | Rendering |
|---------|-----------|
| `BlockTopBorder` (selected) | LAVENDER fg, no bg, full-row width |
| `BlockTopBorder` (normal) | SURFACE2 fg, no bg |
| `BlockTopBorder` (visual) | YELLOW fg (VISUAL_BORDER_FG), no bg |
| `BlockBottomBorder` (selected) | LAVENDER fg, no bg |
| `BlockBottomBorder` (normal) | SURFACE2 fg, no bg |
| `BlockBottomBorder` (visual) | YELLOW fg (VISUAL_BORDER_FG), no bg |
| `StyledBlockBodyLine` | ANSI colors via `render_styled_framed_text()`, border_fg from style |
| `BlockDetailLine` | `render_block_detail_line()` with semantic colors and Nerd Font icons |
| `BlockBodyLine` (plain fallback) | `render_framed_text()` |
| `DetailTopBorder` / `DetailBottomBorder` | DETAIL_BORDER_FG (LAVENDER), no bg |
| `StyledDetailBodyLine` (cursor) | CURSOR_BG background, ANSI colors |
| `StyledDetailBodyLine` (visual) | VISUAL_LINE_BG background, ANSI colors |
| `StyledDetailBodyLine` (no cursor/visual) | ANSI colors via `render_styled_framed_text()` |
| `Footer` | FOOTER_FG foreground, no background fill |

`render_styled_framed_text` is the primary function for ANSI-colored body lines. It iterates `StyledText.spans`, applying per-span `TextStyle` (fg/bg, bold, italic, underline, reverse) via `apply_span_style` and resetting between spans via `reset_span_style`. When a background `bg` is provided, it's applied after the left `│` border (border keeps default bg) and covers the content area between `│` chars.

`render_block_detail_line` handles metadata lines with semantic color mapping and Nerd Font icons. Each known field (command, cwd, status, duration, actions) gets a fixed-width icon+label column (12 cells) with role-specific icon foreground colors.

`render_top_border` renders `BlockTopBorder` with per-segment coloring via `TopLabel` fields. When `match_query` is non-empty, the command text is split into highlighted/normal spans via `search_tokens()` / `highlight_spans()`.

`render_footer` renders `Footer` segments with appropriate foreground colors: `Key` → `FOOTER_KEY_FG`, `Sep` → `FOOTER_SEP_FG`, `Label`/`Plain` → `FOOTER_FG`.

`render_help_overlay` draws a centered floating box with either `BLOCK_HELP_ENTRIES` or `DETAIL_HELP_ENTRIES`. Uses `HELP_BORDER` / `HELP_KEY_FG` / `HELP_TEXT_FG` / `HELP_SEL_BG` / `HELP_SEL_FG` / `HELP_DIM_FG` theme colors.

`render_confirm_overlay` draws a centered floating confirmation box for delete/rerun actions. Uses `HELP_BORDER` / `HELP_KEY_FG` / `HELP_DIM_FG` theme colors.

For Plain View, cursor position is drawn from `ShellBuffer.cursor_position()`. For Block/Detail views, the cursor is hidden.

`enter_block_render` switches to the alternate screen buffer and hides the cursor. Called once when entering Block View (Ctrl-B). Must not be called while holding the `RuntimeState` lock (deadlock risk — output thread locks state → stdout).

`leave_block_render` leaves the alternate screen, resets SGR (`ResetColor`), and shows the cursor. **`LeaveAlternateScreen` must come before `ResetColor`/`Show`** so that SGR and cursor state are applied on the newly-restored main screen, not on the discarded alt screen.

Normal mode should not continuously redraw through the renderer. The renderer is used for reconstructed views (Blocks, Detail) which render in the alternate screen.

For Blocks, the compositor receives terminal height, subtracts footer height, and slices the complete `VisualLayout` by `BlockViewport.line_offset`. For Detail, the compositor generates a full-screen single-block layout. Plain/Normal mode remains transparent passthrough and does not use this block viewport.

## Osc777Parser and ShellHookEvent

```rust
#[derive(Debug, Default)]
pub struct Osc777Parser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPtyPart {
    Visible(Vec<u8>),
    Event(ShellHookEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellHookEvent {
    Preexec { command: String },
    Precmd { exit_code: i32, cwd: Option<String> },
}

impl Osc777Parser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<ParsedPtyPart>;
    pub fn flush_visible(&mut self) -> Vec<u8>;
}
```

`Osc777Parser` splits PTY byte streams into visible bytes and shell lifecycle events. Events are delimited by `\x1b]777;block_` ... `\x07` (OSC 777 sequences).

The parser handles split markers (marker bytes arriving across multiple PTY reads) and does not delay normal output.

## Config

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub shell: ShellConfig,
    pub ui: UiConfig,
    pub blocks: BlocksConfig,
    pub history: HistoryConfig,
    pub block_view: BlockViewConfig,
    pub block_layout: BlockLayoutConfig,
    pub raw_programs: Vec<String>,
    pub tui_apps: BTreeMap<String, TuiAppConfig>,
}

pub struct BlockViewConfig {
    pub preview_lines: usize,
    pub expanded_lines: usize,
    pub follow_tail: bool,
    pub block_gap: usize,
    pub scroll_margin_blocks: usize,
    pub scroll_margin_lines: usize,
    pub auto_follow_on_reach_bottom: bool,
    pub horizontal_margin: usize,
    pub body_padding: usize,
    pub show_footer: bool,
    pub copy_format: CopyFormat,
}

pub struct BlockLayoutConfig {
    pub horizontal_padding: usize,
    pub show_padding_in_plain: bool,
}
```

`copy_format` controls the serialization format for clipboard copy operations. Defaults to `CopyFormat::Plaintext`. Configurable via TOML as `copy_format = "markdown"` (or `"plaintext"`, `"transcript"`, `"json"`).

`raw_programs` may remain as a legacy compatibility field in loaded config, but it must not be required for terminal passthrough. Full-screen programs work without a whitelist because Normal mode is transparent.

If no config file exists, defaults are used.

Defaults:

- `history.max_blocks = 1000`
- `block_view.preview_lines = 4`
- `block_view.expanded_lines = 15`
- `block_view.follow_tail = true`
- `block_view.block_gap = 0`
- `block_view.scroll_margin_lines = 2`
- `block_view.scroll_margin_blocks = 2` legacy compatibility
- `block_view.auto_follow_on_reach_bottom = false`
- `block_view.horizontal_margin = 1`
- `block_view.body_padding = 1`
- `block_view.show_footer = true`
- `block_view.copy_format = "plaintext"`
- `block_layout.horizontal_padding = 1`
- `block_layout.show_padding_in_plain = true`

## Navigation / Input API (pty.rs)

Input processing is handled inline in the PTY input thread via:

```rust
fn handle_view_key_sequence(bytes: &[u8], state: &Arc<Mutex<RuntimeState>>) -> Option<usize>;

fn handle_block_view_byte(byte: u8, state: &mut RuntimeState) -> bool;

fn accumulate_block_delta(state: &mut RuntimeState, delta: isize);

fn flush_navigation_delta(state: &mut RuntimeState) -> bool;

fn select_relative_block(state: &mut RuntimeState, delta: isize) -> bool;

fn select_block_index(state: &mut RuntimeState, index: usize, anchor: ViewAnchor);

fn select_tail_block(state: &mut RuntimeState);

fn enter_block_view(state: &mut RuntimeState);

fn handle_search_input(byte: u8, state: &mut RuntimeState) -> bool;

fn maybe_flush_navigation_and_render(
    state: &Arc<Mutex<RuntimeState>>,
    stdout: &Arc<Mutex<io::Stdout>>,
    wait_for_frame: bool,
) -> io::Result<()>;
```

### Block View key bindings (`handle_block_view_byte`)

| Key | Action |
|-----|--------|
| `j` / `k` | Accumulate navigation delta (rendered at frame cadence via `flush_navigation_delta`) |
| `g` | Jump to oldest block (Top anchor, force render) |
| `G` | Jump to newest block (Tail anchor, force render) |
| `Ctrl-u` / `Ctrl-d` | Scroll half screen up/down |
| `Ctrl-b` / `Ctrl-f` | Scroll full screen up/down |
| `Enter` | Toggle inline block expansion (stays in `ViewKind::Blocks`, force render). If already expanded, collapse; otherwise expand. `expanded_block` follows selection during navigation. |
| `i` | Enter Detail View (switches to `ViewKind::Detail`, resets cursor and line_offset, force render) |
| `c` | Copy command text to clipboard (supports visual range) |
| `o` | Copy output text to clipboard (supports visual range) |
| `y` | Copy command + output to clipboard (supports visual range) |
| `v` | Toggle visual selection mode (anchors at selected block, range extends as cursor moves) |
| `r` | Rerun: saves selected block's command to `pending_paste`, sets `needs_cleanup`, exits to Plain View. For visual range with >1 block, shows confirm dialog. |
| `d` | Delete block (single or visual range, shows confirm dialog) |
| `f` | Toggle failed-only filter |
| `/` | Open inline search bar |
| `n` / `N` | Next / previous search result (cycles through filtered visible blocks) |
| `?` | Open Help overlay |
| `q` / `Esc` | Return to Plain View (resets to default ViewState, force render). When in visual mode, first press exits visual mode. |

### Detail View key bindings (`handle_view_key_sequence`)

| Key | Action |
|-----|--------|
| `j` / ArrowDown | Move cursor down one line. Auto-scrolls `line_offset` when cursor reaches the visible area bottom. |
| `k` / ArrowUp | Move cursor up one line. Auto-scrolls `line_offset` when cursor passes the visible area top. |
| `g` | Jump cursor to first output line, reset `line_offset` to 0 |
| `G` | Jump cursor to last output line, scroll `line_offset` to show it |
| `c` | Copy command text to clipboard (single key, replaces old `yc`) |
| `o` | Copy output text to clipboard (respects visual line selection, replaces old `yo`) |
| `y` | Copy command + output to clipboard (output respects visual line selection, replaces old `yb`) |
| `v` / `V` | Toggle visual line selection (anchors at current cursor line, range extends as cursor moves) |
| `r` | Rerun: saves expanded block's command to `pending_paste`, sets `needs_cleanup` |
| `?` | Open Help overlay |
| `q` / `Esc` | Return to Block View (resets `detail_line_cursor`, `detail_visual_anchor`, and `line_offset` to 0, force render) |

### Help overlay key bindings (`handle_view_key_sequence`)

| Key | Action |
|-----|--------|
| `j` / ArrowDown | Move cursor down one entry (scroll viewport if needed) |
| `k` / ArrowUp | Move cursor up one entry (scroll viewport if needed) |
| `g` | Jump to first entry |
| `G` | Jump to last entry |
| `?` / `q` / `Esc` | Close Help overlay, restore previous view |
| any other key | Close Help overlay |

### Navigation helpers

```rust
fn detail_output_line_count(state: &RuntimeState) -> usize;
fn detail_meta_line_count(state: &RuntimeState) -> usize;
fn detail_inner_height(state: &RuntimeState) -> usize;
```

`detail_output_line_count` returns the number of output lines in the expanded block for Detail View, using `parse_ansi_lines(&block.output_raw)` instead of shell line ranges (returns `1` for `RawProgram` blocks). `detail_meta_line_count` returns the number of metadata lines displayed in the Detail View pager (including `output_truncated` info and RawProgram specifics). `detail_inner_height` returns the available output scroll area: `(state.rows as usize).saturating_sub(4).saturating_sub(meta_count)` (rows minus top margin, top border, metadata, bottom border, footer).

### General flow

`handle_view_key_sequence` dispatches based on current `ViewKind`:
- `Plain` / `RawProgram` → returns `None` (bytes go to PTY), except `Ctrl-B` (0x02) handled in the outer loop
- `Blocks` → returns `None` (all Block View input goes through `handle_block_view_byte`), with confirm dialog intercept
- `Detail` → matches the bindings above directly
- `Help` → matches help-specific bindings (j/k/g/G/?/q/Esc), closes Help on any unrecognized key
- `Agent` → returns `Some(1)`

`accumulate_block_delta` adds to `InputAccumulator.pending_block_delta` and clamps to `[-limit, limit]`. It also checks `is_navigation_boundary_noop` to skip no-op accumulation at boundaries.

`flush_navigation_delta` resets `pending_block_delta` to 0 and calls `select_relative_block`.

`select_block_index` updates `selected_block`, `selected_index`, and the viewport anchor. When `expanded_block.is_some()`, the expanded state follows selection automatically.

`select_tail_block` jumps to the newest block with Tail anchor, and also syncs `expanded_block` when applicable.

`ensure_selected_visible` builds a `VisualLayout` and adjusts `line_offset` so the selected block's span is fully visible within the content area, respecting `scroll_margin_lines`.

`handle_search_input` processes the inline search bar: accumulates characters, backspaces, Enter applies the filter and rebuilds the visible set, Esc restores the pre-search query.

`rebuild_visible` recomputes `ViewState::visible` based on the current filter (failed_only, command_query, or both).

`execute_delete_blocks` removes blocks from `BlockStore` and syncs the visible source and selection.

`maybe_flush_navigation_and_render` is called after each input chunk. If `dirty` is set, it waits up to `FRAME_DURATION` (16ms) before flushing and rendering. If `force_render` is set, the wait is skipped.

The `Ctrl-B` byte (0x02) is handled in the input loop itself (not in `handle_view_key_sequence`): it calls `enter_block_view` and triggers navigation flush + render. `Ctrl-B` is always consumed when the current view is `Plain`, regardless of `active_block_id`.

### Copy logic

```rust
fn copy_blocks(state: &mut RuntimeState, part: CopyPart);
fn detail_copy_output(state: &RuntimeState) -> Option<String>;
fn visual_range_ids(state: &RuntimeState) -> Vec<BlockId>;
fn copy_flash(count: usize, part: CopyPart, fmt: CopyFormat) -> String;
```

`copy_blocks` collects all block IDs in the current visual range (or the single selected block when not in visual mode), calls `format_blocks()` to serialize them, writes to clipboard, and sets a flash message. Exits visual mode after copying.

`detail_copy_output` returns the selected block's output text. When `detail_visual_anchor` is set, only the selected range of lines is returned.

`visual_range_ids` returns `BlockId`s from `visual_anchor` through `selected_block` (timeline order), or the single selected block when not in visual mode.

`copy_flash` builds a human-readable flash message like "copied command", "copied 3 outputs · markdown".

## Rerun Flow

The `r` key handler:
1. Looks up the current block's command text (or shows confirm dialog for multi-block visual range)
2. Resets `ViewState` to defaults
3. Sets `RenderState.needs_cleanup = true`
4. Sets `RenderState.pending_paste = Some(command)`

On the next iteration of the input loop cleanup path:
1. `needs_cleanup` triggers alt-screen exit (leave alternate screen, reset SGR, show cursor)
2. `pending_paste` is extracted and written to the PTY writer
3. The command text appears in the shell as if the user typed it, followed by a newline

Remaining bytes that followed the exit key in the same read chunk are forwarded to the PTY after the cleanup paste (preserving user input that might follow `q`).

## Delete Flow

The `d` key handler:
1. Collects block IDs from `visual_range_ids()` (single or visual range)
2. Sets `view.confirm` to a `ConfirmState` with the appropriate `ConfirmKind`
3. `handle_block_view_byte` intercepts all input while confirm is open
4. `y`/`Y`/`Enter` confirms: `execute_delete_blocks` removes blocks from store, syncs visible source, and clamps selection
5. Any other key dismisses the dialog

## Flash Message Lifecycle

Flash messages (e.g. "copied output") provide transient user feedback for clipboard operations:

1. Set by `perform_block_action`, `copy_blocks`, or `detail_copy_output` on successful clipboard write
2. Read by `render_runtime` from `RenderState.flash_message` at the start of each render
3. Expires after 1500ms (`Duration::from_millis(1500)`) — if `at.elapsed() >= 1500ms`, the flash is cleared
4. Passed to the compositor as `flash_message: Option<&str>`
5. The compositor renders it in the footer via `footer_segments()` (Block View) or `detail_footer_segments()` (Detail View)

## Render Loop (pty.rs)

The output thread reads PTY output, dispatches to `Osc777Parser` for marker stripping, updates `ShellBuffer` and `BlockStore`, and renders if in Block/Detail view (but skips render when Help/Confirm overlay is showing to avoid flicker).

The input thread reads stdin, applies `handle_view_key_sequence` for view-mode keys, forwards remaining bytes to the PTY writer, and calls `maybe_flush_navigation_and_render` for frame-rate-limited redraws.

The central render dispatch function:

```rust
fn render_runtime(state: &Arc<Mutex<RuntimeState>>, stdout: &Arc<Mutex<io::Stdout>>) -> io::Result<()>;
```

1. Locks `RuntimeState`
2. Checks/expires `flash_message`
3. Calls `Compositor::build_visual_lines()` (which dispatches to `build_block_lines` for Block View, `build_detail_lines` for Detail View, shell snapshot for Plain, or Help-specific code)
4. Unlocks state, passes the visual lines to `renderer::render()` along with `last_rendered_rows`
5. Re-locks state and stores `rendered` count into `last_rendered_rows`
6. Tracks `drew_underlying` for Help overlay flicker suppression

Frame rate is controlled by `FRAME_DURATION` (16ms ≈ 60fps). View mode switches use `force_render` to bypass the frame timer.

## Help Overlay

The Help overlay (`render_help_overlay`) is a floating centered box drawn on top of the underlying Block/Detail view. It shows keybinding entries from either `BLOCK_HELP_ENTRIES` or `DETAIL_HELP_ENTRIES`.

```rust
pub struct HelpEntry {
    pub key: &'static str,
    pub desc: &'static str,
}

pub const BLOCK_HELP_ENTRIES: &[HelpEntry] = &[
    "j / k" → "navigate blocks",
    "Ctrl-u / Ctrl-d" → "scroll half screen",
    "Ctrl-b / Ctrl-f" → "scroll full screen",
    "g / G" → "top / bottom",
    "Enter" → "expand / collapse",
    "i" → "detail view",
    "v" → "visual select mode",
    "/" → "search commands",
    "n / N" → "next / prev result",
    "f" → "toggle failed filter",
    "c" → "copy command",
    "o" → "copy output",
    "y" → "copy command + output",
    "r" → "rerun command",
    "d" → "delete block",
    "?" → "close help",
    "q / Esc" → "return to shell",
];

pub const DETAIL_HELP_ENTRIES: &[HelpEntry] = &[
    "j / k" → "scroll output",
    "g / G" → "top / bottom",
    "v / V" → "visual line select",
    "c" → "copy command",
    "o" → "copy output / selection",
    "y" → "copy command + output",
    "r" → "rerun command",
    "?" → "close help",
    "q / Esc" → "back to blocks",
];
```

The overlay is managed by `HelpState` and rendered with `underlying_rendered` flag to avoid full-screen flicker during j/k navigation within the help box itself.

## Confirm Dialog

The confirm dialog (`render_confirm_overlay`) is a floating centered box for destructive actions (delete, multi-block rerun). It uses `ConfirmState` / `ConfirmKind`:

```rust
ConfirmKind::DeleteBlock  → "Delete block [N]?"
ConfirmKind::DeleteBlocks → "Delete [N] blocks?"
ConfirmKind::RerunBlocks  → "Rerun [N] commands?"
```

Displayed with a warning line ("This cannot be undone.") and action prompt ("[Y]es  (N)o").

## Clipboard and Copy

```rust
fn write_to_clipboard(text: &str) -> bool;
fn perform_block_action(state: &mut RuntimeState, action: BlockAction);
```

`write_to_clipboard` uses `pbcopy` on macOS (via `std::process::Command`) and `arboard` on other platforms. Returns `false` on failure (e.g. headless CI, no clipboard available).

`perform_block_action` looks up the current block, maps `BlockAction` to `CopyPart`, calls `format_blocks()` to serialize, writes to clipboard, and sets a flash message.

## Format Module (src/format.rs)

```rust
pub fn compact_command(command: &str, max_width: usize) -> String;
pub fn compact_cwd(path: &Path, home: Option<&Path>, max_width: usize) -> String;
pub fn build_top_label(block: &CommandBlock, home: Option<&Path>, available_width: usize) -> String;
pub fn build_top_label_parts(block: &CommandBlock, home: Option<&Path>, available_width: usize) -> TopLabel;
pub fn truncate_str(s: &str, max_width: usize) -> String;
pub fn format_blocks(blocks: &[&CommandBlock], part: CopyPart, fmt: CopyFormat) -> String;

#[derive(Debug, Clone)]
pub struct TopLabel {
    pub id_marker: String,
    pub command: String,
    pub cwd: Option<String>,
    pub status: BlockStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyPart {
    Command,
    Output,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyFormat {
    #[default]
    Plaintext,
    Markdown,
    ShellTranscript,
    Json,
}
```

`compact_command` strips ANSI escapes, normalizes whitespace, and right-truncates with `…` using unicode-aware width.

`compact_cwd` substitutes `~` for home, middle-compresses long paths (keeping 2 tail components, falling back to 1), and right-truncates as last resort.

`build_top_label_parts` constructs the structured `TopLabel` for top border labels. Format `[N] <marker> <command>  <cwd>` with 4-level graceful degradation as available width shrinks. `build_top_label` is the flat string variant kept for tests and Detail View.

`truncate_str` unicode-aware truncation without ellipsis.

`format_blocks` serializes one or more blocks into a string for clipboard copy. Supports multiple formats:
- **Plaintext**: command, output, or both with `\n\n---\n\n` multi-block separator
- **Markdown**: command in backticks, output in fenced code blocks, both as `## heading + code block`
- **ShellTranscript**: command prefixed with `$ `
- **Json**: `{"command":...,"output":...}` or `[{...},{...}]` for multi-block

`CopyFormat` is configurable via `block_view.copy_format` in the TOML config.

## BlockIndex (src/index.rs)

```rust
pub struct BlockIndex {
    pub failed: Vec<BlockId>,
    pub tokens: HashMap<String, Vec<BlockId>>,
}

impl BlockIndex {
    pub fn new() -> Self;
    pub fn on_block_failed(&mut self, id: BlockId);
    pub fn query_failed(&self, executions: &HashMap<BlockId, CommandBlock>) -> Vec<BlockId>;
    pub fn index_command(&mut self, id: BlockId, command: &str);
    pub fn query_command(&self, query: &str, executions: &HashMap<BlockId, CommandBlock>) -> Vec<BlockId>;
}
```

`failed` is an incremental list of failed block IDs maintained by `on_block_failed()`, called from `finish_command` in the shell hook handler. `query_failed` tombstone-filters via `executions.contains_key()`.

`tokens` is a token inverted index built by `index_command()`, called at block start (preexec). Commands are ANSI-stripped, lowercased, and tokenized by non-alphanumeric characters. `query_command` tokenizes the query, performs substring matching against vocab tokens, and ANDs results across query tokens. Results are sorted by `BlockId` (monotonic = temporal order).

Both indexes use lazy eviction — tombstone filtering via `executions.contains_key()` avoids O(n) Vec removal on block eviction.

## Search Match Highlighting (renderer.rs)

```rust
fn search_tokens(query: &str) -> Vec<String>;
fn highlight_spans<'a>(text: &'a str, tokens: &[String]) -> Vec<(bool, &'a str)>;
```

`search_tokens` splits the query by non-alphanumeric characters and lowercases each token.

`highlight_spans` performs case-insensitive substring matching against all tokens, merges overlapping match intervals, and produces `(bool, &str)` spans where `true` = match. Used by `render_top_border` to highlight matched portions of the command text with `SEARCH_MATCH_FG` (YELLOW).

## Theme System (src/theme.rs)

```rust
pub struct CatppuccinFrappe;
// LAVENDER, TEXT, SUBTEXT0, SUBTEXT1, SURFACE2, SURFACE1, SURFACE0,
// MANTLE, GREEN, RED, YELLOW, MAUVE, BLUE, TEAL

pub struct Theme;
// BORDER_NORMAL_FG, BORDER_SELECTED_FG, BODY_SELECTED_BG, BODY_SELECTED_FG,
// CURSOR_BG, CURSOR_FG, FOOTER_BG, FOOTER_FG, FOOTER_KEY_FG, FOOTER_SEP_FG,
// DETAIL_BORDER_FG,
// STATUS_OK_FG, STATUS_FAILED_FG, STATUS_RUNNING_FG,
// META_LABEL_FG, META_HEADER_FG, META_PATH_FG,
// META_ACTION_KEY_FG, META_ACTION_TEXT_FG,
// HELP_BG, HELP_BORDER, HELP_KEY_FG, HELP_TEXT_FG, HELP_SEL_BG, HELP_SEL_FG, HELP_DIM_FG,
// SEARCH_MATCH_FG, VISUAL_BORDER_FG, VISUAL_LINE_BG,
// ICON_SECTION_FG, ICON_CMD_FG, ICON_PATH_FG, ICON_TIME_FG, ICON_ACTION_FG
```

Color constants for all themed rendering. All color values are Catppuccin Frappe RGB values applied via `SetForegroundColor`/`SetBackgroundColor` instead of `Attribute::Reverse`.

## ANSI Parser (src/ansi.rs)

```rust
pub fn parse_ansi_lines(bytes: &[u8]) -> Vec<StyledText>;

pub struct StyledText {
    pub spans: Vec<StyledSpan>,
}

pub struct StyledSpan {
    pub text: String,
    pub style: TextStyle,
}

pub struct TextStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}
```

`parse_ansi_lines` processes raw PTY byte streams into per-line styled text. Only SGR sequences (`ESC[...m`) are interpreted for styling; all other control sequences (CSI cursor movement, erase, OSC, alternate screen) are consumed and discarded. Style state carries across newlines until a reset is encountered. `\r` clears the current line (for progress bar overwrites). `\r\n` (ONLCR) is handled as a single line break.

Helper functions:
- `styled_width(text: &StyledText) -> usize` — total display width across all spans
- `styled_to_plain(text: &StyledText) -> String` — concatenate span text without styles
- `truncate_styled_to_width(text: &StyledText, max_width: usize) -> StyledText` — unicode-aware truncation preserving per-span styles
- `StyledText::plain(text) -> StyledText` — create a styled text from a plain string
- `StyledText::is_empty() -> bool` — check if all spans are empty

## Ownership Summary

- `selected_block` and `expanded_block` belong to `ViewState`.
- `selected_index`, block viewport `line_offset`, deprecated `scroll_offset`, and `anchor` belong to `BlockViewport`.
- `detail_line_cursor` belongs to `ViewState` (Detail View only).
- `help` (HelpState), `confirm` (ConfirmState), `visual_anchor`, `detail_visual_anchor`, `search_buffer`, `pre_search_query`, `filter`, and `visible` all belong to `ViewState`.
- `start_line` and `end_line` belong to `CommandBlock`.
- `ShellLine.block_id` lets the compositor identify block-owned shell output.
- Block borders and detail text are `VisualLine` values.
- ShellBuffer must stay free of rendered block metadata.
- `BlockStore.max_blocks` is data retention, not viewport size.
- `BlockViewConfig.preview_lines` and `expanded_lines` control body truncation.
- `BlockViewConfig.auto_follow_on_reach_bottom` gates `j`→Tail anchor mode.
- `BlockViewConfig.copy_format` controls clipboard serialization format.
- `RenderState.force_render` ensures view switches always render immediately.
- `RenderState.flash_message` provides transient clipboard feedback.
- `RenderState.pending_paste` carries rerun command text through cleanup.
- `RenderState.last_rendered_rows` enables incremental tail-line clearing.
- If a block has no captured linear text, Block View should display a placeholder such as `no captured text output`.
- Full-screen program terminal behavior is preserved by Normal passthrough, not by a whitelist.
- Viewport math (visible range, tail offset, scroll margin) is computed by the `Compositor` using the same `build_one_block_lines().len()` function as rendering, preventing height estimate mismatches.
- `ViewState.filter` and `ViewState.visible` control which blocks are shown (AllTimeline vs Filtered).
- All navigation functions use `view.visible.ids(blocks)` instead of `blocks.timeline` directly.
- `BlockIndex.failed` tracks failed blocks incrementally; `BlockIndex.tokens` is an inverted index for command text search.
- Body text is rendered from `block.output_raw` via `ansi::parse_ansi_lines()` when available, preserving ANSI colors and styles.
- Renderer uses Catppuccin Frappe theme colors (`theme::Theme`) instead of raw `Attribute::Reverse` for selection highlighting.
- `selected` and `in_visual` are separate flags: selected controls block focus (LAVENDER border), in_visual controls visual range (YELLOW border). `BlockSelectionStyle::from_state()` prioritises visual over selected.
- All borders always use round characters (`╭╮╰╯`). No body background fill on any border or body variant.
- Colors displayed after `bg:` are applied in `render_styled_framed_text` after the left `│` border, not before it.
- StyledDetailBodyLine uses `CURSOR_BG` for cursor line and `VISUAL_LINE_BG` for visual selection — never both at once.
