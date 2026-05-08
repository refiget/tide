# Internal API

This document describes Tide's internal Rust interfaces. It is not an HTTP API.

The exact implementation can evolve, but code should preserve these ownership boundaries:

- `ShellBuffer` stores shell text.
- `BlockStore` stores structured execution data.
- `ViewState` stores selected / expanded / current view state.
- `Compositor` builds visual lines and computes visible ranges.
- `Renderer` draws visual lines.
- `Osc777Parser` strips shell markers and emits lifecycle events.

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
}
```

The central mutable state. Wrapped in `Arc<Mutex<RuntimeState>>` and shared between the input thread (mutates view state, triggers render) and the output thread (updates shell/block state from PTY output, renders).

## BlockId

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u64);
```

`BlockId` identifies one execution block. Display via `{}` prints the inner `u64`.

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
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn set_cwd(&mut self, cwd: String);
}
```

`timeline` preserves execution order as `Vec<BlockId>`. `executions` provides lookup by id via `HashMap`.

`max_blocks` controls retention only. When the timeline exceeds `max_blocks`, the oldest block is evicted. `max_blocks = None` means unbounded history.

`append_output` is capped at `max_output_bytes_per_block`. Once the cap is reached, `output_truncated` is set and further output is silently dropped.

`finish_command` converts `output_raw` to `output_text` by stripping ANSI escapes via `strip_ansi_escapes`.

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

Dispatched by `perform_block_action()` in the PTY input handler. `CopyCommand`, `CopyOutput`, and `CopyBlock` are wired to clipboard via `write_to_clipboard()`. `RerunCommand` exits to Plain View and pastes the command text into the PTY. The remaining variants are future/reserved for AI integration.

## ViewKind

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Plain,
    Blocks,
    Detail,
    Agent,
    RawProgram,
}
```

`Agent` and `RawProgram` are future-facing/reserved. Current Normal passthrough uses `ViewKind::Plain`.

Detail View is a full-screen pager mode for deep inspection of a single block, entered via `i` from Block View (not Enter; Enter toggles inline block expansion).

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

Current combinations:

```text
Normal / Plain:
  InputMode::Shell
  ViewKind::Plain

Block View:
  InputMode::BlockNav
  ViewKind::Blocks

Detail View:
  InputMode::DetailNav
  ViewKind::Detail

Full-screen programs in Normal:
  InputMode::Shell
  ViewKind::Plain
```

Note: `InputMode` is defined but currently unused in the main loop; view-mode dispatch is done directly on `ViewKind` in `handle_view_key_sequence`.

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
```

`selected_block` and `expanded_block` belong to `ViewState`.

`detail_line_cursor` is a 0-indexed cursor line within Detail View output, used by the full-screen pager when `ViewKind::Detail` is active.

`BlockViewport` controls which portion of block history is visible. It does not store block content.

- `line_offset` is the primary viewport offset. It is the first visible visual line in the full `VisualLayout`.
- `scroll_offset` is deprecated compatibility from the old block-index viewport model.
- `ViewAnchor::Tail` follows the end of the visual layout; new blocks shift the viewport only while Tail is active.
- `ViewAnchor::Top` displays from the first visual line.
- `ViewAnchor::Manual` preserves the current `line_offset` unless the selected block would become partially clipped.

Default for both `ViewState` and `BlockViewport` is `Plain` view + `Tail` anchor.

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
    BlockBodyLine { text: String, block_id: BlockId, selected: bool },
    BlockTopBorder { block_id: BlockId, selected: bool, label: String },
    BlockBottomBorder { block_id: BlockId, selected: bool, label: String },
    BlockDetailLine { block_id: BlockId, text: String, selected: bool },
    DetailTopBorder { block_id: BlockId, label: String },
    DetailBottomBorder { block_id: BlockId, label: String },
    DetailBodyLine { block_id: BlockId, text: String, is_cursor: bool },
    Footer { text: String },
}
```

Block borders and details are visual lines. They are not stored in `ShellBuffer`.

Block View uses the `Block*` variants: `selected: true` on borders and body lines highlights the entire selected block.

Detail View uses the `Detail*` variants: borders are never highlighted, and body lines carry `is_cursor: bool` to highlight only the active cursor line. The semantics are completely separated from Block View.

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
    ) -> Vec<VisualLine>;

    pub fn build_visual_layout(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        width: u16,
        block_view: &BlockViewConfig,
    ) -> VisualLayout;

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
    shell: &ShellBuffer,
    blocks: &BlockStore,
    view: &ViewState,
    width: u16,
    height: u16,
    block_view: &BlockViewConfig,
    flash_message: Option<&str>,
) -> Vec<VisualLine>;
```

`build_detail_lines` constructs a full-screen single-block pager with:
- `DetailTopBorder` / `DetailBottomBorder` (no Reverse highlighting)
- `DetailBodyLine` (with `is_cursor` for line cursor highlighting)
- Vertical centering of short output
- Auto-scroll support via `block_viewport.line_offset`

Helper functions in `compositor.rs`:

```rust
fn get_block_output_lines(block: &CommandBlock, shell_lines: &[ShellLine]) -> Vec<String>;
fn detail_footer_text(block: &CommandBlock, view: &ViewState, total_lines: usize, inner_height: usize, flash_message: Option<&str>) -> String;
```

`get_block_output_lines` extracts the block's visible output from shell lines (or returns `"interactive program; screen output was not captured"` for `RawProgram` blocks). `detail_footer_text` builds the pager footer with scroll position and available keybindings.

### Block View composition

Plain View generation (`build_visual_lines` with `ViewKind::Plain`):

```text
ShellBuffer.lines -> VisualLine::ShellText
```

Block View generation (`ViewKind::Blocks`):

```text
for each block in the full VisualLayout:
  BlockTopBorder (selected: true for selected block)
  BlockBodyLine values for shell lines in the block's range
  (if expanded_block == Some(id): BlockDetailLine values)
  BlockBottomBorder (selected: true for selected block)

then slice by BlockViewport.line_offset and content height
```

`expanded_block` is a per-block inline toggle that stays in `ViewKind::Blocks`. When set, the block shows all output lines (capped at `expanded_lines`) plus detail metadata (command, cwd, exit code, duration, actions). When set, the selected block is expanded; navigation via `j`/`k`/`g`/`G`/Enter automatically moves the expanded state to the newly selected block.

### Detail View composition

```text
for the single expanded block:
  DetailTopBorder (no highlight)
  DetailBodyLine values (with is_cursor: bool for cursor highlighting)
  DetailBottomBorder (no highlight)

then pad to fill screen (short output vertically centered)
```

Detail View is a full-screen pager entered via `i` from Block View. The cursor (`detail_line_cursor`) moves independently with `j`/`k`; the viewport auto-scrolls when the cursor leaves the visible area. `g`/`G` jump to top/bottom.

Height calculations come from the generated visual layout for Block View, or from direct arithmetic for Detail View. This intentionally avoids separate estimated block heights that can drift from rendered output.

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
) -> io::Result<usize>;

pub fn enter_block_render<W: Write>(w: &mut W) -> io::Result<()>;

pub fn leave_block_render<W: Write>(w: &mut W) -> io::Result<()>;
```

`render` draws visual lines to the terminal via crossterm. It no longer does a full-screen `Clear(ClearType::All)` on every frame. Instead:
1. Draws each visible line, stopping at terminal height
2. Clears stale tail rows from the previous frame that fall outside the new frame's range (using `last_rendered_rows`)
3. Returns the number of lines actually rendered

The caller (`render_runtime` in `pty.rs`) stores the returned count in `RenderState.last_rendered_rows` for the next frame.

Crossterm `queue!` is used for batching; all drawing commands are flushed atomically at the end to avoid intermediate blank frames.

Detail View rendering: `DetailTopBorder` and `DetailBottomBorder` draw titled borders without Reverse highlighting. `DetailBodyLine` applies `SetAttribute(Reverse)` only when `is_cursor` is true, followed by `SetAttribute(Reset)`.

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
    pub selected_body_reverse: bool,
}

pub struct BlockLayoutConfig {
    pub horizontal_padding: usize,
    pub show_padding_in_plain: bool,
}
```

`raw_programs` may remain as a legacy compatibility field in loaded config, but it must not be required for terminal passthrough. Full-screen programs work without a whitelist because Normal mode is transparent.

If no config file exists, defaults are used.

Defaults:

- `history.max_blocks = 1000`
- `block_view.preview_lines = 4`
- `block_view.expanded_lines = 20`
- `block_view.follow_tail = true`
- `block_view.block_gap = 0`
- `block_view.scroll_margin_lines = 2`
- `block_view.scroll_margin_blocks = 2` legacy compatibility
- `block_view.auto_follow_on_reach_bottom = false`
- `block_view.horizontal_margin = 1`
- `block_view.body_padding = 1`
- `block_view.show_footer = true`
- `block_view.selected_body_reverse = false`
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
| `Enter` | Toggle inline block expansion (stays in `ViewKind::Blocks`, force render). If already expanded, collapse; otherwise expand. `expanded_block` follows selection during navigation. |
| `i` | Enter Detail View (switches to `ViewKind::Detail`, resets cursor and line_offset, force render) |
| `y` | Copy output text to clipboard |
| `Y` | Copy command text to clipboard |
| `r` | Rerun: saves selected block's command to `pending_paste`, sets `needs_cleanup`, exits to Plain View |
| `q` / `Esc` | Return to Plain View (resets to default ViewState, force render) |

### Detail View key bindings (`handle_view_key_sequence`)

| Key | Action |
|-----|--------|
| `j` / ArrowDown | Move cursor down one line. Auto-scrolls `line_offset` when cursor reaches the visible area bottom. |
| `k` / ArrowUp | Move cursor up one line. Auto-scrolls `line_offset` when cursor passes the visible area top. |
| `g` | Jump cursor to first output line, reset `line_offset` to 0 |
| `G` | Jump cursor to last output line, scroll `line_offset` to show it |
| `yc` | Copy command text to clipboard (two-key sequence, consumes `y` + `c`) |
| `yo` | Copy output text to clipboard (two-key sequence, consumes `y` + `o`) |
| `yb` | Copy block (command + output) to clipboard (two-key sequence, consumes `y` + `b`) |
| `r` | Rerun: saves expanded block's command to `pending_paste`, sets `needs_cleanup` |
| `q` / `Esc` | Return to Block View (resets `detail_line_cursor` and `line_offset` to 0, force render) |

### Navigation helpers

```rust
fn detail_output_line_count(state: &RuntimeState) -> usize;
fn detail_inner_height(state: &RuntimeState) -> usize;
```

`detail_output_line_count` returns the number of output lines in the expanded block for Detail View (returns `1` for `RawProgram` blocks to match the compositor's single placeholder line). `detail_inner_height` returns the available output area in rows: `(state.rows as usize).saturating_sub(4)` (top margin + top border + bottom border + footer).

### General flow

`handle_view_key_sequence` dispatches based on current `ViewKind`:
- `Plain` / `RawProgram` → returns `None` (bytes go to PTY), except `Ctrl-B` (0x02) handled in the outer loop
- `Blocks` → returns `None` (all Block View input goes through `handle_block_view_byte`)
- `Detail` → matches the bindings above directly

`accumulate_block_delta` adds to `InputAccumulator.pending_block_delta` and clamps to `[-limit, limit]`.

`flush_navigation_delta` resets `pending_block_delta` to 0 and calls `select_relative_block`.

`select_block_index` updates `selected_block`, `selected_index`, and the viewport anchor. When `expanded_block.is_some()`, the expanded state follows selection automatically.

`select_tail_block` jumps to the newest block with Tail anchor, and also syncs `expanded_block` when applicable.

`maybe_flush_navigation_and_render` is called after each input chunk. If `dirty` is set, it waits up to `FRAME_DURATION` (16ms) before flushing and rendering. If `force_render` is set, the wait is skipped.

The `Ctrl-B` byte (0x02) is handled in the input loop itself (not in `handle_view_key_sequence`): it calls `enter_block_view` and triggers navigation flush + render.

## Clipboard and Copy

```rust
fn write_to_clipboard(text: &str) -> bool;
fn perform_block_action(state: &mut RuntimeState, action: BlockAction);
```

`write_to_clipboard` uses `pbcopy` on macOS (via `std::process::Command`) and `arboard` on other platforms. Returns `false` on failure (e.g. headless CI, no clipboard available).

`perform_block_action` looks up the current block (from `selected_block` in Block View or `expanded_block` in Detail View), extracts the appropriate text for the action, and calls `write_to_clipboard`. On success, it sets `RenderState.flash_message` (e.g. "copied output", "copied command", "copied block") with a 1500ms expiration timer.

## Rerun Flow

The `r` key handler:
1. Looks up the current block's command text
2. Resets `ViewState` to defaults
3. Sets `RenderState.needs_cleanup = true`
4. Sets `RenderState.pending_paste = Some(command)`

On the next iteration of the input loop cleanup path:
1. `needs_cleanup` triggers alt-screen exit (leave alternate screen, reset SGR, show cursor)
2. `pending_paste` is extracted and written to the PTY writer
3. The command text appears in the shell as if the user typed it, followed by a newline

## Flash Message Lifecycle

Flash messages (e.g. "copied output") provide transient user feedback for clipboard operations:

1. Set by `perform_block_action` on successful clipboard write
2. Read by `render_runtime` from `RenderState.flash_message` at the start of each render
3. Expires after 1500ms (`Duration::from_millis(1500)`) — if `at.elapsed() >= 1500ms`, the flash is cleared
4. Passed to the compositor as `flash_message: Option<&str>`
5. The compositor renders it in the footer via `footer_text()` (Block View) or `detail_footer_text()` (Detail View)

## Render Loop (pty.rs)

The output thread reads PTY output, dispatches to `Osc777Parser` for marker stripping, updates `ShellBuffer` and `BlockStore`, and renders if in Block/Detail view.

The input thread reads stdin, applies `handle_view_key_sequence` for view-mode keys, forwards remaining bytes to the PTY writer, and calls `maybe_flush_navigation_and_render` for frame-rate-limited redraws.

The central render dispatch function:

```rust
fn render_runtime(state: &Arc<Mutex<RuntimeState>>, stdout: &Arc<Mutex<io::Stdout>>) -> io::Result<()>;
```

1. Locks `RuntimeState`
2. Checks/expires `flash_message`
3. Calls `Compositor::build_visual_lines()` (which dispatches to `build_block_lines` for Block View or `build_detail_lines` for Detail View)
4. Unlocks state, passes the visual lines to `renderer::render()` along with `last_rendered_rows`
5. Re-locks state and stores `rendered` count into `last_rendered_rows`

Frame rate is controlled by `FRAME_DURATION` (16ms ≈ 60fps). View mode switches use `force_render` to bypass the frame timer.

## Ownership Summary

- `selected_block` and `expanded_block` belong to `ViewState`.
- `selected_index`, block viewport `line_offset`, deprecated `scroll_offset`, and `anchor` belong to `BlockViewport`.
- `detail_line_cursor` belongs to `ViewState` (Detail View only).
- `start_line` and `end_line` belong to `CommandBlock`.
- `ShellLine.block_id` lets the compositor identify block-owned shell output.
- Block borders and detail text are `VisualLine` values.
- ShellBuffer must stay free of rendered block metadata.
- `BlockStore.max_blocks` is data retention, not viewport size.
- `BlockViewConfig.preview_lines` and `expanded_lines` control body truncation.
- `BlockViewConfig.auto_follow_on_reach_bottom` gates `j`→Tail anchor mode.
- `RenderState.force_render` ensures view switches always render immediately.
- `RenderState.flash_message` provides transient clipboard feedback.
- `RenderState.pending_paste` carries rerun command text through cleanup.
- `RenderState.last_rendered_rows` enables incremental tail-line clearing.
- If a block has no captured linear text, Block View should display a placeholder such as `no captured text output`.
- Full-screen program terminal behavior is preserved by Normal passthrough, not by a whitelist.
- Viewport math (visible range, tail offset, scroll margin) is computed by the `Compositor` using the same `build_one_block_lines().len()` function as rendering, preventing height estimate mismatches.
