use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use strip_ansi_escapes::strip;

use crate::app::{BlockId, BlockKind, BlockStatus, CommandBlock};

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

impl BlockStore {
    pub fn new(
        current_cwd: PathBuf,
        max_blocks: Option<usize>,
        max_output_bytes_per_block: usize,
    ) -> Self {
        Self {
            next_id: 1,
            timeline: Vec::new(),
            executions: HashMap::new(),
            max_blocks,
            active_block_id: None,
            current_cwd,
            max_output_bytes_per_block,
        }
    }

    pub fn start_command(
        &mut self,
        command: String,
        start_line: usize,
        kind: BlockKind,
    ) -> BlockId {
        let id = BlockId(self.next_id);
        self.next_id += 1;

        if let Some(max_blocks) = self.max_blocks
            && self.timeline.len() >= max_blocks
        {
            if let Some(oldest) = self.timeline.first().copied() {
                self.executions.remove(&oldest);
                self.timeline.remove(0);
            }
        }

        self.timeline.push(id);
        self.executions.insert(
            id,
            CommandBlock {
                id,
                command,
                cwd: self.current_cwd.clone(),
                started_at: SystemTime::now(),
                finished_at: None,
                duration_ms: None,
                exit_code: None,
                output_raw: Vec::new(),
                output_text: String::new(),
                output_truncated: false,
                app_name: None,
                kind,
                status: BlockStatus::Running,
                git_context: None,
                suggestions: Vec::new(),
                start_line,
                end_line: start_line,
            },
        );
        self.active_block_id = Some(id);
        id
    }

    pub fn append_output(&mut self, bytes: &[u8]) {
        let Some(active_block_id) = self.active_block_id else {
            return;
        };

        let Some(block) = self.executions.get_mut(&active_block_id) else {
            return;
        };

        let remaining = self
            .max_output_bytes_per_block
            .saturating_sub(block.output_raw.len());
        if remaining == 0 {
            block.output_truncated = true;
            return;
        }

        let to_append = remaining.min(bytes.len());
        block.output_raw.extend_from_slice(&bytes[..to_append]);
        if to_append < bytes.len() {
            block.output_truncated = true;
        }
    }

    pub fn finish_command(&mut self, exit_code: i32, end_line: usize) {
        let Some(active_block_id) = self.active_block_id.take() else {
            return;
        };

        let Some(block) = self.executions.get_mut(&active_block_id) else {
            return;
        };

        let finished_at = SystemTime::now();
        block.duration_ms = finished_at
            .duration_since(block.started_at)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok());
        block.finished_at = Some(finished_at);
        block.exit_code = Some(exit_code);
        block.end_line = end_line;
        block.status = if exit_code == 0 {
            BlockStatus::Success
        } else {
            if block.kind == BlockKind::NormalCommand {
                block.kind = BlockKind::FailedCommand;
            }
            BlockStatus::Failed
        };
        block.output_text = String::from_utf8_lossy(&strip(&block.output_raw)).to_string();
    }

    pub fn active_block_id(&self) -> Option<BlockId> {
        self.active_block_id
    }

    pub fn block(&self, id: BlockId) -> Option<&CommandBlock> {
        self.executions.get(&id)
    }

    pub fn block_mut(&mut self, id: BlockId) -> Option<&mut CommandBlock> {
        self.executions.get_mut(&id)
    }

    pub fn remove(&mut self, id: BlockId) {
        self.timeline.retain(|&b| b != id);
        self.executions.remove(&id);
    }

    pub fn set_cwd(&mut self, cwd: String) {
        self.current_cwd = PathBuf::from(cwd);
    }

    #[allow(dead_code)]
    pub fn blocks_oldest_first(&self) -> Vec<CommandBlock> {
        self.timeline
            .iter()
            .filter_map(|id| self.executions.get(id).cloned())
            .collect()
    }

    #[allow(dead_code)]
    pub fn block_ids_oldest_first(&self) -> Vec<BlockId> {
        self.timeline.clone()
    }

    pub fn len(&self) -> usize {
        self.timeline.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timeline.is_empty()
    }

    pub fn block_id_at(&self, index: usize) -> Option<BlockId> {
        self.timeline.get(index).copied()
    }

    #[allow(dead_code)]
    pub fn next_block(&self, id: BlockId) -> Option<BlockId> {
        let index = self
            .timeline
            .iter()
            .position(|candidate| *candidate == id)?;
        self.timeline.get(index + 1).copied()
    }

    pub fn position_of(&self, id: BlockId) -> Option<usize> {
        self.timeline.iter().position(|candidate| *candidate == id)
    }

    #[allow(dead_code)]
    pub fn prev_block(&self, id: BlockId) -> Option<BlockId> {
        let index = self
            .timeline
            .iter()
            .position(|candidate| *candidate == id)?;
        index
            .checked_sub(1)
            .and_then(|previous| self.timeline.get(previous).copied())
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::app::BlockKind;

    use super::BlockStore;

    #[test]
    fn keeps_only_latest_blocks() {
        let mut store = BlockStore::new(PathBuf::from("/tmp"), Some(10), 1024);

        for index in 1..=12 {
            store.start_command(format!("echo {index}"), index, BlockKind::NormalCommand);
            store.finish_command(0, index + 1);
        }

        let blocks = store.blocks_oldest_first();

        assert_eq!(blocks.len(), 10);
        assert_eq!(blocks.first().unwrap().command, "echo 3");
        assert_eq!(blocks.last().unwrap().command, "echo 12");
    }

    #[test]
    fn navigates_prev_and_next_blocks() {
        let mut store = BlockStore::new(PathBuf::from("/tmp"), Some(10), 1024);
        let first = store.start_command("one".to_string(), 0, BlockKind::NormalCommand);
        store.finish_command(0, 1);
        let second = store.start_command("two".to_string(), 1, BlockKind::NormalCommand);
        store.finish_command(0, 2);

        assert_eq!(store.next_block(first), Some(second));
        assert_eq!(store.prev_block(second), Some(first));
        assert_eq!(store.prev_block(first), None);
        assert_eq!(store.next_block(second), None);
    }

    #[test]
    fn can_keep_unbounded_history() {
        let mut store = BlockStore::new(PathBuf::from("/tmp"), None, 1024);

        for index in 1..=12 {
            store.start_command(format!("echo {index}"), index, BlockKind::NormalCommand);
            store.finish_command(0, index + 1);
        }

        assert_eq!(store.blocks_oldest_first().len(), 12);
    }

    #[test]
    fn block_marks_truncated_when_limit_reached() {
        let mut store = BlockStore::new(PathBuf::from("/tmp"), None, 10);
        let id = store.start_command("echo hi".to_string(), 0, BlockKind::NormalCommand);

        store.append_output(b"1234567890");
        assert!(!store.block(id).unwrap().output_truncated);

        store.append_output(b"extra");
        assert!(store.block(id).unwrap().output_truncated);

        store.finish_command(0, 0);
        assert!(store.block(id).unwrap().output_truncated);
    }
}
