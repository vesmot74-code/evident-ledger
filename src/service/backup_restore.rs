use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::backup_snapshot::{parse_snapshot, validate_structural_integrity, ChainHeadSummary};
use crate::service::verification::STRUCTURAL_INTEGRITY_ERROR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChainState {
    pub event_count: usize,
    pub head_event_id: Uuid,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSummary {
    pub chain_id: Uuid,
    pub backup_id: Uuid,
    pub event_count: usize,
    pub output_path: PathBuf,
}

#[derive(Debug)]
pub enum RestoreError {
    Parse(String),
    Structural(String),
    Conflict { message: String },
    Declined,
    Io(io::Error),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::Parse(msg) => write!(f, "{msg}"),
            RestoreError::Structural(msg) => write!(f, "{msg}"),
            RestoreError::Conflict { message } => write!(f, "{message}"),
            RestoreError::Declined => write!(f, "restore cancelled"),
            RestoreError::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl From<io::Error> for RestoreError {
    fn from(err: io::Error) -> Self {
        RestoreError::Io(err)
    }
}

pub fn backups_dir(evident_dir: &Path) -> PathBuf {
    evident_dir.join("backups")
}

pub fn scan_local_chain_state(evident_dir: &Path, chain_id: Uuid) -> Option<LocalChainState> {
    let mut best: Option<LocalChainState> = None;

    let backups = backups_dir(evident_dir);
    if backups.exists() {
        if let Ok(entries) = fs::read_dir(&backups) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(bytes) = fs::read(&path) else {
                    continue;
                };
                let Ok(snapshot) = parse_snapshot(&bytes) else {
                    continue;
                };
                if snapshot.chain_id != chain_id {
                    continue;
                }
                let Some(summary) = snapshot.head_summary() else {
                    continue;
                };
                best = Some(LocalChainState {
                    event_count: summary.event_count,
                    head_event_id: summary.head_event_id,
                    source: format!("backups/{}", path.file_name()?.to_string_lossy()),
                });
            }
        }
    }

    let proofs_dir = evident_dir.join("proofs").join(chain_id.to_string());
    if proofs_dir.exists() {
        if let Ok(entries) = fs::read_dir(&proofs_dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(content) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
                    continue;
                };
                let Some(events) = json["events"].as_array() else {
                    continue;
                };
                let head = json["head_event_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .or_else(|| {
                        events
                            .last()
                            .and_then(|e| e["event_id"].as_str())
                            .and_then(|s| Uuid::parse_str(s).ok())
                    })?;
                best = Some(LocalChainState {
                    event_count: events.len(),
                    head_event_id: head,
                    source: format!("proofs/{chain_id}/{}", path.file_name()?.to_string_lossy()),
                });
            }
        }
    }

    best
}

pub fn format_conflict_message(
    chain_id: Uuid,
    local: &LocalChainState,
    backup: &ChainHeadSummary,
) -> String {
    format!(
        "Local data already exists for chain {chain_id} (via {}):\n  \
         local:  {} events, head {}\n  \
         backup: {} events, head {}\n\
         Re-run with --force to overwrite the restored backup file.",
        local.source,
        local.event_count,
        local.head_event_id,
        backup.event_count,
        backup.head_event_id
    )
}

pub fn restore_snapshot_bytes<F>(
    evident_dir: &Path,
    backup_id: Uuid,
    snapshot_bytes: &[u8],
    force: bool,
    confirm: F,
) -> Result<RestoreSummary, RestoreError>
where
    F: FnOnce(&str) -> bool,
{
    let snapshot = parse_snapshot(snapshot_bytes).map_err(RestoreError::Parse)?;
    validate_structural_integrity(&snapshot).map_err(RestoreError::Structural)?;

    let backup_summary = snapshot
        .head_summary()
        .ok_or_else(|| RestoreError::Structural(STRUCTURAL_INTEGRITY_ERROR.to_string()))?;

    if let Some(local) = scan_local_chain_state(evident_dir, snapshot.chain_id) {
        if force {
            // --force skips confirmation
        } else {
            let prompt = format_conflict_message(snapshot.chain_id, &local, &backup_summary);
            if !confirm(&prompt) {
                return Err(RestoreError::Declined);
            }
        }
    }

    let out_dir = backups_dir(evident_dir);
    fs::create_dir_all(&out_dir)?;
    let output_path = out_dir.join(format!("{backup_id}.json"));
    fs::write(&output_path, snapshot_bytes)?;

    Ok(RestoreSummary {
        chain_id: snapshot.chain_id,
        backup_id,
        event_count: backup_summary.event_count,
        output_path,
    })
}

pub fn print_restore_summary(summary: &RestoreSummary) {
    println!(
        "Restored: chain {}, {} events.",
        summary.chain_id, summary.event_count
    );
    println!("Saved to: {}", summary.output_path.display());
    println!("This restore validates structural consistency only:");
    println!("event ordering, sequence continuity, and parent linkage.");
    println!("It does not verify cryptographic authenticity or confirm");
    println!("that event contents match the authoritative ledger state.");
    println!("Before relying on restored data, run:");
    println!("  evident verify --chain {}", summary.chain_id);
}

pub fn prompt_confirm(message: &str) -> io::Result<bool> {
    print!("{message}\nProceed? [y/N] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::backup_snapshot::{BackupSnapshot, EventSnapshot};
    use chrono::Utc;
    use tempfile::TempDir;

    fn sample_bytes() -> (Uuid, Uuid, Vec<u8>) {
        let backup_id = Uuid::new_v4();
        let chain_id = Uuid::new_v4();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let now = Utc::now();
        let snapshot = BackupSnapshot {
            chain_id,
            exported_at: now,
            events: vec![
                EventSnapshot {
                    event_id: e1,
                    chain_id,
                    parent_event_id: Uuid::nil(),
                    file_hash: "cc".repeat(32),
                    idempotency_key: "k1".into(),
                    signature: String::new(),
                    created_at: now,
                    sequence: 1,
                },
                EventSnapshot {
                    event_id: e2,
                    chain_id,
                    parent_event_id: e1,
                    file_hash: "dd".repeat(32),
                    idempotency_key: "k2".into(),
                    signature: String::new(),
                    created_at: now,
                    sequence: 2,
                },
            ],
        };
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        (backup_id, chain_id, bytes)
    }

    #[test]
    fn restore_to_empty_evident_dir_succeeds() {
        let tmp = TempDir::new().unwrap();
        let evident_dir = tmp.path();
        let (backup_id, chain_id, bytes) = sample_bytes();

        let summary = restore_snapshot_bytes(evident_dir, backup_id, &bytes, false, |_| false)
            .expect("restore should succeed");

        assert_eq!(summary.chain_id, chain_id);
        assert_eq!(summary.event_count, 2);
        assert!(summary.output_path.exists());
    }

    #[test]
    fn corrupt_snapshot_refuses_write() {
        let tmp = TempDir::new().unwrap();
        let evident_dir = tmp.path();
        let (backup_id, _chain_id, mut bytes) = sample_bytes();
        let snapshot: BackupSnapshot = serde_json::from_slice(&bytes).unwrap();
        let mut broken = snapshot.clone();
        broken.events[1].sequence = 5;
        bytes = serde_json::to_vec(&broken).unwrap();

        let err =
            restore_snapshot_bytes(evident_dir, backup_id, &bytes, false, |_| true).unwrap_err();
        assert!(matches!(err, RestoreError::Structural(_)));
        assert!(!backups_dir(evident_dir)
            .join(format!("{backup_id}.json"))
            .exists());
    }

    #[test]
    fn existing_local_data_requires_confirmation_without_force() {
        let tmp = TempDir::new().unwrap();
        let evident_dir = tmp.path();
        let (backup_id, chain_id, bytes) = sample_bytes();

        let proofs_dir = evident_dir.join("proofs").join(chain_id.to_string());
        fs::create_dir_all(&proofs_dir).unwrap();
        let other_head = Uuid::new_v4();
        let proof_json = serde_json::json!({
            "chain_id": chain_id.to_string(),
            "head_event_id": other_head.to_string(),
            "events": [{"event_id": other_head.to_string(), "sequence": 1}],
            "proof": {}
        });
        fs::write(
            proofs_dir.join("old.json"),
            serde_json::to_string(&proof_json).unwrap(),
        )
        .unwrap();

        let err =
            restore_snapshot_bytes(evident_dir, backup_id, &bytes, false, |_| false).unwrap_err();
        assert!(matches!(err, RestoreError::Declined));
        assert!(!backups_dir(evident_dir)
            .join(format!("{backup_id}.json"))
            .exists());
    }

    #[test]
    fn existing_local_data_succeeds_when_confirmed() {
        let tmp = TempDir::new().unwrap();
        let evident_dir = tmp.path();
        let (backup_id, chain_id, bytes) = sample_bytes();

        let proofs_dir = evident_dir.join("proofs").join(chain_id.to_string());
        fs::create_dir_all(&proofs_dir).unwrap();
        let other_head = Uuid::new_v4();
        let proof_json = serde_json::json!({
            "chain_id": chain_id.to_string(),
            "head_event_id": other_head.to_string(),
            "events": [{"event_id": other_head.to_string(), "sequence": 1}],
            "proof": {}
        });
        fs::write(
            proofs_dir.join("old.json"),
            serde_json::to_string(&proof_json).unwrap(),
        )
        .unwrap();

        let summary = restore_snapshot_bytes(evident_dir, backup_id, &bytes, false, |_| true)
            .expect("confirmed restore");
        assert!(summary.output_path.exists());
    }

    #[test]
    fn force_skips_confirmation_when_local_differs() {
        let tmp = TempDir::new().unwrap();
        let evident_dir = tmp.path();
        let (backup_id, chain_id, bytes) = sample_bytes();

        let proofs_dir = evident_dir.join("proofs").join(chain_id.to_string());
        fs::create_dir_all(&proofs_dir).unwrap();
        let other_head = Uuid::new_v4();
        let proof_json = serde_json::json!({
            "chain_id": chain_id.to_string(),
            "head_event_id": other_head.to_string(),
            "events": [{"event_id": other_head.to_string(), "sequence": 1}],
            "proof": {}
        });
        fs::write(
            proofs_dir.join("old.json"),
            serde_json::to_string(&proof_json).unwrap(),
        )
        .unwrap();

        let summary = restore_snapshot_bytes(evident_dir, backup_id, &bytes, true, |_| false)
            .expect("force restore");
        assert!(summary.output_path.exists());
    }
}
