use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::EventRow;
use crate::service::verification::{check_event_structure, STRUCTURAL_INTEGRITY_ERROR};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct BackupSnapshot {
    pub chain_id: Uuid,
    pub events: Vec<EventSnapshot>,
    pub exported_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct EventSnapshot {
    pub event_id: Uuid,
    pub chain_id: Uuid,
    pub parent_event_id: Uuid,
    pub file_hash: String,
    pub idempotency_key: String,
    pub signature: String,
    pub created_at: DateTime<Utc>,
    pub sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHeadSummary {
    pub event_count: usize,
    pub head_event_id: Uuid,
}

impl BackupSnapshot {
    pub fn head_summary(&self) -> Option<ChainHeadSummary> {
        let last = self.events.last()?;
        Some(ChainHeadSummary {
            event_count: self.events.len(),
            head_event_id: last.event_id,
        })
    }

    pub fn to_event_rows(&self) -> Vec<EventRow> {
        self.events
            .iter()
            .map(|e| EventRow {
                event_id: e.event_id,
                parent_event_id: e.parent_event_id,
                file_hash: e.file_hash.clone(),
                created_at: e.created_at,
                sequence: e.sequence,
            })
            .collect()
    }
}

pub fn parse_snapshot(bytes: &[u8]) -> Result<BackupSnapshot, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("invalid backup JSON: {e}"))
}

pub fn validate_structural_integrity(snapshot: &BackupSnapshot) -> Result<String, String> {
    for event in &snapshot.events {
        if event.chain_id != snapshot.chain_id {
            return Err(STRUCTURAL_INTEGRITY_ERROR.to_string());
        }
    }

    let rows = snapshot.to_event_rows();
    check_event_structure(&rows).map_err(|_| STRUCTURAL_INTEGRITY_ERROR.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> BackupSnapshot {
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let chain_id = Uuid::new_v4();
        let now = Utc::now();
        BackupSnapshot {
            chain_id,
            exported_at: now,
            events: vec![
                EventSnapshot {
                    event_id: e1,
                    chain_id,
                    parent_event_id: Uuid::nil(),
                    file_hash: "aa".repeat(32),
                    idempotency_key: "k1".into(),
                    signature: String::new(),
                    created_at: now,
                    sequence: 1,
                },
                EventSnapshot {
                    event_id: e2,
                    chain_id,
                    parent_event_id: e1,
                    file_hash: "bb".repeat(32),
                    idempotency_key: "k2".into(),
                    signature: String::new(),
                    created_at: now,
                    sequence: 2,
                },
            ],
        }
    }

    #[test]
    fn valid_snapshot_passes_structural_check() {
        let snapshot = sample_snapshot();
        assert!(validate_structural_integrity(&snapshot).is_ok());
    }

    #[test]
    fn broken_parent_chain_fails() {
        let mut snapshot = sample_snapshot();
        snapshot.events[1].parent_event_id = Uuid::new_v4();
        let err = validate_structural_integrity(&snapshot).unwrap_err();
        assert_eq!(err, STRUCTURAL_INTEGRITY_ERROR);
    }

    #[test]
    fn broken_sequence_fails() {
        let mut snapshot = sample_snapshot();
        snapshot.events[1].sequence = 99;
        let err = validate_structural_integrity(&snapshot).unwrap_err();
        assert_eq!(err, STRUCTURAL_INTEGRITY_ERROR);
    }

    // Structural validation checks chain shape (linkage/sequence), not
    // event content authenticity. Content tampering is only caught by
    // evident verify against the authoritative ledger. This test locks in
    // that boundary so future changes to validate_structural_integrity are
    // deliberate, not accidental.
    #[test]
    fn tampered_file_hash_passes_structural_check() {
        let mut snapshot = sample_snapshot();
        snapshot.events[0].file_hash = "ff".repeat(32);
        assert!(validate_structural_integrity(&snapshot).is_ok());
    }
}
