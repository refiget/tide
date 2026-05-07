use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use strip_ansi_escapes::strip;

use crate::app::{BlockId, BlockKind, BlockStatus, CommandBlock};

#[derive(Debug)]
pub struct BlockStore {
    blocks: VecDeque<CommandBlock>,
    active_block_id: Option<BlockId>,
    next_id: BlockId,
    current_cwd: PathBuf,
    max_blocks: usize,
    max_output_bytes_per_block: usize,
}

impl BlockStore {
    pub fn new(current_cwd: PathBuf, max_blocks: usize, max_output_bytes_per_block: usize) -> Self {
        Self {
            blocks: VecDeque::new(),
            active_block_id: None,
            next_id: 1,
            current_cwd,
            max_blocks,
            max_output_bytes_per_block,
        }
    }

    pub fn start_command(&mut self, command: String) {
        let id = self.next_id;
        self.next_id += 1;

        if self.blocks.len() >= self.max_blocks {
            self.blocks.pop_front();
        }

        self.blocks.push_back(CommandBlock {
            id,
            command,
            cwd: self.current_cwd.clone(),
            started_at: SystemTime::now(),
            finished_at: None,
            duration_ms: None,
            exit_code: None,
            output_raw: Vec::new(),
            output_text: String::new(),
            kind: BlockKind::NormalCommand,
            status: BlockStatus::Running,
            git_context: None,
            suggestions: Vec::new(),
        });
        self.active_block_id = Some(id);
    }

    pub fn append_output(&mut self, bytes: &[u8]) {
        let Some(active_block_id) = self.active_block_id else {
            return;
        };

        let Some(block) = self
            .blocks
            .iter_mut()
            .find(|block| block.id == active_block_id)
        else {
            return;
        };

        let remaining = self
            .max_output_bytes_per_block
            .saturating_sub(block.output_raw.len());
        if remaining == 0 {
            return;
        }

        let to_append = remaining.min(bytes.len());
        block.output_raw.extend_from_slice(&bytes[..to_append]);
    }

    pub fn finish_command(&mut self, exit_code: i32) {
        let Some(active_block_id) = self.active_block_id.take() else {
            return;
        };

        let Some(block) = self
            .blocks
            .iter_mut()
            .find(|block| block.id == active_block_id)
        else {
            return;
        };

        let finished_at = SystemTime::now();
        block.duration_ms = finished_at
            .duration_since(block.started_at)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok());
        block.finished_at = Some(finished_at);
        block.exit_code = Some(exit_code);
        block.status = if exit_code == 0 {
            BlockStatus::Success
        } else {
            block.kind = BlockKind::FailedCommand;
            BlockStatus::Failed
        };
        block.output_text = String::from_utf8_lossy(&strip(&block.output_raw)).to_string();
    }

    pub fn set_cwd(&mut self, cwd: String) {
        self.current_cwd = PathBuf::from(cwd);
    }

    pub fn blocks_newest_first(&self) -> Vec<CommandBlock> {
        self.blocks.iter().rev().cloned().collect()
    }
}

pub fn format_duration_ms(duration_ms: Option<u64>) -> String {
    let Some(duration_ms) = duration_ms else {
        return "running".to_string();
    };

    if duration_ms < 1000 {
        format!("{duration_ms}ms")
    } else {
        format!("{:.1}s", duration_ms as f64 / 1000.0)
    }
}

pub fn format_started_at(time: SystemTime) -> String {
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return "-".to_string();
    };

    duration.as_secs().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::BlockStore;

    #[test]
    fn keeps_only_latest_blocks() {
        let mut store = BlockStore::new(PathBuf::from("/tmp"), 10, 1024);

        for index in 1..=12 {
            store.start_command(format!("echo {index}"));
            store.finish_command(0);
        }

        let blocks = store.blocks_newest_first();

        assert_eq!(blocks.len(), 10);
        assert_eq!(blocks.first().unwrap().command, "echo 12");
        assert_eq!(blocks.last().unwrap().command, "echo 3");
    }
}
