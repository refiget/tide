# Internal API

This document describes Tide's internal Rust interfaces. It is not an HTTP API.

The exact implementation can evolve, but code should preserve these ownership boundaries:

- `ShellBuffer` stores shell text.
- `BlockStore` stores structured execution data.
- `ViewState` stores selected / expanded / current view state.
- `Compositor` builds visual lines.
- `Renderer` draws visual lines.
- `CaptureEngine` updates buffer and block state from PTY output and shell markers.

## BlockId

```rust
pub struct BlockId(pub u64);
```

`BlockId` identifies one execution block.

## ShellBuffer

```rust
pub struct ShellBuffer {
    pub lines: Vec<ShellLine>,
}

pub struct ShellLine {
    pub text: String,
    pub block_id: Option<BlockId>,
}
```

`ShellBuffer` stores the shell text layer only.

`ShellLine.block_id` lets the compositor know which shell lines belong to which block.

Block borders, metadata, detail text, selected state, and expanded state are not part of `ShellBuffer`.

## ExecutionBlock

```rust
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

pub struct ExecutionBlock {
    pub id: BlockId,
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration: Option<Duration>,
    pub status: BlockStatus,
    pub kind: ExecutionKind,
    pub start_line: usize,
    pub end_line: usize,
    pub created_at: SystemTime,
}
```

`start_line` and `end_line` describe where the block appears in `ShellBuffer`.

If layout becomes more complex later, these fields may move into a dedicated `BlockLayout`.

## ExecutionKind

```rust
pub enum ExecutionKind {
    Normal,
    RawProgram,
}
```

`ExecutionKind::RawProgram` is reserved for future metadata. Current passthrough does not depend on command classification; Normal mode is transparent for every command.

## BlockStatus

```rust
pub enum BlockStatus {
    Running,
    Success,
    Failed,
    Cancelled,
}
```

## BlockStore

```rust
use std::collections::HashMap;

pub struct BlockStore {
    pub next_id: u64,
    pub timeline: Vec<BlockId>,
    pub executions: HashMap<BlockId, ExecutionBlock>,
    pub max_blocks: Option<usize>,
}
```

Expected methods:

```rust
impl BlockStore {
    pub fn new() -> Self;

    pub fn start_execution(
        &mut self,
        command: String,
        cwd: Option<PathBuf>,
        start_line: usize,
    ) -> BlockId;

    pub fn finish_execution(
        &mut self,
        id: BlockId,
        exit_code: i32,
        cwd: Option<PathBuf>,
        end_line: usize,
        duration: Duration,
    );

    pub fn get(&self, id: BlockId) -> Option<&ExecutionBlock>;

    pub fn get_mut(&mut self, id: BlockId) -> Option<&mut ExecutionBlock>;

    pub fn all(&self) -> Vec<&ExecutionBlock>;
}
```

`timeline` preserves execution order. `executions` provides lookup by id.

`max_blocks` controls retention only. It must not be used as the number of blocks visible on screen.

## ViewKind

```rust
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
Normal:
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

## ViewState

```rust
pub struct ViewState {
    pub view: ViewKind,
    pub selected_block: Option<BlockId>,
    pub expanded_block: Option<BlockId>,
    pub scroll_offset: usize,
    pub block_viewport: BlockViewport,
}

pub struct BlockViewport {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub anchor: ViewAnchor,
}

pub enum ViewAnchor {
    Top,
    Tail,
    Manual,
}
```

`selected_block` and `expanded_block` belong to `ViewState`.

`BlockViewport` controls which portion of block history is visible. It does not store block content.

`ViewAnchor::Tail` bottom-aligns the visible block region. `ViewAnchor::Top` displays from the top. `ViewAnchor::Manual` preserves the current viewport unless the selected block leaves the visible range.

Do not store selected or expanded state in `ExecutionBlock`.

## VisualLine

```rust
pub enum VisualLine {
    ShellText {
        text: String,
        block_id: Option<BlockId>,
    },
    BlockBodyLine {
        text: String,
        block_id: BlockId,
        selected: bool,
    },
    BlockTopBorder {
        block_id: BlockId,
        selected: bool,
        label: String,
    },
    BlockBottomBorder {
        block_id: BlockId,
        selected: bool,
        label: String,
    },
    BlockDetailLine {
        block_id: BlockId,
        text: String,
        selected: bool,
    },
}
```

Block borders and details are visual lines. They are not stored in `ShellBuffer`.

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
}
```

The compositor is responsible for turning `ShellBuffer + BlockStore + ViewState` into renderable `VisualLine` values.

Plain View generation is only used when Tide needs to restore a captured shell view, such as returning from Block View. During ordinary Normal mode, visible PTY bytes are passed through directly.

Plain View generation:

```text
ShellBuffer.lines -> VisualLine::ShellText
```

Block View generation:

```text
BlockTopBorder
BlockBodyLine values for shell lines in the block
BlockBottomBorder
```

Detail View generation:

```text
BlockTopBorder
BlockBodyLine values for shell lines in the block
BlockDetailLine...
BlockBottomBorder
```

## Renderer API

```rust
pub trait Renderer {
    fn render(&mut self, lines: &[VisualLine], view: &ViewState) -> anyhow::Result<()>;
}
```

The renderer draws visual lines to the terminal. It should not parse PTY output, mutate block data, or decide command lifecycle.

Renderer is used for reconstructed views such as Blocks and Detail. Normal mode should not continuously redraw through the renderer.

## Capture API

```rust
pub struct CaptureEngine {
    // internal state
}

impl CaptureEngine {
    pub fn on_marker(
        &mut self,
        marker: ShellMarker,
        shell: &mut ShellBuffer,
        blocks: &mut BlockStore,
    );

    pub fn on_output(
        &mut self,
        bytes: &[u8],
        shell: &mut ShellBuffer,
    );
}
```

`CaptureEngine` owns transient command-capture state, such as the currently running block id and start time.

Normal mode should forward visible bytes to the real terminal and capture best-effort plain text on the side. The first phase does not try to preserve full-screen program screen state.

## ShellMarker

```rust
use std::path::PathBuf;

pub enum ShellMarker {
    BlockStart {
        command: String,
    },
    BlockEnd {
        exit_code: i32,
        cwd: PathBuf,
    },
}
```

Markers should come from shell integration, not from prompt parsing.

`BlockStart` creates the block. `BlockEnd` finishes it. Do not use command-name detection as the primary lifecycle boundary.

## Config API

```rust
pub struct TideConfig {
    pub raw_programs: Vec<String>,
    pub history: HistoryConfig,
    pub block_view: BlockViewConfig,
    pub block_layout: BlockLayoutConfig,
}

pub struct RuntimeConfig {
    pub max_blocks: Option<usize>,
    pub block_view: BlockViewConfig,
    pub block_layout: BlockLayoutConfig,
}

pub struct HistoryConfig {
    pub max_blocks: Option<usize>,
}

pub struct BlockViewConfig {
    pub preview_lines: usize,
    pub expanded_lines: usize,
    pub follow_tail: bool,
    pub block_gap: usize,
    pub scroll_margin_blocks: usize,
}

pub struct BlockLayoutConfig {
    pub horizontal_padding: usize,
    pub show_padding_in_plain: bool,
}

pub fn load_config() -> anyhow::Result<TideConfig>;

pub fn build_runtime_config(config: TideConfig) -> RuntimeConfig;
```

Implementation note: the current code may name the loaded config type `Config`; it should still expose the same behavior.

`raw_programs` may remain as a legacy compatibility field in loaded config, but it must not be required for terminal passthrough.

If no config file exists, defaults are used.

Defaults:

- `history.max_blocks = 1000`
- `block_view.preview_lines = 6`
- `block_view.expanded_lines = 30`
- `block_view.follow_tail = true`
- `block_view.block_gap = 0`
- `block_view.scroll_margin_blocks = 2`

## Input API

```rust
use crossterm::event::KeyEvent;

pub enum AppCommand {
    SendToShell(Vec<u8>),
    EnterBlockView,
    EnterDetailView,
    Back,
    MoveSelectionUp,
    MoveSelectionDown,
    Redraw,
    Quit,
    Noop,
}

pub fn handle_key(key: KeyEvent, state: &ViewState) -> AppCommand;
```

The input layer maps keys to commands. Runtime code applies those commands to app state or forwards bytes to zsh.

## Ownership Summary

- `selected_block` and `expanded_block` belong to `ViewState`.
- `selected_index`, block viewport `scroll_offset`, and `anchor` belong to `BlockViewport`.
- `start_line` and `end_line` belong to `ExecutionBlock` or `BlockLayout`.
- `ShellLine.block_id` lets the compositor identify block-owned shell output.
- Block borders and detail text are `VisualLine` values.
- `ShellBuffer` must stay free of rendered block metadata.
- `BlockStore.max_blocks` is data retention, not viewport size.
- `BlockViewConfig.preview_lines` and `expanded_lines` control body truncation.
- If a block has no captured linear text, Block View should display a placeholder such as `no captured text output`.
- Full-screen program terminal behavior is preserved by Normal passthrough, not by a whitelist.
