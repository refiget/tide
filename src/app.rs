#![allow(dead_code)]

use std::{path::PathBuf, time::SystemTime};

pub type BlockId = u64;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    NormalCommand,
    FailedCommand,
    TuiSession,
    AiGenerated,
    SystemEvent,
}

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
