# Internal API

This document describes Tide's internal Rust interfaces. It is not an HTTP API.

The exact implementation can evolve, but code should preserve these ownership boundaries:

- `ShellBuffer` stores shell text.
- `BlockStore` stores structured execution data.
- `ViewState` stores selected / expanded / current view state.
- `Compositor` builds visual lines and computes visible ranges.
- `Renderer` draws visual lines.
- `Osc777Parser` strips shell markers and emits lifecycle events.

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
}
```

`start_line` and `end_line` describe where the block appears in `ShellBuffer` (indices into `ShellBuffer.lines`).

`output_raw` stores raw bytes captured during command execution. `output_text` is derived from `output_raw` by stripping ANSI escape sequences (via `strip_ansi_escapes`).

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

`append_output` is capped at `max_output_bytes_per_block`. Once the cap is reached, further output is silently dropped for that block.

`finish_command` converts `output_raw` to `output_text` by stripping ANSI escapes via `strip_ansi_escapes`.

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
}

#[derive(Debug, Clone)]
pub struct BlockViewport {
    pub selected_index: usize,
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

`BlockViewport` controls which portion of block history is visible. It does not store block content.

- `ViewAnchor::Tail` bottom-aligns the visible block region; new blocks shift the viewport.
- `ViewAnchor::Top` displays from the oldest visible block.
- `ViewAnchor::Manual` preserves the current viewport unless the selected block leaves the visible range or crosses the scroll margin.

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
}
```

`dirty` is set `true` whenever navigation or view state changes. `force_render` is set `true` on view mode switches (enter/exit Block View, enter/exit Detail, g/G jumps) to bypass frame-rate limiting and guarantee an immediate redraw.

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
}
```

Block borders and details are visual lines. They are not stored in `ShellBuffer`.

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

The compositor is responsible for turning `ShellBuffer + BlockStore + ViewState` into renderable `VisualLine` values, AND is the single source of truth for viewport math (visible range, tail offset, scroll offset).

Height calculations inside the compositor use `build_one_block_lines().len()` — the same function that generates the actual visual lines — guaranteeing that viewport estimates match rendered output.

Plain View generation (`build_visual_lines` with `ViewKind::Plain`):

```text
ShellBuffer.lines -> VisualLine::ShellText
```

Block View generation (`ViewKind::Blocks` or `ViewKind::Detail`):

```text
for each visible block:
  BlockTopBorder
  BlockBodyLine values for shell lines in the block's range
  (if Detail and expanded: BlockDetailLine values)
  BlockBottomBorder
```

## Renderer API

```rust
pub fn render<W: Write>(
    w: &mut W,
    visual_lines: &[VisualLine],
    view: &ViewState,
    cursor: Option<(usize, usize)>,
    layout: &BlockLayoutConfig,
    rows: u16,
    cols: u16,
) -> io::Result<()>;
```

The renderer draws visual lines to the terminal via crossterm. It clears the screen, draws each line with appropriate formatting, and positions the cursor.

For Plain View, cursor position is drawn from `ShellBuffer.cursor_position()`. For Block/Detail views, the cursor is hidden.

Normal mode should not continuously redraw through the renderer. The renderer is used for reconstructed views (Blocks, Detail) and for restoring Plain View after exiting Block View.

The viewport start is computed from `view.view`: Plain uses `lines.len() - height` (bottom of buffer), Blocks/Detail uses 0 (from the scroll offset's first visible block).

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

pub type RuntimeConfig = Config; // via build_runtime_config

pub struct BlockViewConfig {
    pub preview_lines: usize,
    pub expanded_lines: usize,
    pub follow_tail: bool,
    pub block_gap: usize,
    pub scroll_margin_blocks: usize,
    pub auto_follow_on_reach_bottom: bool,
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
- `block_view.preview_lines = 6`
- `block_view.expanded_lines = 30`
- `block_view.follow_tail = true`
- `block_view.block_gap = 0`
- `block_view.scroll_margin_blocks = 2`
- `block_view.auto_follow_on_reach_bottom = false`
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

`handle_view_key_sequence` dispatches based on current `ViewKind`:
- `Plain` / `RawProgram` → returns `None` (bytes go to PTY), except `Ctrl-B` (0x02) handled in the outer loop
- `Blocks` → matches escape sequences (arrow keys) and single bytes (j/k/g/G/Enter/q/Esc)
- `Detail` → matches q/Esc to return to Blocks

`accumulate_block_delta` adds to `InputAccumulator.pending_block_delta` and clamps to `[-limit, limit]`.

`flush_navigation_delta` resets `pending_block_delta` to 0 and calls `select_relative_block`.

`maybe_flush_navigation_and_render` is called after each input chunk. If `dirty` is set, it waits up to `FRAME_DURATION` (16ms) before flushing and rendering. If `force_render` is set, the wait is skipped.

The `Ctrl-B` byte (0x02) is handled in the input loop itself (not in `handle_view_key_sequence`): it calls `enter_block_view` and triggers navigation flush + render.

## Render Loop (pty.rs)

The output thread reads PTY output, dispatches to `Osc777Parser` for marker stripping, updates `ShellBuffer` and `BlockStore`, and renders if in Block/Detail view.

The input thread reads stdin, applies `handle_view_key_sequence` for view-mode keys, forwards remaining bytes to the PTY writer, and calls `maybe_flush_navigation_and_render` for frame-rate-limited redraws.

Frame rate is controlled by `FRAME_DURATION` (16ms ≈ 60fps). View mode switches use `force_render` to bypass the frame timer.

## Ownership Summary

- `selected_block` and `expanded_block` belong to `ViewState`.
- `selected_index`, block viewport `scroll_offset`, and `anchor` belong to `BlockViewport`.
- `start_line` and `end_line` belong to `CommandBlock`.
- `ShellLine.block_id` lets the compositor identify block-owned shell output.
- Block borders and detail text are `VisualLine` values.
- `ShellBuffer` must stay free of rendered block metadata.
- `BlockStore.max_blocks` is data retention, not viewport size.
- `BlockViewConfig.preview_lines` and `expanded_lines` control body truncation.
- `BlockViewConfig.auto_follow_on_reach_bottom` gates `j`→Tail anchor mode.
- `RenderState.force_render` ensures view switches always render immediately.
- If a block has no captured linear text, Block View should display a placeholder such as `no captured text output`.
- Full-screen program terminal behavior is preserved by Normal passthrough, not by a whitelist.
- Viewport math (visible range, tail offset, scroll margin) is computed by the `Compositor` using the same `build_one_block_lines().len()` function as rendering, preventing height estimate mismatches.
