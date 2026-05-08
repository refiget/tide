use crate::app::{BlockId, CommandBlock};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct BlockIndex {
    pub failed: Vec<BlockId>,
}

impl BlockIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_block_failed(&mut self, id: BlockId) {
        self.failed.push(id);
    }

    pub fn on_block_evicted(&mut self, _id: BlockId) {}

    pub fn query_failed(&self, executions: &HashMap<BlockId, CommandBlock>) -> Vec<BlockId> {
        self.failed
            .iter()
            .copied()
            .filter(|id| executions.contains_key(id))
            .collect()
    }
}
