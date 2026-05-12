#![allow(dead_code)]

use std::{
    fmt,
    path::PathBuf,
    time::{Instant, SystemTime},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u64);

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub mode: AppMode,
    pub blocks: Vec<CommandBlock>,
    pub current_cwd: PathBuf,
}

impl App {
    pub fn new(current_cwd: PathBuf) -> Self {
        Self {
            mode: AppMode::ShellIdle,
            blocks: Vec::new(),
            current_cwd,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockViewAction {
    NavDown,
    NavUp,
    NavTop,
    NavBottom,
    ScrollHalfDown,
    ScrollHalfUp,
    ScrollFullDown,
    ScrollFullUp,
    Expand,
    DetailView,
    ToggleFailedFilter,
    OpenSearch,
    CopyCommand,
    CopyOutput,
    CopyBoth,
    Rerun,
    Delete,
    VisualMode,
    SearchNext,
    SearchPrev,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailViewAction {
    NavDown,
    NavUp,
    NavTop,
    NavBottom,
    ScrollHalfDown,
    ScrollHalfUp,
    ScrollFullDown,
    ScrollFullUp,
    CopyCommand,
    CopyOutput,
    CopyBoth,
    Rerun,
    VisualMode,
    Help,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    ShellIdle,
    CommandRunning,
    TuiHandoff,
    Returning,
    BlockInteraction,
    ReturnPanel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Plain,
    Blocks,
    Detail,
    Help,
    ReturnPanel,
    Agent,
    RawProgram,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Shell,
    BlockNav,
    DetailNav,
    NaturalLanguage,
    OpenCode,
    RawProgram,
}

/// The set of BlockIds currently visible in Block View.
/// Navigation always uses this; never indexes into BlockStore.timeline directly.
#[derive(Debug, Clone)]
pub enum VisibleSource {
    /// No filter active: implicitly the full BlockStore timeline.
    AllTimeline,
    /// Filter active: pre-computed result in timeline order.
    Filtered(Vec<BlockId>),
}

impl VisibleSource {
    /// Slice of visible BlockIds.
    pub fn ids<'a>(&'a self, blocks: &'a crate::block::BlockStore) -> &'a [BlockId] {
        match self {
            VisibleSource::AllTimeline => &blocks.timeline,
            VisibleSource::Filtered(v) => v.as_slice(),
        }
    }

    pub fn len(&self, blocks: &crate::block::BlockStore) -> usize {
        self.ids(blocks).len()
    }

    pub fn is_empty(&self, blocks: &crate::block::BlockStore) -> bool {
        self.ids(blocks).is_empty()
    }

    pub fn block_at(&self, blocks: &crate::block::BlockStore, idx: usize) -> Option<BlockId> {
        self.ids(blocks).get(idx).copied()
    }

    pub fn index_of(&self, blocks: &crate::block::BlockStore, id: BlockId) -> Option<usize> {
        self.ids(blocks).iter().position(|&b| b == id)
    }
}

impl Default for VisibleSource {
    fn default() -> Self {
        VisibleSource::AllTimeline
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlockFilter {
    pub failed_only: bool,
    pub command_query: String,
}

impl BlockFilter {
    pub fn is_active(&self) -> bool {
        self.failed_only || !self.command_query.is_empty()
    }
}

/// State for a confirmation dialog. Present only while the dialog is open.
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub kind: ConfirmKind,
    /// All block ids this action covers. Always at least one element.
    pub block_ids: Vec<BlockId>,
}

impl ConfirmState {
    pub fn single(kind: ConfirmKind, id: BlockId) -> Self {
        Self {
            kind,
            block_ids: vec![id],
        }
    }
    pub fn multi(kind: ConfirmKind, ids: Vec<BlockId>) -> Self {
        Self {
            kind,
            block_ids: ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmKind {
    DeleteBlock,
    DeleteBlocks,
    RerunBlocks,
}

/// State for the Help overlay. Present only while the overlay is open.
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
    pub fn open(return_view: ViewKind) -> Self {
        Self {
            cursor: 0,
            scroll: 0,
            return_view,
            underlying_rendered: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ViewState {
    pub view: ViewKind,
    pub selected_block: Option<BlockId>,
    pub expanded_block: Option<BlockId>,
    pub scroll_offset: usize,
    pub block_viewport: BlockViewport,
    /// 0-indexed cursor line within the Detail View output.
    pub detail_line_cursor: usize,
    pub filter: BlockFilter,
    pub visible: VisibleSource,
    pub search_buffer: Option<String>,
    /// Saved filter.command_query before opening the search bar; restored on Esc.
    pub pre_search_query: String,
    /// Non-None while the Help overlay is open.
    pub help: Option<HelpState>,
    /// Non-None while a confirmation dialog is open.
    pub confirm: Option<ConfirmState>,
    /// Anchor block for Block View visual selection (v mode). None = not in visual mode.
    pub visual_anchor: Option<BlockId>,
    /// Anchor line for Detail View visual selection (v mode). None = not in visual mode.
    pub detail_visual_anchor: Option<usize>,
    /// Non-None while the Return Panel overlay is showing.
    pub return_panel: Option<ReturnPanelState>,
}

#[derive(Debug, Clone)]
pub struct BlockViewport {
    pub selected_index: usize,
    pub line_offset: usize,
    /// Deprecated: old block-index offset. New Block View rendering uses line_offset.
    pub scroll_offset: usize,
    pub anchor: ViewAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAnchor {
    Top,
    Tail,
    Manual,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            view: ViewKind::Plain,
            selected_block: None,
            expanded_block: None,
            scroll_offset: 0,
            block_viewport: BlockViewport::default(),
            detail_line_cursor: 0,
            filter: BlockFilter::default(),
            visible: VisibleSource::default(),
            search_buffer: None,
            pre_search_query: String::new(),
            help: None,
            confirm: None,
            visual_anchor: None,
            detail_visual_anchor: None,
            return_panel: None,
        }
    }
}

impl Default for BlockViewport {
    fn default() -> Self {
        Self {
            selected_index: 0,
            line_offset: 0,
            scroll_offset: 0,
            anchor: ViewAnchor::Tail,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct InputAccumulator {
    pub pending_block_delta: isize,
    pub last_input_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct RenderState {
    pub dirty: bool,
    pub force_render: bool,
    pub last_render_at: Instant,
    /// Set true when leaving Block/Detail view so the input thread performs
    /// terminal cleanup (leave alternate screen, reset SGR, show cursor).
    pub needs_cleanup: bool,
    /// Set true when a TUI crash is detected in precmd, forcing alt-screen cleanup
    /// even if Tide's own UI was already closed.
    pub force_pty_alt_screen_cleanup: bool,
    /// Transient flash message (e.g. "copied output") shown in the footer
    /// for ~1.5 seconds. Reset to None after the duration expires.
    pub flash_message: Option<(String, Instant)>,
    /// Number of rows rendered in the previous frame, used to clear stale
    /// tail lines when the new frame is shorter than the previous one.
    pub last_rendered_rows: usize,
    /// Command text pending paste to PTY after alt-screen cleanup (Rerun).
    /// Set by the r key handler, consumed by the cleanup handler.
    pub pending_paste: Option<String>,
    /// Whether Tide is currently inside its own alternate screen buffer.
    pub in_tide_alt_screen: bool,
    /// Actions to execute on the main screen before entering Tide alt screen.
    pub pending_terminal_actions: Vec<TerminalAction>,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            dirty: false,
            force_render: false,
            last_render_at: Instant::now(),
            needs_cleanup: false,
            force_pty_alt_screen_cleanup: false,
            flash_message: None,
            last_rendered_rows: 0,
            pending_paste: None,
            in_tide_alt_screen: false,
            pending_terminal_actions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    KeyInput(Vec<u8>),
    PtyOutput(Vec<u8>),
    ShellPreexec {
        command: String,
    },
    ShellPrecmd {
        exit_code: i32,
    },
    CwdChanged {
        cwd: String,
    },
    CommandStarted {
        block_id: BlockId,
        command: String,
    },
    CommandOutput {
        block_id: BlockId,
        bytes: Vec<u8>,
    },
    CommandFinished {
        block_id: BlockId,
        exit_code: i32,
    },
    TuiAppMatched {
        command: String,
        app_name: String,
    },
    TuiAppExited {
        command: String,
        exit_code: i32,
    },
    BlockSelected {
        block_id: BlockId,
    },
    BlockActionRequested {
        block_id: BlockId,
        action: BlockAction,
    },
    ReturnStarted {
        block_id: BlockId,
    },
    ReturnFinished {
        block_id: BlockId,
    },
    Tick,
    Resize {
        cols: u16,
        rows: u16,
    },
    Shutdown,
}

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
    /// App name when this block represents a known TUI session (e.g. "lazygit").
    pub app_name: Option<String>,
}

impl Default for CommandBlock {
    fn default() -> Self {
        Self {
            id: BlockId(0),
            command: String::new(),
            cwd: PathBuf::new(),
            started_at: std::time::UNIX_EPOCH,
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            output_raw: Vec::new(),
            output_text: String::new(),
            kind: BlockKind::NormalCommand,
            status: BlockStatus::Success,
            git_context: None,
            suggestions: Vec::new(),
            start_line: 0,
            end_line: 0,
            output_truncated: false,
            app_name: None,
        }
    }
}

pub type ExecutionBlock = CommandBlock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    NormalCommand,
    FailedCommand,
    TuiSession,
    RawProgram,
    AiGenerated,
    SystemEvent,
}

pub type ExecutionKind = BlockKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockStatus {
    Running,
    Success,
    Failed,
    Interrupted,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct TuiSession {
    pub app_name: String,
    pub command: String,
    pub cwd_before: PathBuf,
    pub cwd_after: Option<PathBuf>,
    pub started_at: SystemTime,
    pub finished_at: Option<SystemTime>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub snapshot_before: Option<SessionSnapshot>,
    pub snapshot_after: Option<SessionSnapshot>,
    pub after_exit_results: Vec<AfterExitResult>,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub cwd: PathBuf,
    pub git_branch: Option<String>,
    pub git_status_short: Option<String>,
    pub git_diff_stat: Option<String>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AfterExitResult {
    pub command: String,
    pub exit_code: i32,
    pub output_text: String,
}

#[derive(Debug, Clone)]
pub struct GitContext {
    pub branch: Option<String>,
    pub status_short: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SuggestedAction {
    pub label: String,
    pub command: Option<String>,
}

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

#[derive(Debug, Clone)]
pub enum FooterSegment {
    Label(String),
    Key(String),
    Sep,
    Plain(String),
    Spacer,
}

// ─── TUI App Detection ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TuiAppMatch {
    pub app_name: String,
    pub command_name: String,
    pub source: TuiAppMatchSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiAppMatchSource {
    Builtin,
    UserConfig,
}

/// Default list of well-known TUI / full-screen programs.
/// These are commands that enter alternate screen or take over the terminal.
pub const DEFAULT_TUI_COMMANDS: &[&str] = &[
    // editors
    "vim",
    "vi",
    "nvim",
    "nano",
    "emacs",
    "hx",
    "micro",
    // git / dev TUI
    "lazygit",
    "tig",
    "gitui",
    "gh-dash",
    // file managers
    "ranger",
    "lf",
    "nnn",
    "yazi",
    "vifm",
    "mc",
    // fuzzy finders
    "fzf",
    "sk",
    "peco",
    // pagers / docs
    "less",
    "more",
    "most",
    "man",
    "info",
    // monitors
    "htop",
    "btop",
    "btm",
    "glances",
    // multiplexers
    "tmux",
    "screen",
    "zellij",
    // infra TUIs
    "lazydocker",
    "k9s",
    "ctop",
];

// ─── Alt-Screen Lifecycle ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    Normal,
    SuspendedForTui,
}

/// States for the TUI full-screen lifecycle state machine.
///
/// Transitions:
///   Idle → preexec matched TUI → Pending
///   Pending → alt-screen enter → InAltScreen
///   Pending → precmd no alt-screen → Idle (normal command)
///   InAltScreen → alt-screen exit → ExitedAltScreen
///   ExitedAltScreen → precmd → Idle (finalized)
#[derive(Debug, Clone)]
pub enum TuiRuntimeState {
    Idle,
    Pending {
        app_match: TuiAppMatch,
        command: String,
    },
    InAltScreen {
        block_id: BlockId,
    },
    ExitedAltScreen {
        block_id: BlockId,
    },
}

impl Default for TuiRuntimeState {
    fn default() -> Self {
        Self::Idle
    }
}

// ─── Return Panel ──────────────────────────────────────────────────────────

/// Target view after dismissing the Return Panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnPanelTarget {
    #[default]
    None,
    Plain,
    Blocks,
    Detail,
}

/// State carried while the Return Panel overlay is showing.
#[derive(Debug, Clone)]
pub struct ReturnPanelState {
    pub block_id: BlockId,
    pub target: ReturnPanelTarget,
    pub clear_main_screen_before_show: bool,
}

/// Kind of a line in the Return Panel body, used for semantic coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnPanelLineKind {
    Empty,
    Title,
    Field,
    Separator,
    Hint,
}

/// Side-effect action that must run on the main screen
/// (never inside Tide's alternate screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalAction {
    ClearMainScreen,
}

/// Enter Return Panel view with the given state.
pub fn enter_return_panel(view: &mut ViewState, panel: ReturnPanelState) {
    view.return_panel = Some(panel);
    view.view = ViewKind::ReturnPanel;
}

/// Clear Return Panel state and reset view to Plain.
/// Returns the previous state, if any.
pub fn clear_return_panel(view: &mut ViewState) -> Option<ReturnPanelState> {
    let panel = view.return_panel.take();
    if panel.is_some() {
        view.view = ViewKind::Plain;
    }
    panel
}

impl FooterSegment {
    pub fn flatten(segments: &[FooterSegment]) -> String {
        segments
            .iter()
            .map(|s| match s {
                FooterSegment::Label(t) | FooterSegment::Key(t) | FooterSegment::Plain(t) => {
                    t.as_str()
                }
                FooterSegment::Sep => " | ",
                FooterSegment::Spacer => " ",
            })
            .collect()
    }
}
