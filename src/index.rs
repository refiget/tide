use std::collections::{HashMap, HashSet};

use crate::app::{BlockId, CommandBlock};

#[derive(Debug, Default)]
pub struct BlockIndex {
    pub failed: Vec<BlockId>,
    pub tokens: HashMap<String, Vec<BlockId>>,
}

impl BlockIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_block_failed(&mut self, id: BlockId) {
        self.failed.push(id);
    }

    pub fn on_block_evicted(&mut self, _id: BlockId) {}

    pub fn index_command(&mut self, id: BlockId, command: &str) {
        let stripped = strip_ansi_escapes::strip(command.as_bytes());
        let text = String::from_utf8_lossy(&stripped).to_lowercase();
        for token in tokenize_str(&text) {
            self.tokens.entry(token).or_default().push(id);
        }
    }

    pub fn query_failed(&self, executions: &HashMap<BlockId, CommandBlock>) -> Vec<BlockId> {
        self.failed
            .iter()
            .copied()
            .filter(|id| executions.contains_key(id))
            .collect()
    }

    pub fn query_command(
        &self,
        query: &str,
        executions: &HashMap<BlockId, CommandBlock>,
    ) -> Vec<BlockId> {
        let query_tokens = tokenize_str(&query.to_lowercase());
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let mut per_query: Vec<HashSet<BlockId>> = Vec::new();
        for qt in &query_tokens {
            let mut matched: HashSet<BlockId> = HashSet::new();
            for (vocab_token, posting) in &self.tokens {
                if vocab_token.contains(qt.as_str()) {
                    for &id in posting {
                        if executions.contains_key(&id) {
                            matched.insert(id);
                        }
                    }
                }
            }
            per_query.push(matched);
        }

        let result = per_query
            .into_iter()
            .reduce(|acc, set| acc.intersection(&set).copied().collect())
            .unwrap_or_default();

        let mut sorted: Vec<BlockId> = result.into_iter().collect();
        sorted.sort();
        sorted
    }
}

fn tokenize_str(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_command_substring_and_semantics() {
        let mut index = BlockIndex::default();
        index.index_command(BlockId(1), "cargo build --release");
        index.index_command(BlockId(2), "cargo test");
        index.index_command(BlockId(3), "git commit -m message");
        let executions: HashMap<_, _> = [
            (
                BlockId(1),
                CommandBlock {
                    id: BlockId(1),
                    ..Default::default()
                },
            ),
            (
                BlockId(2),
                CommandBlock {
                    id: BlockId(2),
                    ..Default::default()
                },
            ),
            (
                BlockId(3),
                CommandBlock {
                    id: BlockId(3),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .collect();

        let r = index.query_command("car", &executions);
        assert_eq!(r, vec![BlockId(1), BlockId(2)]);

        let r = index.query_command("cargo build", &executions);
        assert_eq!(r, vec![BlockId(1)]);

        let r = index.query_command("docker", &executions);
        assert!(r.is_empty());

        let r = index.query_command("", &executions);
        assert!(r.is_empty());
    }
}
