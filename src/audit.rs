use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub chain_id: Uuid,
    pub file_hash: String,
    pub parent_event_id: Option<Uuid>,
    pub sequence: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub kind: AuditEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditEventKind {
    Created,
    Submitted { idempotency_key: String },
    Anchored {
        server_event_id: Uuid,
        proof: Option<ChainAnchorProof>,
    },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventState {
    Created,
    Submitted,
    Anchored,
    Failed,
}

/// Internal proof that a chain's Merkle root was anchored and signed
/// by this ledger at a given point. This is NOT a TSA (external time
/// authority) attestation — see `crate::tsa::TsaAttestation` for that.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainAnchorProof {
    pub root: String,
    pub signature: String,
    pub anchored_by: String,
}

impl ChainAnchorProof {
    pub fn new(root: String, signature: String, anchored_by: String) -> Self {
        Self { root, signature, anchored_by }
    }
}

impl AuditEvent {
    pub fn created(event_id: Uuid, chain_id: Uuid, file_hash: String, parent_event_id: Option<Uuid>) -> Self {
        Self {
            event_id,
            chain_id,
            file_hash,
            parent_event_id,
            sequence: None,
            created_at: Utc::now(),
            kind: AuditEventKind::Created,
        }
    }

    pub fn submitted(
        event_id: Uuid,
        chain_id: Uuid,
        file_hash: String,
        parent_event_id: Option<Uuid>,
        idempotency_key: String,
    ) -> Self {
        Self {
            event_id,
            chain_id,
            file_hash,
            parent_event_id,
            sequence: None,
            created_at: Utc::now(),
            kind: AuditEventKind::Submitted { idempotency_key },
        }
    }

    pub fn anchored(
        event_id: Uuid,
        chain_id: Uuid,
        file_hash: String,
        parent_event_id: Option<Uuid>,
        sequence: i64,
        server_event_id: Uuid,
        proof: Option<ChainAnchorProof>,
    ) -> Self {
        Self {
            event_id,
            chain_id,
            file_hash,
            parent_event_id,
            sequence: Some(sequence),
            created_at: Utc::now(),
            kind: AuditEventKind::Anchored { server_event_id, proof },
        }
    }

    pub fn failed(event_id: Uuid, chain_id: Uuid, file_hash: String, parent_event_id: Option<Uuid>, error: String) -> Self {
        Self {
            event_id,
            chain_id,
            file_hash,
            parent_event_id,
            sequence: None,
            created_at: Utc::now(),
            kind: AuditEventKind::Failed { error },
        }
    }

    pub fn state(&self) -> EventState {
        match &self.kind {
            AuditEventKind::Created => EventState::Created,
            AuditEventKind::Submitted { .. } => EventState::Submitted,
            AuditEventKind::Anchored { .. } => EventState::Anchored,
            AuditEventKind::Failed { .. } => EventState::Failed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditStore {
    path: PathBuf,
}

impl AuditStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn append(&self, event: &AuditEvent) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        let line = serde_json::to_string(event).unwrap();
        writeln!(file, "{line}")?;
        file.sync_all()?;
        Ok(())
    }

    pub fn read_all(&self) -> io::Result<Vec<AuditEvent>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);
        reader
            .lines()
            .map(|line| serde_json::from_str(&line.unwrap_or_default()).map_err(io::Error::other))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_events_is_append_only_and_readable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let store = AuditStore::new(path.clone());

        let created = AuditEvent::created(Uuid::new_v4(), Uuid::new_v4(), "hash".into(), None);
        store.append(&created).unwrap();

        let submitted = AuditEvent::submitted(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "hash".into(),
            None,
            "idempotency".into(),
        );
        store.append(&submitted).unwrap();

        let events = store.read_all().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].state(), EventState::Created);
        assert_eq!(events[1].state(), EventState::Submitted);
    }

    #[test]
    fn anchored_event_reports_the_right_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let store = AuditStore::new(path);

        let anchored = AuditEvent::anchored(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "hash".into(),
            None,
            1, // sequence number
            Uuid::new_v4(),
            Some(ChainAnchorProof::new("root".into(), "sig".into(), "evident-ledger".into())),
        );

        store.append(&anchored).unwrap();
        let events = store.read_all().unwrap();
        assert_eq!(events[0].state(), EventState::Anchored);
    }
}
