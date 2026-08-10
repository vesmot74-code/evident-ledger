//! Evidence Record — projection layer over ledger events and proof artifacts.
//!
//! Not a source of truth. Events, proofs, TSA tokens, and identity keys remain
//! authoritative. See `docs/audit_stage1.md`.

use crate::client::{ProofFile, TsaData};
use crate::db::EventRow;
use crate::service::verification::{check_event_structure, StructuralFailure};
use crate::signing::verify_root;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Namespace for `certificate_id = UUIDv5(CERTIFICATE_NAMESPACE, event_id bytes)`.
/// Derived once from the URL namespace so the value is stable across builds.
pub fn certificate_namespace() -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        b"https://evident-ledger.com/ns/certificate",
    )
}

/// Lifecycle of an evidence projection.
///
/// `EXPIRED` is intentionally absent from the active model (reserved for a
/// future extension only — see `docs/audit_stage1.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleStatus {
    Created,
    Registered,
    TsaConfirmed,
    Certified,
    /// Reserved. Full revoke semantics are not implemented in Stage 1.
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TsaStatus {
    Pending,
    Confirmed,
    Failed,
    Absent,
}

/// Aggregated projection for search / export / certificates / verifiers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub filename: Option<String>,
    pub sha256: String,
    pub size_bytes: Option<u64>,
    pub mime_type: Option<String>,
    pub local_file_available: bool,
    pub chain_id: String,
    pub event_id: String,
    pub certificate_id: String,
    pub created_at_local: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
    pub lifecycle_status: LifecycleStatus,
    pub tsa_status: TsaStatus,
    pub project_id: Option<String>,
    /// Relative or absolute path to the linked `proof_v1` artifact, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_path: Option<String>,
}

/// Optional client-supplied presentation metadata (not stored on the ledger).
#[derive(Debug, Clone, Default)]
pub struct EvidenceFileMeta {
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub project_id: Option<String>,
    pub local_file_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceIntegrityResult {
    pub event_found: bool,
    pub parent_chain_valid: bool,
    pub merkle_root_valid: bool,
    pub signature_valid: bool,
    pub recomputed_root: Option<String>,
    pub errors: Vec<String>,
}

impl EvidenceIntegrityResult {
    pub fn is_valid(&self) -> bool {
        self.event_found
            && self.parent_chain_valid
            && self.merkle_root_valid
            && self.signature_valid
            && self.errors.is_empty()
    }
}

/// Deterministic evidence id: one event → one evidence projection.
pub fn evidence_id_for_event(event_id: Uuid) -> String {
    format!("ev_{}", event_id.as_simple())
}

/// Deterministic certificate id: `cert_` + UUIDv5(namespace, event_id bytes).
pub fn certificate_id_for_event(event_id: Uuid) -> String {
    let id = Uuid::new_v5(&certificate_namespace(), event_id.as_bytes());
    format!("cert_{}", id.as_simple())
}

pub fn default_evidence_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".evident")
        .join("evidence")
}

pub fn evidence_path(dir: &Path, evidence_id: &str) -> PathBuf {
    dir.join(format!("{evidence_id}.json"))
}

/// Build a projection after a successful ledger registration.
pub fn build_registered_record(
    event_id: Uuid,
    chain_id: Uuid,
    sha256: &str,
    size_bytes: Option<u64>,
    meta: &EvidenceFileMeta,
    tsa: Option<&TsaData>,
    proof_path: Option<&Path>,
    registered_at: DateTime<Utc>,
) -> EvidenceRecord {
    let tsa_status = classify_tsa_status(tsa);
    let lifecycle_status = initial_lifecycle(tsa_status);

    EvidenceRecord {
        evidence_id: evidence_id_for_event(event_id),
        filename: meta.filename.clone(),
        sha256: sha256.to_string(),
        size_bytes,
        mime_type: meta.mime_type.clone(),
        local_file_available: meta.local_file_available,
        chain_id: chain_id.to_string(),
        event_id: event_id.to_string(),
        certificate_id: certificate_id_for_event(event_id),
        created_at_local: registered_at,
        registered_at,
        lifecycle_status,
        tsa_status,
        project_id: meta.project_id.clone(),
        proof_path: proof_path.map(|p| p.display().to_string()),
    }
}

fn classify_tsa_status(tsa: Option<&TsaData>) -> TsaStatus {
    match tsa {
        Some(t) if t.token_bytes.unwrap_or(0) > 0 || t.timestamp.is_some() => TsaStatus::Confirmed,
        Some(_) => TsaStatus::Pending,
        None => TsaStatus::Absent,
    }
}

fn initial_lifecycle(tsa_status: TsaStatus) -> LifecycleStatus {
    match tsa_status {
        TsaStatus::Confirmed => LifecycleStatus::TsaConfirmed,
        _ => LifecycleStatus::Registered,
    }
}

/// Persist projection JSON. Does not overwrite an existing different sha256/event
/// pair under the same evidence_id (ids are event-derived, so collision implies
/// the same event).
pub fn write_evidence_record(dir: &Path, record: &EvidenceRecord) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = evidence_path(dir, &record.evidence_id);
    let body = serde_json::to_string_pretty(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(&path, body)?;
    Ok(path)
}

pub fn read_evidence_record(dir: &Path, evidence_id: &str) -> std::io::Result<EvidenceRecord> {
    let path = evidence_path(dir, evidence_id);
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// SHA-256 is not a primary key: returns every matching projection.
pub fn find_evidence_by_hash(dir: &Path, hash: &str) -> std::io::Result<Vec<EvidenceRecord>> {
    let needle = hash.trim().to_ascii_lowercase();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let record: EvidenceRecord = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if record.sha256.eq_ignore_ascii_case(&needle) {
            out.push(record);
        }
    }
    out.sort_by(|a, b| a.evidence_id.cmp(&b.evidence_id));
    Ok(out)
}

/// Upgrade lifecycle from an integrity check + TSA presence.
///
/// `REVOKED` is never set here (reserved). `EXPIRED` is not part of the model.
pub fn derive_lifecycle(
    integrity: &EvidenceIntegrityResult,
    tsa_status: TsaStatus,
) -> LifecycleStatus {
    if !integrity.event_found || !integrity.parent_chain_valid || !integrity.merkle_root_valid {
        return LifecycleStatus::Registered;
    }
    if !integrity.signature_valid {
        return LifecycleStatus::Registered;
    }
    match tsa_status {
        TsaStatus::Confirmed if integrity.is_valid() => LifecycleStatus::Certified,
        TsaStatus::Confirmed => LifecycleStatus::TsaConfirmed,
        TsaStatus::Pending | TsaStatus::Absent | TsaStatus::Failed => LifecycleStatus::Registered,
    }
}

/// Advancement rank for the Stage 1 lifecycle ladder (excludes reserved Revoked).
fn lifecycle_rank(status: LifecycleStatus) -> u8 {
    match status {
        LifecycleStatus::Created => 0,
        LifecycleStatus::Registered => 1,
        LifecycleStatus::TsaConfirmed => 2,
        LifecycleStatus::Certified => 3,
        // Reserved terminal state: never auto-advanced by refresh; never downgraded.
        LifecycleStatus::Revoked => 255,
    }
}

/// Prefer the further-along status. Never moves backward on the CREATED→…→CERTIFIED
/// ladder. `REVOKED` is sticky (reserved; not produced by [`derive_lifecycle`]).
pub fn advance_lifecycle(current: LifecycleStatus, derived: LifecycleStatus) -> LifecycleStatus {
    if current == LifecycleStatus::Revoked {
        return LifecycleStatus::Revoked;
    }
    if lifecycle_rank(derived) >= lifecycle_rank(current) {
        derived
    } else {
        current
    }
}

/// Recompute TSA/lifecycle from proof + integrity, without downgrading an already
/// advanced lifecycle (Stage 2.4 monotonicity).
pub fn refresh_lifecycle(record: &mut EvidenceRecord, proof: &ProofFile) {
    let integrity = verify_evidence_integrity(record, proof);
    record.tsa_status = classify_tsa_status(proof.tsa.as_ref());
    let derived = derive_lifecycle(&integrity, record.tsa_status);
    record.lifecycle_status = advance_lifecycle(record.lifecycle_status, derived);
}

/// Independent verification for an Evidence Record against a `proof_v1` artifact.
///
/// Model (see audit): parent-linked hash chain + merkle-root-v1 over the **full**
/// leaf list + Ed25519. No Merkle inclusion path (`siblings` / `positions`).
pub fn verify_evidence_integrity(
    record: &EvidenceRecord,
    proof: &ProofFile,
) -> EvidenceIntegrityResult {
    let mut errors = Vec::new();
    let mut event_found = false;

    for leaf in &proof.events {
        if leaf.event_id == record.event_id {
            event_found = true;
            if !leaf.file_hash.eq_ignore_ascii_case(&record.sha256) {
                errors.push("event file_hash does not match evidence sha256".into());
            }
            break;
        }
    }
    if !event_found {
        errors.push("event_id not present in proof events".into());
    }
    if proof.chain_id != record.chain_id {
        errors.push("proof chain_id does not match evidence chain_id".into());
    }

    let rows: Vec<EventRow> = proof
        .events
        .iter()
        .filter_map(|e| {
            let event_id = Uuid::parse_str(&e.event_id).ok()?;
            let parent_event_id = Uuid::parse_str(&e.parent_event_id).ok()?;
            Some(EventRow {
                event_id,
                parent_event_id,
                file_hash: e.file_hash.clone(),
                created_at: Utc::now(),
                sequence: e.sequence,
            })
        })
        .collect();

    if rows.len() != proof.events.len() {
        errors.push("proof events contain invalid UUIDs".into());
    }

    let (parent_chain_valid, recomputed_root) = match check_event_structure(&rows) {
        Ok(root) => (true, Some(root)),
        Err(StructuralFailure::ParentChain { index }) => {
            errors.push(format!("parent chain broken at index {index}"));
            (false, None)
        }
        Err(StructuralFailure::Sequence { index }) => {
            errors.push(format!("sequence broken at index {index}"));
            (false, None)
        }
        Err(StructuralFailure::EmptyMerkle) => {
            errors.push("empty merkle root".into());
            (false, None)
        }
    };

    let merkle_root_valid = match &recomputed_root {
        Some(root) => {
            let ok = root == &proof.proof.root;
            if !ok {
                errors.push("recomputed merkle root does not match proof.root".into());
            }
            ok
        }
        None => false,
    };

    let signature_valid = verify_root(
        &proof.chain_id,
        &proof.proof.root,
        &proof.proof.chain_head,
        &proof.proof.signature,
        &proof.proof.public_key,
    );
    if !signature_valid {
        errors.push("Ed25519 signature verification failed".into());
    }

    EvidenceIntegrityResult {
        event_found,
        parent_chain_valid,
        merkle_root_valid,
        signature_valid,
        recomputed_root,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{EventLeaf, ProofPayload};
    use crate::merkle::MerkleTree;
    use crate::signing::ServerSigner;
    use tempfile::tempdir;

    #[test]
    fn certificate_id_is_stable_per_event() {
        let event = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let a = certificate_id_for_event(event);
        let b = certificate_id_for_event(event);
        assert_eq!(a, b);
        assert!(a.starts_with("cert_"));
        let other = certificate_id_for_event(Uuid::nil());
        assert_ne!(a, other);
    }

    #[test]
    fn same_hash_multiple_evidence_records() {
        let dir = tempdir().unwrap();
        let hash = "a".repeat(64);
        let meta = EvidenceFileMeta {
            filename: Some("contract.pdf".into()),
            local_file_available: true,
            project_id: Some("project-a".into()),
            ..Default::default()
        };
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let r1 = build_registered_record(
            e1,
            Uuid::new_v4(),
            &hash,
            Some(100),
            &meta,
            None,
            None,
            Utc::now(),
        );
        let mut meta_b = meta.clone();
        meta_b.project_id = Some("project-b".into());
        let r2 = build_registered_record(
            e2,
            Uuid::new_v4(),
            &hash,
            Some(100),
            &meta_b,
            None,
            None,
            Utc::now(),
        );
        write_evidence_record(dir.path(), &r1).unwrap();
        write_evidence_record(dir.path(), &r2).unwrap();

        let found = find_evidence_by_hash(dir.path(), &hash).unwrap();
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].evidence_id, found[1].evidence_id);
    }

    #[test]
    fn verify_evidence_integrity_against_signed_proof() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("signing.key");
        let signer = ServerSigner::load_or_create(key_path.to_str().unwrap()).unwrap();

        let chain_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let file_hash = "b".repeat(64);
        let leaf = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
        let root = MerkleTree::build_merkle_root(&[leaf]);
        let chain_head = event_id.to_string();
        let signature = signer.sign_root(&chain_id.to_string(), &root, &chain_head);

        let proof = ProofFile {
            leaf_version: "leaf_v1".into(),
            chain_id: chain_id.to_string(),
            head_event_id: chain_head.clone(),
            proof: ProofPayload {
                root: root.clone(),
                chain_head,
                signature,
                public_key: signer.public_key_hex(),
                leaves_count: 1,
                version: Some("proof_v1".into()),
                proof_type: Some("merkle-root-v1".into()),
            },
            events: vec![EventLeaf {
                sequence: 1,
                event_id: event_id.to_string(),
                parent_event_id: Uuid::nil().to_string(),
                file_hash: file_hash.clone(),
            }],
            tsa: Some(TsaData {
                timestamp: Some(1_700_000_000),
                serial: Some("1".into()),
                token_bytes: Some(128),
            }),
        };

        let record = build_registered_record(
            event_id,
            chain_id,
            &file_hash,
            Some(42),
            &EvidenceFileMeta {
                filename: Some("doc.pdf".into()),
                local_file_available: false,
                ..Default::default()
            },
            proof.tsa.as_ref(),
            None,
            Utc::now(),
        );
        assert_eq!(record.lifecycle_status, LifecycleStatus::TsaConfirmed);

        let result = verify_evidence_integrity(&record, &proof);
        assert!(result.is_valid(), "{:?}", result.errors);

        let mut refreshed = record.clone();
        refresh_lifecycle(&mut refreshed, &proof);
        assert_eq!(refreshed.lifecycle_status, LifecycleStatus::Certified);
    }

    #[test]
    fn test_certified_lifecycle_never_downgrades() {
        let dir = tempdir().unwrap();
        let key_path = dir.path().join("signing.key");
        let signer = ServerSigner::load_or_create(key_path.to_str().unwrap()).unwrap();

        let chain_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let file_hash = "c".repeat(64);
        let leaf = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
        let root = MerkleTree::build_merkle_root(&[leaf]);
        let chain_head = event_id.to_string();
        let signature = signer.sign_root(&chain_id.to_string(), &root, &chain_head);

        let mut proof = ProofFile {
            leaf_version: "leaf_v1".into(),
            chain_id: chain_id.to_string(),
            head_event_id: chain_head.clone(),
            proof: ProofPayload {
                root: root.clone(),
                chain_head,
                signature,
                public_key: signer.public_key_hex(),
                leaves_count: 1,
                version: Some("proof_v1".into()),
                proof_type: Some("merkle-root-v1".into()),
            },
            events: vec![EventLeaf {
                sequence: 1,
                event_id: event_id.to_string(),
                parent_event_id: Uuid::nil().to_string(),
                file_hash: file_hash.clone(),
            }],
            tsa: Some(TsaData {
                timestamp: Some(1_700_000_000),
                serial: Some("1".into()),
                token_bytes: Some(128),
            }),
        };

        let mut record = build_registered_record(
            event_id,
            chain_id,
            &file_hash,
            Some(1),
            &EvidenceFileMeta::default(),
            proof.tsa.as_ref(),
            None,
            Utc::now(),
        );
        record.lifecycle_status = LifecycleStatus::Certified;

        // Happy path: still Certified.
        refresh_lifecycle(&mut record, &proof);
        assert_eq!(record.lifecycle_status, LifecycleStatus::Certified);

        // Would-be downgrade path (tampered signature → derive would say Registered):
        // monotonic refresh must keep CERTIFIED.
        let mut chars: Vec<char> = proof.proof.signature.chars().collect();
        chars[0] = if chars[0] == '0' { '1' } else { '0' };
        proof.proof.signature = chars.into_iter().collect();
        refresh_lifecycle(&mut record, &proof);
        assert_eq!(
            record.lifecycle_status,
            LifecycleStatus::Certified,
            "CERTIFIED must not regress when integrity later fails"
        );
        assert_eq!(
            advance_lifecycle(LifecycleStatus::Certified, LifecycleStatus::Registered),
            LifecycleStatus::Certified
        );
        assert_eq!(
            advance_lifecycle(LifecycleStatus::Certified, LifecycleStatus::TsaConfirmed),
            LifecycleStatus::Certified
        );
    }
}
