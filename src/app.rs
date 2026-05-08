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
    /// Transient flash message (e.g. "copied output") shown in the footer
    /// for ~1.5 seconds. Reset to None after the duration expires.
    pub flash_message: Option<(String, Instant)>,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            dirty: false,
            force_render: false,
            last_render_at: Instant::now(),
            needs_cleanup: false,
            flash_message: None,
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
