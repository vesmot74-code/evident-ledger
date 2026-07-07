use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub event_id: String,
    pub chain_id: String,
    pub sequence: u64,
    pub timestamp: u64,
    pub payload_hash: String,
    pub parent_hash: String,
    pub event_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Proof {
    pub chain_id: String,
    pub root_hash: String,
    pub tsa_timestamp: String,
    pub tsa_signature: String,
    pub event_count: u64,
    pub verification_status: bool,
}

impl Event {
    pub fn from_payload(chain_id: &str, sequence: u64, payload_hash: &str, parent_hash: &str, event_type: &str) -> Self {
        let event_id = Uuid::new_v4().to_string();
        Self {
            event_id,
            chain_id: chain_id.to_string(),
            sequence,
            timestamp: Utc::now().timestamp() as u64,
            payload_hash: payload_hash.to_string(),
            parent_hash: parent_hash.to_string(),
            event_type: event_type.to_string(),
        }
    }

    pub fn canonical_hash(&self) -> String {
        let mut without_id = self.clone();
        without_id.event_id = String::new();
        let json = serde_json::to_string(&without_id).expect("event serialization must be stable");
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("canonical serialization must be stable")
}

pub fn append_event_log(path: &Path, event: &Event) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(event).expect("event serialization must be stable"))?;
    file.sync_all()?;
    Ok(())
}

pub fn load_latest_proof(dir: &Path, chain_id: &str) -> Option<Proof> {
    let path = dir.join(chain_id).join("proof.json");
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn write_proof(path: &Path, proof: &Proof) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(proof).expect("proof serialization must be stable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_is_stable_for_same_event() {
        let left = Event::from_payload("chain", 1, "payload", "parent", "commit");
        let mut right = left.clone();
        right.event_id = "different".to_string();
        assert_eq!(left.canonical_hash(), right.canonical_hash());
    }

    #[test]
    fn proof_serialization_is_stable() {
        let proof = Proof {
            chain_id: "chain".into(),
            root_hash: "root".into(),
            tsa_timestamp: "123".into(),
            tsa_signature: "sig".into(),
            event_count: 1,
            verification_status: true,
        };
        let encoded = serde_json::to_string(&proof).unwrap();
        let decoded: Proof = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.verification_status, true);
    }
}
