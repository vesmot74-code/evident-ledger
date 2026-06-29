use sha2::{Sha256, Digest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    pub root: String,
    pub leaves: Vec<String>,
    pub leaf_count: usize,
}

impl MerkleTree {
    pub fn new(leaves: Vec<String>) -> Self {
        let leaf_count = leaves.len();
        let root = Self::build_merkle_root(&leaves);
        Self { root, leaves, leaf_count }
    }

    /// Детерминированный лист: зависит только от структуры цепочки
    /// sequence + parent_event_id + file_hash
    pub fn build_leaf(sequence: i64, parent_event_id: &Uuid, file_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(sequence.to_be_bytes());
        hasher.update(parent_event_id.as_bytes());
        hasher.update(file_hash.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn build_merkle_root(leaves: &[String]) -> String {
        if leaves.is_empty() {
            return "empty".to_string();
        }
        if leaves.len() == 1 {
            return leaves[0].clone();
        }

        let mut hashed: Vec<String> = leaves.iter()
            .map(|leaf| {
                let mut hasher = Sha256::new();
                hasher.update(leaf.as_bytes());
                format!("{:x}", hasher.finalize())
            })
            .collect();

        while hashed.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in hashed.chunks(2) {
                let left = &chunk[0];
                let right = if chunk.len() > 1 { &chunk[1] } else { left };
                let mut hasher = Sha256::new();
                hasher.update(left.as_bytes());
                hasher.update(right.as_bytes());
                let hash = format!("{:x}", hasher.finalize());
                next_level.push(hash);
            }
            hashed = next_level;
        }

        hashed[0].clone()
    }

    pub fn recompute_root_from_events(events: &[crate::db::EventRow]) -> String {
        let leaves: Vec<String> = events.iter()
            .map(|e| Self::build_leaf(e.sequence, &e.parent_event_id, &e.file_hash))
            .collect();
        Self::build_merkle_root(&leaves)
    }
}
