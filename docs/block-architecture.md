# Block Architecture

This document describes Tide's block layer: the data model, lifecycle, rendering pipeline, and view systems that turn shell command history into structured, navigable blocks.

---

## Core Data Structures

### BlockId

```rust
// src/app.rs
pub struct BlockId(pub u64);
```

A newtype wrapper around `u64`. Each command execution gets a unique, monotonically increasing ID. Implements `Display`, `Hash`, `Ord`. Display format: `[42]` (square brackets).

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
    NormalCommand,   // ordinary command
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
    ├── Sets status=Failed on non-zero exit
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
    BlockBodyLine { text: String, block_id: BlockId, selected: bool, in_visual: bool },

    /// Top border of a block frame (Block View)
    BlockTopBorder {
        block_id: BlockId,
        selected: bool,
        in_visual: bool,
        label: TopLabel,
        match_query: String,
    },

    /// Bottom border of a block frame (Block View)
    BlockBottomBorder { block_id: BlockId, selected: bool, in_visual: bool, label: String },

    /// One line of inline detail metadata (Block View, expanded blocks, or Detail View)
    BlockDetailLine {
        block_id: BlockId,
        text: String,
        selected: bool,
        in_visual: bool,
        in_detail_view: bool,
    },

    /// Top border in Detail View (no highlight)
    DetailTopBorder { block_id: BlockId, label: String },

    /// Bottom border in Detail View (no highlight)
    DetailBottomBorder { block_id: BlockId, label: String },

    /// Body line in Detail View, with cursor and visual-selection highlight
    StyledDetailBodyLine {
        block_id: BlockId,
        styled: StyledText,
        plain_text: String,
        is_cursor: bool,
        is_visual: bool,
    },

    /// Body line in Block View with ANSI styled spans
    StyledBlockBodyLine {
        block_id: BlockId,
        styled: StyledText,
        plain_text: String,
        selected: bool,
        in_visual: bool,
    },

    /// Footer bar (segments at screen bottom)
    Footer { segments: Vec<FooterSegment> },
}
```

### FooterSegment

```rust
// src/app.rs
pub enum FooterSegment {
    Label(String),   // descriptive text (dimmed)
    Key(String),     // keyboard shortcut (accent color)
    Sep,             // vertical separator " | "
    Plain(String),   // raw text
    Spacer,          // right-justified fill
}
```

`FooterSegment::flatten()` concatenates segments into a flat string (used in tests).

### Variant semantics — `selected` vs `is_cursor`

| Variant | Border Color | Body BG | Text FG | Used in |
|---------|-------------|---------|---------|---------|
| `BlockTopBorder` `selected=true` | LAVENDER (`BORDER_SELECTED_FG`) | None | per-status | Block View |
| `BlockTopBorder` `selected=false` | SURFACE2 (`BORDER_NORMAL_FG`) | None | per-status | Block View |
| `BlockTopBorder` `in_visual=true` | YELLOW (`VISUAL_BORDER_FG`) | None | per-status | Block View |
| `BlockBodyLine` `selected=true` | LAVENDER | None | TEXT | Block View |
| `BlockBodyLine` `selected=false` | SURFACE2 | None | SUBTEXT1 | Block View |
| `BlockBodyLine` `in_visual=true` | YELLOW | None | SUBTEXT1 | Block View |
| `StyledBlockBodyLine` | same as BlockBodyLine | per span | per span | Block View |
| `BlockDetailLine` `in_detail_view=true` | LAVENDER (`DETAIL_BORDER_FG`) | None | per field | Detail View |
| `BlockDetailLine` `in_detail_view=false` | per selection style | None | per field | Block View (expanded) |
| `DetailTopBorder` / `DetailBottomBorder` | LAVENDER (`DETAIL_BORDER_FG`) | None | per field | Detail View |
| `StyledDetailBodyLine` `is_cursor=true` | LAVENDER | SURFACE1 (`CURSOR_BG`) | per span | Detail View |
| `StyledDetailBodyLine` `is_visual=true` | LAVENDER | SURFACE1 (`VISUAL_LINE_BG`) | per span | Detail View |
| `StyledDetailBodyLine` neither | LAVENDER | None | per span | Detail View |

**Key design rules**:
- Selection uses `BlockSelectionStyle` which maps `selected`/`in_visual` to themed colors.
- No body background fill for selection in Block View (`body_bg: None`).
- `selected=true` does **not** use `Attribute::Reverse` — all highlighting is via `BlockSelectionStyle` border colors.
- `in_visual=true` overrides `selected` — all blocks in the visual range get YELLOW borders.
- Detail View cursor uses a background color (`SURFACE1`) instead of reverse video.

### BlockSelectionStyle

```rust
// src/renderer.rs (internal to render_line dispatch)
struct BlockSelectionStyle {
    border_fg: Color,
    body_bg: Option<Color>,
    text_fg: Color,
}

impl BlockSelectionStyle {
    fn selected() -> Self { /* border_fg: LAVENDER, body_bg: None, text_fg: TEXT */ }
    fn normal() -> Self   { /* border_fg: SURFACE2, body_bg: None, text_fg: SUBTEXT1 */ }
    fn visual() -> Self   { /* border_fg: YELLOW,  body_bg: None, text_fg: SUBTEXT1 */ }
    fn from_state(selected: bool, in_visual: bool) -> Self { /* visual wins over selected */ }
}
```

`from_state` is the single dispatch point used by all 5 Group-A render functions.

---

## ViewState — Navigation and Display State

```rust
// src/app.rs
pub struct ViewState {
    pub view: ViewKind,                    // Plain | Blocks | Detail | Help | Agent | RawProgram
    pub selected_block: Option<BlockId>,   // currently highlighted block (Block View)
    pub expanded_block: Option<BlockId>,   // currently expanded block (enter/exit expand mode)
    pub scroll_offset: usize,              // deprecated (kept for transition)
    pub block_viewport: BlockViewport,     // scroll position, anchor, selection index
    pub detail_line_cursor: usize,         // 0-indexed cursor line in Detail View
    pub filter: BlockFilter,               // failed-only / command search filter
    pub visible: VisibleSource,            // AllTimeline or Filtered(Vec<BlockId>)
    pub search_buffer: Option<String>,     // non-None while search bar is open
    pub pre_search_query: String,          // saved filter before search, restored on Esc
    pub help: Option<HelpState>,           // non-None while Help overlay is open
    pub confirm: Option<ConfirmState>,     // non-None while a confirmation dialog is open
    pub visual_anchor: Option<BlockId>,    // anchor block for Block View visual mode
    pub detail_visual_anchor: Option<usize>, // anchor line for Detail View visual selection
}

pub struct BlockViewport {
    pub selected_index: usize,  // the index within visible[] of selected_block
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

### BlockFilter

```rust
pub struct BlockFilter {
    pub failed_only: bool,
    pub command_query: String,
}
```

Toggled via `f` key. When `command_query` is non-empty, substring token-matching filters visible blocks.

### VisibleSource

```rust
pub enum VisibleSource {
    AllTimeline,                      // no filter: full BlockStore.timeline
    Filtered(Vec<BlockId>),           // pre-computed result in timeline order
}
```

Navigation always uses `visible.ids(blocks)` — never indexes into `BlockStore.timeline` directly.

### HelpState

```rust
pub struct HelpState {
    pub cursor: usize,
    pub scroll: usize,
    pub return_view: ViewKind,      // Blocks or Detail — restored on close
    pub underlying_rendered: bool,  // true after first render with suppressed selection
}
```

### ConfirmState

```rust
pub struct ConfirmState {
    pub kind: ConfirmKind,            // DeleteBlock | DeleteBlocks | RerunBlocks
    pub block_ids: Vec<BlockId>,
}

pub enum ConfirmKind {
    DeleteBlock,
    DeleteBlocks,
    RerunBlocks,
}
```

Rendered as a floating modal on top of the current view. `y`/`Enter` confirms, any other key dismisses.

### ExpansionMode (implicit, no enum)

Whether a block is "expanded" is derived from the relation between `expanded_block` and `selected_block`:

```
expanded_block = None        → Collapsed mode
expanded_block = Some(id)    → FollowSelection mode
```

In **FollowSelection mode**, `expanded_block` automatically updates to match `selected_block` on every navigation:

```rust
if state.view.expanded_block.is_some() {
    state.view.expanded_block = state.view.selected_block;
}
```

### Navigation Flow

```
Plain  ──Ctrl-B──►  Blocks  ──i──►  Detail
  ▲                  │  ▲              │
  │                  │  │              │
  │                  │  └── ? ──► Help ──── ?/q/Esc ──► return
  │                  │              ▲
  │                  └── ? ─────────┘
  └──q/Esc───────────┘  └────q/Esc─────┘
```

| Key | Block View | Detail View |
|-----|-----------|-------------|
| `j`/`k` or Up/Down | Accumulate delta, move block selection | Move cursor line (auto-scrolls viewport) |
| `g` | Jump to oldest block | Jump to output top |
| `G` | Jump to newest block (Tail) | Jump to output bottom |
| `Enter` | Toggle expand/collapse current block | — |
| `i` | Enter Detail View on current block | — |
| `c` | Copy command | Copy command |
| `o` | Copy output | Copy output (respects visual line selection) |
| `y` | Copy command + output | Copy command + output (respects visual selection) |
| `v` | Toggle visual block selection mode | Toggle visual line selection mode |
| `V` | — | Toggle visual line selection mode (same as v) |
| `d` | Delete block(s) with confirm | — |
| `n`/`N` | Next/prev filtered result | — |
| `/` | Open search bar | — |
| `f` | Toggle failed-only filter | — |
| `Ctrl-u`/`Ctrl-d` | Scroll half screen up/down | — |
| `Ctrl-b`/`Ctrl-f` | Scroll full screen up/down | — |
| `?` | Open Help overlay | Open Help overlay |
| `r` | Exit to Plain + paste command to PTY | Exit to Plain + paste command to PTY |
| `q`/`Esc` | Exit to Plain (or exit visual mode first) | Return to Block View |

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
render_runtime (pty.rs:605)
  │
  ▼
build_visual_lines (compositor.rs:124)
  ├── ViewKind::Plain  → ShellText lines
  ├── ViewKind::Blocks  → build_block_lines
  │     │
  │     ├── build_visual_layout (compositor.rs:226)
  │     │     ├── Computes visual_range from visual_anchor → selected_block
  │     │     ├── Iterates view.visible.ids(blocks), calls build_one_block_lines for each
  │     │     ├── Returns VisualLayout { lines, spans, total_height }
  │     │     └── build_one_block_lines (compositor.rs:333)
  │     │           ├── BlockTopBorder (selected, in_visual, label: TopLabel, match_query)
  │     │           ├── Body: BlockBodyLine or StyledBlockBodyLine × N
  │     │           │     ├── Collapsed (expanded_block ≠ block_id): preview_lines rows
  │     │           │     │     └── Truncation hint: "... N more lines, Enter to expand"
  │     │           │     └── Expanded (expanded_block == block_id): expanded_lines rows
  │     │           │           └── Truncation hint: "... N more lines · i to inspect in Detail"
  │     │           ├── [If expanded]: BlockDetailLine × N (metadata with Nerd Font icons)
  │     │           └── BlockBottomBorder + gap Empty lines
  │     │
  │     ├── slice_visible_lines (compositor.rs:279)
  │     │     └── Clips VisualLayout by viewport.line_offset, fills Empty padding
  │     │
  │     └── Footer { segments: footer_segments() }
  │           Normal: [Spacer] [Label"Keybindings: "] [Key"?"]
  │           Filtered: ["query" · failed] [Spacer] [Label"Keybindings: "] [Key"?"]
  │           Search: [Plain"/query█"] [Sep] [Label"Apply: "] [Key"Enter"] [Sep] [Label"Cancel: "] [Key"Esc"]
  │           Flash: [Plain"copied output"]
  │
  ├── ViewKind::Detail → build_detail_lines (compositor.rs:530)
  │     ├── Empty (top margin / centered padding)
  │     ├── DetailTopBorder { label }
  │     ├── StyledDetailBodyLine × N (output lines, is_cursor + is_visual highlights)
  │     ├── BlockDetailLine × N (metadata with Nerd Font icons)
  │     ├── DetailBottomBorder { label }
  │     └── Footer { segments: detail_footer_segments() }
  │           Short: [Plain""] [Spacer] [Label"Keybindings: "] [Key"?"]
  │           Long:  [Plain"cursor/total"] [Spacer] [Label"Keybindings: "] [Key"?"]
  │           Flash: [Plain"copied output"]
  │
  ├── ViewKind::Help → rendered underlying view + overlay
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
- Footer: empty left segment

**Long mode** (`total_output_lines > inner_height`):
- Frame fills screen; `inner_height = rows - 4 - meta_count` (top_margin + top_border + meta_lines + bottom_border + footer)
- Output lines start at `line_offset`, fill `inner_height` rows
- Footer shows `cursor/total`

### Vertical Centering (Short Mode)

```rust
let frame_height = 2 + styled_output_lines.len() + meta_count;
let available = height - 1;                // minus footer
let top_padding = max((available - frame_height) / 2, 1);
```

### Detail Metadata (Expanded / Detail View)

Inline metadata is rendered as `BlockDetailLine` variants with `in_detail_view` flag. The text payload uses a key: value format with Nerd Font icons:

```
│ 󰋼 Detail ──────────────────────────────────────────────── │
│                                                            │
│ 󰘧 command: cargo test                                      │
│ 󰉋 cwd: ~/Projects/tide                                     │
│ 󰄬 status: ok                                              │
│ 󰔟 duration: 12.3s                                         │
│                                                            │
│ 󰘳 actions: 󰆏 c copy   󰉆 o output   󰈚 y block   󰑓 r rerun │
```

Exit code and status are merged into a single `status:` row. Empty separator lines are rendered as solid `│ … │`.

### Detail Visual Line Selection

Press `v`/`V` in Detail View to toggle visual line selection. The cursor position becomes the anchor. Moving `j`/`k` extends the range. All lines in the range get `VISUAL_LINE_BG` background. Copy via `o` copies only the selected lines.

---

## Help Overlay

### Entry

`?` in Block View or Detail View → sets `view = ViewKind::Help`, creates `HelpState` with `return_view` set to the current view.

### Rendering

The Help overlay is a floating box centered on screen:

```
╭────────────────────── Keybindings ────────────────────────╮
│  j / k                 navigate blocks                     │
│  Ctrl-u / Ctrl-d       scroll half screen                  │
│  g / G                 top / bottom                        │
│  Enter                 expand / collapse                   │
│  ...                                                       │
│                         15 of 18                           │
╰───────────────────────────────────────────────────────────╯
```

**Rendering strategy** (flicker-free):
1. First render: compositor builds the underlying view with selection suppressed (`selected_block = None`), then the overlay is drawn on top in the same flush. `HelpState::underlying_rendered = true`.
2. Subsequent navigations: only the overlay is redrawn (`render_help_overlay`), skipping the full underlying render.

### Keybindings

| Key | Action |
|-----|--------|
| `j`/`k` | Scroll entries |
| `g`/`G` | Top/bottom |
| `?`/`q`/`Esc` | Close, return to previous view |

Two entry sets: `BLOCK_HELP_ENTRIES` and `DETAIL_HELP_ENTRIES`, selected based on `return_view`.

---

## Confirm Dialog

### Entry

Destructive or multi-block actions (`d` delete, `r` rerun with visual selection) set `view.confirm = Some(ConfirmState)`.

### Rendering

A floating modal box rendered **after** the main view (on top):

```
╭────────────────────── Confirm ────────────────────────────╮
│ Delete block [42]?                                         │
│ This cannot be undone.                                     │
│                                                            │
├────────────────────────────────────────────────────────────┤
│ [Y]es                                       (N)o          │
╰────────────────────────────────────────────────────────────╯
```

### Keybindings

| Key | Action |
|-----|--------|
| `y`/`Y`/`Enter` | Confirm action |
| any other key | Dismiss dialog |

---

## Visual Selection Mode (Block View)

### Entry

Press `v` in Block View. Sets `visual_anchor = Some(selected_block)`. All blocks from anchor to cursor get `in_visual = true`, rendering with YELLOW borders.

### Behavior

- Moving `j`/`k` extends or shrinks the visual range.
- Copy actions (`c`/`o`/`y`) copy all blocks in the range, then exit visual mode.
- `d` shows a confirm dialog for all blocks in the range.
- `r` reruns commands sequentially (confirm dialog for multi-block).
- `q`/`Esc` exits visual mode (first press) or exits Block View (second press).

### Visual

```
╭─ [7] ✗  cargo test  ~/Projects/tide ──────────────────────╮  ← YELLOW border
│ test result: FAILED. 0 passed; 1 failed                    │
╰─ 󰅙 fail · exit 101 · 12.3s · 5m ago ──────────────────────╯  ← YELLOW border
╭─ [8]  ls  ~/Projects/tide ────────────────────────────────╮  ← YELLOW border
│ src/  docs/  Cargo.toml                                    │
╰─ 󰄬 ok · 0.1s · 5m ago ──────────────────────────────────────╯  ← YELLOW border
```

---

## Top Border Label

### TopLabel struct

```rust
// src/format.rs
pub struct TopLabel {
    pub id_marker: String,   // "[42]" or "[42] ✗" for failed
    pub command: String,     // compacted command text
    pub cwd: Option<String>, // compacted working directory
    pub status: BlockStatus,
}
```

Built by `build_top_label_parts()`. ID format uses `[N]` (square brackets). Failed blocks append `✗` after the ID.

### Rendering (render_top_border)

```
╭─ [42]  cargo test  ~/Projects/tide ───────────────────────╮
```

The command text uses search-match highlighting: when `match_query` is non-empty, matching tokens render with `SEARCH_MATCH_FG` (YELLOW).

---

## Bottom Border Label

### Format

Bottom labels use Nerd Font icons and relative timestamps:

| Status | Label |
|--------|-------|
| Running | `󰔟 running · 12.3s` |
| Success | `󰄬 ok · 1.2s · 5m ago` |
| Failed | `󰅙 fail · exit 1 · 2.1s · 5m ago` |
| Interrupted | `󰅙 cancelled · 0.5s · 5m ago` |
| Unknown | `? unknown · 0.5s · 5m ago` |
| Truncated | `· truncated` appended |

Timestamps use relative format: `Xs ago`, `Xm ago`, `Xh ago`.

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
  ├── [if Help]: render_help_overlay(w, view, cols, rows)
  └── [if Confirm]: render_confirm_overlay(w, view, cols, rows)
  └── w.flush()
```

### Variant → Rendering Map

| Variant | Renderer Handler | Visual Effect |
|---------|-----------------|---------------|
| `Empty` | no-op | Nothing drawn |
| `ShellText` | `Print(text)` | Plain text with optional horizontal padding |
| `BlockBodyLine` | `render_framed_text(w, text, style, ...)` | `│ colored │` with themed border color |
| `BlockTopBorder` | `render_top_border(w, label, query, style, ...)` | `╭─ [N] cmd ~/cwd ─╮` with themed border + search highlights |
| `BlockBottomBorder` | `render_border(w, label, style, false, ...)` | `╰─ icon · dur ─╯` with themed border |
| `BlockDetailLine` | `render_block_detail_line(w, text, style, in_dv, ...)` | `│ icon label: value │` with field-specific icons and colors |
| `StyledBlockBodyLine` | `render_styled_framed_text(w, styled, ..., border_fg, ...)` | `│ styled spans │` with per-span ANSI colors |
| `DetailTopBorder` | `SetFg(DETAIL_BORDER_FG) + Print(╭─ label ─╮)` | `╭─ label ─╮`, always LAVENDER |
| `DetailBottomBorder` | `SetFg(DETAIL_BORDER_FG) + Print(╰─ label ─╯)` | `╰─ label ─╯`, always LAVENDER |
| `StyledDetailBodyLine` | `render_styled_framed_text(w, styled, ..., bg, border_fg, ...)` | `│ styled │` with cursor/visual background |
| `Footer` | `render_footer(w, segments, width)` | Segment-based layout with Spacer, Label, Key, Sep colors |

### BlockSelectionStyle Dispatch

All Block View render functions go through `BlockSelectionStyle::from_state(selected, in_visual)`:
- `in_visual=true` → `visual()` → YELLOW border
- `selected=true` → `selected()` → LAVENDER border
- otherwise → `normal()` → SURFACE2 border

### Help Overlay Rendering

```rust
fn render_help_overlay(w, view, cols, rows)
  ├── Computes box dimensions (56-wide, centered)
  ├── Draws titled border with "Keybindings" header
  ├── For each visible entry: key column + description column
  ├── Footer row: "X of N" counter
  └── Bottom border
```

### Confirm Dialog Rendering

```rust
fn render_confirm_overlay(w, view, cols, rows)
  ├── Computes box dimensions (44-wide, centered)
  ├── Draws titled border with "Confirm" header
  ├── Message row ("Delete block [42]?" or "Delete [3] blocks?")
  ├── Hint row ("This cannot be undone.")
  ├── Blank row + divider
  ├── Actions row: "[Y]es" + fill + "(N)o"
  └── Bottom border
```

### Blink-Free Rendering

- No `Clear(All)` — each row is individually overwritten
- `last_rendered_rows` tracks previous frame's line count
- If new frame is shorter, stale tail rows are cleared via `Clear(CurrentLine)`
- Footer is always on the last rendered row
- Help overlay is composited in a single flush with its underlying view (no flicker)

---

## Key Terminology

| Term | Definition |
|------|-----------|
| **Block** | One shell command execution with its output, metadata, and lifecycle |
| **Block expansion** | Inline toggle within Block View: `Enter` shows/hides all output + metadata for the selected block. `expanded_block` follows the selection. |
| **Detail View** | Full-screen pager (`ViewKind::Detail`), entered via `i`. Shows one block with line-cursor scrolling. |
| **Collapsed** | Default state: each block shows `preview_lines` (default 4) of output |
| **Expanded** | FollowSelection mode: selected block shows `expanded_lines` (default 15) + metadata |
| **FollowSelection** | When a block is expanded, navigation (j/k/g/G) automatically updates `expanded_block` to match `selected_block` |
| **View Anchor** | Controls viewport alignment: Top (oldest), Tail (newest/follow), Manual (user-scrolled) |
| **Visual Selection** | Block View mode (`v`) where a range of blocks is highlighted with YELLOW borders. Copy/delete/rerun operates on all selected blocks. |
| **Help Overlay** | Floating keybinding reference box, opened via `?` from Block or Detail View |
| **Confirm Dialog** | Floating modal for destructive actions (delete, multi-rerun), confirmed with `y`/`Enter` |

---

## Config (Block View)

```rust
// src/config.rs
pub struct BlockViewConfig {
    pub preview_lines: usize,                 // default: 4
    pub expanded_lines: usize,                // default: 15
    pub follow_tail: bool,                    // default: true
    pub block_gap: usize,                     // default: 0
    pub scroll_margin_blocks: usize,          // default: 2
    pub scroll_margin_lines: usize,           // default: 2
    pub auto_follow_on_reach_bottom: bool,    // default: false
    pub horizontal_margin: usize,             // default: 1
    pub body_padding: usize,                  // default: 1
    pub show_footer: bool,                    // default: true
    pub copy_format: CopyFormat,              // default: Plaintext
}
```

### CopyFormat

```rust
pub enum CopyFormat {
    Plaintext,          // default
    Markdown,
    ShellTranscript,
    Json,
}
```

Controlled by `copy_format` in TOML config. Serializes block command/output for clipboard operations.

---

## Copy System

### Single Key Shortcuts

| Key | Part | Detail View behavior |
|-----|------|---------------------|
| `c` | Command | Copies command text |
| `o` | Output | Copies output (or visual selection lines in Detail) |
| `y` | Both | Copies command + output (or command + visual selection in Detail) |

### Visual Range Copy

When visual mode is active in Block View, `c`/`o`/`y` operate on all blocks in the range. The flash message reflects the count: `"copied 3 commands"`.

### CopyFormat Flash

When a non-default format is active, flash appends the format name: `"copied block · markdown"`.

---

## Footer System

Footer is built from `FooterSegment` values. The `Spacer` segment right-justifies everything after it. `Label` renders in `FOOTER_FG`, `Key` in `FOOTER_KEY_FG`, `Sep` as ` | ` in `FOOTER_SEP_FG`.

| View State | Segments |
|-----------|----------|
| Block View, normal | `[Spacer] [Label"Keybindings: "] [Key"?"]` |
| Block View, filtered | `[Plain"query · failed"] [Spacer] [Label"Keybindings: "] [Key"?"]` |
| Block View, search open | `[Plain"/text█"] [Sep] [Label"Apply: "] [Key"Enter"] [Sep] [Label"Cancel: "] [Key"Esc"]` |
| Block View, flash | `[Plain"copied output"]` (or `"copied command"`, `"copied block"`, `"copied 3 commands"`) |
| Detail View, short | `[Plain""] [Spacer] [Label"Keybindings: "] [Key"?"]` |
| Detail View, long | `[Plain"23/150"] [Spacer] [Label"Keybindings: "] [Key"?"]` |
| Detail View, flash | `[Plain"copied output"]` |
| Help overlay | N/A (overlay drawn on top) |
| Confirm dialog | N/A (overlay drawn on top) |

Flash messages override the normal footer. They expire after 1500ms and the next render restores the correct footer.
