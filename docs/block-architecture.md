# Block Architecture

This document describes Tide's block layer: the data model, lifecycle, rendering pipeline, and view systems that turn shell command history into structured, navigable blocks.

---

## Core Data Structures

### BlockId

```rust
// src/app.rs
pub struct BlockId(pub u64);
```

A newtype wrapper around `u64`. Each command execution gets a unique, monotonically increasing ID. Implements `Display`, `Hash`, `Ord`.

### CommandBlock (ExecutionBlock)

```rust
// src/app.rs
pub struct CommandBlock {
    pub id: BlockId,
    pub command: String,         // shell command text (e.g. "echo hello")
    pub cwd: PathBuf,            // working directory when command ran
    pub started_at: SystemTime,
    pub finished_at: Option<SystemTime>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub output_raw: Vec<u8>,     // raw PTY bytes (with ANSI escapes)
    pub output_text: String,     // ANSI-stripped plain text, set on finish_command
    pub kind: BlockKind,
    pub status: BlockStatus,
    pub git_context: Option<GitContext>,
    pub suggestions: Vec<SuggestedAction>,
    pub start_line: usize,       // ShellBuffer line index where output begins
    pub end_line: usize,         // ShellBuffer line index where output ends
    pub output_truncated: bool,  // true when max_output_bytes cap was hit
}
```

The canonical type for one command execution. `ExecutionBlock` is a type alias.

### BlockKind

```rust
pub enum BlockKind {
    NormalCommand,   // successful ordinary command
    FailedCommand,   // non-zero exit (promoted from NormalCommand on finish)
    TuiSession,      // interactive TUI program (reserved)
    RawProgram,      // full-screen program detected (vim, fzf, less, etc.)
    AiGenerated,     // future: AI-generated commands
    SystemEvent,     // future: system notifications
}
```

### BlockStatus

```rust
pub enum BlockStatus {
    Running,       // command still executing
    Success,       // exit code 0
    Failed,        // non-zero exit
    Interrupted,   // killed / SIGINT
    Unknown,       // no exit code recorded
}
```

---

## BlockStore — Storage and Lifecycle

```rust
// src/block.rs
pub struct BlockStore {
    next_id: u64,
    pub timeline: Vec<BlockId>,            // ordered list of block IDs
    pub executions: HashMap<BlockId, CommandBlock>,  // ID→block lookup
    pub max_blocks: Option<usize>,          // retention cap (None = unbounded)
    active_block_id: Option<BlockId>,       // currently executing block
    current_cwd: PathBuf,
    max_output_bytes_per_block: usize,      // output truncation limit
}
```

### Block Lifecycle

```
zsh preexec hook (ShellHookEvent::Preexec)
    │
    ▼
start_command(command, start_line, kind)
    ├── Creates CommandBlock with BlockStatus::Running
    ├── Assigns next_id, pushes to timeline
    ├── Evicts oldest if max_blocks exceeded
    └── Sets active_block_id
    │
    ▼ (during command execution)
append_output(bytes)
    ├── Appends raw bytes to block.output_raw
    ├── Stops at max_output_bytes_per_block
    ├── Sets output_truncated = true if limit hit
    └── ShellBuffer also captures the same bytes
    │
    ▼
zsh precmd hook (ShellHookEvent::Precmd)
    │
    ▼
finish_command(exit_code, end_line)
    ├── Records duration, exit_code, status
    ├── Derives output_text: strips ANSI from output_raw
    ├── Promotes NormalCommand → FailedCommand on non-zero exit
    └── Clears active_block_id
```

Key detail: `output_text` is **only set on finish_command**, not incrementally during output capture. For a currently running command, `output_text` is empty until the command finishes.

### Retention

When `max_blocks` is set and the timeline exceeds it, the oldest block is removed on each new `start_command`:

```rust
if let Some(max_blocks) = self.max_blocks
    && self.timeline.len() >= max_blocks
{
    self.executions.remove(&oldest);
    self.timeline.remove(0);
}
```

Default: `max_blocks = 1000`.

---

## VisualLine Enum — The Rendering Primitives

```rust
// src/compositor.rs
pub enum VisualLine {
    /// Empty padding row (no content drawn)
    Empty,

    /// Plain text from shell history (Normal / Plain View)
    ShellText { text: String, block_id: Option<BlockId> },

    /// One line of a block's body output (Block View)
    BlockBodyLine { text: String, block_id: BlockId, selected: bool },

    /// Top border of a block frame (Block View)
    BlockTopBorder { block_id: BlockId, selected: bool, label: String },

    /// Bottom border of a block frame (Block View)
    BlockBottomBorder { block_id: BlockId, selected: bool, label: String },

    /// One line of inline detail metadata (Block View, expanded blocks)
    BlockDetailLine { block_id: BlockId, text: String, selected: bool },

    /// Top border in Detail View (no highlight)
    DetailTopBorder { block_id: BlockId, label: String },

    /// Bottom border in Detail View (no highlight)
    DetailBottomBorder { block_id: BlockId, label: String },

    /// Body line in Detail View, with cursor highlight
    DetailBodyLine { block_id: BlockId, text: String, is_cursor: bool },

    /// Footer bar (inverted line at screen bottom)
    Footer { text: String },
}
```

### Variant semantics — `selected` vs `is_cursor`

| Variant | Rendering | Used in |
|---------|-----------|---------|
| `BlockTopBorder` / `BlockBottomBorder` | `selected=true` → `Attribute::Reverse` (entire bar highlighted) | Block View |
| `BlockBodyLine` | `selected=true` + config `selected_body_reverse` → Reverse | Block View |
| `BlockDetailLine` | `selected=true` + config `selected_body_reverse` → Reverse | Block View (expanded) |
| `DetailTopBorder` / `DetailBottomBorder` | Always plain (no Reverse) | Detail View |
| `DetailBodyLine` | `is_cursor=true` → Reverse, else plain | Detail View |

**Design rule**: Block View variants use `selected` to mean "this entire block is the navigation target". Detail View variants use `is_cursor` to mean "this specific line is the inspection cursor". The semantics are intentionally separate.

---

## ViewState — Navigation and Display State

```rust
// src/app.rs
pub struct ViewState {
    pub view: ViewKind,                    // Plain | Blocks | Detail | Agent | RawProgram
    pub selected_block: Option<BlockId>,   // currently highlighted block (Block View)
    pub expanded_block: Option<BlockId>,   // currently expanded block (enter/exit expand mode)
    pub scroll_offset: usize,              // deprecated (kept for transition)
    pub block_viewport: BlockViewport,     // scroll position, anchor, selection index
    pub detail_line_cursor: usize,         // 0-indexed cursor line in Detail View
}

pub struct BlockViewport {
    pub selected_index: usize,  // the index within timeline[] of selected_block
    pub line_offset: usize,     // first visual line visible in the viewport
    pub scroll_offset: usize,   // deprecated
    pub anchor: ViewAnchor,     // Top | Tail | Manual
}

pub enum ViewAnchor {
    Top,    // viewport anchored to oldest blocks
    Tail,   // viewport anchored to newest blocks (follow)
    Manual, // viewport at a specific scroll position
}
```

### ExpansionMode (implicit, no enum)

Whether a block is "expanded" is derived from the relation between `expanded_block` and `selected_block`:

```
expanded_block = None        → Collapsed mode
expanded_block = Some(id)    → FollowSelection mode
```

In **FollowSelection mode**, `expanded_block` automatically updates to match `selected_block` on every navigation (`select_block_index` in `pty.rs:928-931`):

```rust
if state.view.expanded_block.is_some() {
    state.view.expanded_block = state.view.selected_block;
}
```

### Navigation Flow

```
Plain  ──Ctrl-B──►  Blocks  ──i──►  Detail
  ▲                  │  ▲              │
  └──q/Esc───────────┘  └────q/Esc─────┘
```

| Key | Block View | Detail View |
|-----|-----------|-------------|
| `j`/`k` or Up/Down | Accumulate delta, move block selection | Move cursor line (auto-scrolls viewport) |
| `g` | Jump to oldest block | Jump to output top |
| `G` | Jump to newest block (Tail) | Jump to output bottom |
| `Enter` | Toggle expand/collapse current block | — |
| `i` | Enter Detail View on current block | — |
| `y` | Copy output | — |
| `Y` | Copy command | — |
| `yc` | — | Copy command |
| `yo` | — | Copy output |
| `yb` | — | Copy block (command + output) |
| `r` | Exit to Plain + paste command to PTY | Exit to Plain + paste command to PTY |
| `q`/`Esc` | Exit to Plain | Return to Block View |

### Input Batching

Block View `j`/`k` inputs are accumulated via `InputAccumulator` to avoid per-key rendering:

```rust
// src/app.rs
pub struct InputAccumulator {
    pub pending_block_delta: isize,
    pub last_input_at: Option<Instant>,
}
```

Deltas are flushed at frame cadence (16ms, via `FRAME_DURATION`) in `flush_navigation_delta()` / `select_relative_block()`. Detail View does NOT use input batching — cursor movements are applied immediately.

---

## Block View Rendering Pipeline

```
render_runtime (pty.rs:476)
  │
  ▼
build_visual_lines (compositor.rs:106)
  ├── ViewKind::Plain  → ShellText lines
  ├── ViewKind::Blocks  → build_block_lines
  │     │
  │     ├── build_visual_layout (compositor.rs:169)
  │     │     ├── Iterates blocks.timeline, calls build_one_block_lines for each
  │     │     ├── Returns VisualLayout { lines, spans, total_height }
  │     │     └── build_one_block_lines (compositor.rs:263)
  │     │           ├── BlockTopBorder (selected if this is the selected block)
  │     │           ├── Body: BlockBodyLine × N
  │     │           │     ├── Collapsed (expanded_block ≠ block_id): preview_lines rows
  │     │           │     │     └── Truncation hint: "... N more lines · Enter to expand"
  │     │           │     └── Expanded (expanded_block == block_id): expanded_lines rows
  │     │           │           └── Truncation hint: "... N more lines · i to inspect in Detail"
  │     │           ├── [If expanded]: BlockDetailLine × N (metadata: command/cwd/exit/...)
  │     │           └── BlockBottomBorder + gap Empty lines
  │     │
  │     ├── slice_visible_lines (compositor.rs:209)
  │     │     └── Clips VisualLayout by viewport.line_offset, fills Empty padding
  │     │
  │     └── Footer { text: footer_text() }
  │           "Block #3/12  j/k move  Enter expand  i detail  g/G top/btm  q quit"
  │
  ├── ViewKind::Detail → build_detail_lines (compositor.rs:425)
  │     ├── Empty (top margin / centered padding)
  │     ├── DetailTopBorder { label }
  │     ├── DetailBodyLine × N (output lines, is_cursor highlights active line)
  │     ├── DetailBottomBorder { label }
  │     └── Footer { text: detail_footer_text() }
  │           Short: "Detail #3   q back   yc cmd   yo output   yb block"
  │           Long:  "Detail #3   ↑↓ scroll   g/G top/bottom   q back   line 23/150"
  │
  └── ViewKind::Agent → empty vec
```

### VisualLayout + Slicing

```rust
pub struct VisualLayout {
    pub lines: Vec<VisualLine>,       // full composited visual lines
    pub spans: Vec<BlockVisualSpan>,  // per-block line ranges
    pub total_height: usize,          // lines.len()
}

pub struct BlockVisualSpan {
    pub block_id: BlockId,
    pub block_index: usize,
    pub start_line: usize,
    pub end_line: usize,
}
```

`slice_visible_lines` uses `line_offset` and `content_height` to extract the visible window. When content fits within the terminal, anchors determine vertical alignment:
- **Top**: content aligned to top, empty space below
- **Tail/Manual**: content aligned to bottom, empty space above

---

## Detail View — Full-Screen Pager

### Entry

`i` in Block View → sets `view = ViewKind::Detail`, `expanded_block = Some(selected)`, `detail_line_cursor = 0`, `line_offset = 0`.

### Cursor + Auto-scroll

```
j (cursor down):
  detail_line_cursor += 1
  if cursor ≥ line_offset + inner_height:
      line_offset = cursor - (inner_height - 1)

k (cursor up):
  detail_line_cursor -= 1
  if cursor < line_offset:
      line_offset = cursor
```

### Short vs Long Mode

**Short mode** (`total_output_lines ≤ inner_height`):
- Frame fits on screen; vertically centered via `top_padding` calculation
- `line_offset` = 0, no scrolling possible
- Footer: no scroll position shown

**Long mode** (`total_output_lines > inner_height`):
- Frame fills screen; `inner_height = rows - 4` (margin + top_border + bottom_border + footer)
- Output lines start at `line_offset`, fill `inner_height` rows
- Footer shows `line X/Y`

### Vertical Centering (Short Mode)

```rust
let frame_height = 2 + output_lines.len(); // top_border + body + bottom_border
let available = height - 1;                // minus footer
let top_padding = max((available - frame_height) / 2, 1);
```

---

## Renderer

### Render Pipeline

```rust
// src/renderer.rs
render(w, visual_lines, view, cursor, layout, block_view, rows, cols, last_rendered_rows)
  │
  ├── For each visible row:
  │     queue!(MoveTo(0, row))
  │     render_line(w, line, width, layout, block_view)
  │     queue!(Clear(UntilNewLine))
  │
  ├── Clear tail rows from previous frame
  │
  ├── Cursor: Show for Plain view, Hide for Block/Detail
  └── w.flush()
```

### Variant → Rendering Map

| Variant | Renderer Handler | Visual Effect |
|---------|-----------------|---------------|
| `Empty` | no-op | Nothing drawn |
| `ShellText` | `Print(text)` | Plain text |
| `BlockBodyLine` | `render_framed_text(w, text, selected, ...)` | `│...│` with conditional Reverse |
| `BlockTopBorder` | `render_border(w, label, selected, top=true, ...)` | `╭─ label ─╮` with conditional Reverse |
| `BlockBottomBorder` | `render_border(w, label, selected, top=false, ...)` | `╰─ label ─╯` with conditional Reverse |
| `BlockDetailLine` | `render_framed_text(w, text, selected, ...)` | `│...│` with conditional Reverse |
| `DetailTopBorder` | `Print(titled_border('╭','╮', label))` | `╭─ label ─╮`, always plain |
| `DetailBottomBorder` | `Print(titled_border('╰','╯', label))` | `╰─ label ─╯`, always plain |
| `DetailBodyLine` | conditional Reverse + `Print(framed_text(...))` | `│...│`, Reverse only on cursor line |
| `Footer` | `render_footer(w, text)` | Inverted bar across full width |

### Blink-Free Rendering

- No `Clear(All)` — each row is individually overwritten
- `last_rendered_rows` tracks previous frame's line count
- If new frame is shorter, stale tail rows are cleared via `Clear(CurrentLine)`
- Footer is always on the last rendered row

---

## Key Terminology

| Term | Definition |
|------|-----------|
| **Block** | One shell command execution with its output, metadata, and lifecycle |
| **Block expansion** | Inline toggle within Block View: `Enter` shows/hides all output + metadata for the selected block. `expanded_block` follows the selection. |
| **Detail View** | Full-screen pager (`ViewKind::Detail`), entered via `i`. Shows one block with line-cursor scrolling. |
| **Collapsed** | Default state: each block shows `preview_lines` (default 4) of output |
| **Expanded** | FollowSelection mode: selected block shows `expanded_lines` (default 20) + metadata |
| **FollowSelection** | When a block is expanded, navigation (j/k/g/G) automatically updates `expanded_block` to match `selected_block` |
| **View Anchor** | Controls viewport alignment: Top (oldest), Tail (newest/follow), Manual (user-scrolled) |

---

## Config (Block View)

```rust
// src/config.rs
pub struct BlockViewConfig {
    pub preview_lines: usize,                 // default: 4
    pub expanded_lines: usize,                // default: 20
    pub follow_tail: bool,                    // default: true
    pub block_gap: usize,                     // default: 0
    pub scroll_margin_blocks: usize,          // default: 2
    pub scroll_margin_lines: usize,           // default: 2
    pub auto_follow_on_reach_bottom: bool,    // default: false
    pub horizontal_margin: usize,             // default: 1
    pub body_padding: usize,                  // default: 1
    pub show_footer: bool,                    // default: true
    pub selected_body_reverse: bool,          // default: false
}
```

---

## Footer System

Footer text is generated based on current view:

| View | Footer Format |
|------|--------------|
| Block View, collapsed | `Block #3/12  j/k move  Enter expand  i detail  g/G top/btm  q quit` |
| Detail View, short output | `Detail #3   q back   yc cmd   yo output   yb block` |
| Detail View, long output | `Detail #3   ↑↓ scroll   g/G top/bottom   q back   line 23/150` |
| Flash message (any) | `copied output` / `copied command` / `copied block` (1.5s transient) |

Flash messages override the normal footer. They expire after 1500ms and the next render restores the correct footer.
