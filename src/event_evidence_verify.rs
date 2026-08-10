//! CLI-oriented event evidence verification (Stage 2.3).
//!
//! Loads on-disk [`EvidenceRecord`] + [`ProofFile`], runs Stage 1
//! [`verify_evidence_integrity`], optionally refreshes lifecycle, and formats
//! a line-oriented report. No new crypto.

use crate::client::ProofFile;
use crate::evidence_record::{
    evidence_id_for_event, evidence_path, refresh_lifecycle, verify_evidence_integrity,
    write_evidence_record, EvidenceIntegrityResult, EvidenceRecord, LifecycleStatus, TsaStatus,
};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug)]
pub enum EventEvidenceVerifyError {
    EvidenceNotFound { path: PathBuf },
    ProofNotFound { path: PathBuf },
    Corrupt { path: PathBuf, message: String },
    /// Integrity checks failed after a successful load (CLI exit 2).
    IntegrityFailed { report: EventEvidenceReport },
}

impl std::fmt::Display for EventEvidenceVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EvidenceNotFound { path } => write!(
                f,
                "Evidence record not found\nExpected path:\n{}",
                path.display()
            ),
            Self::ProofNotFound { path } => write!(
                f,
                "Proof file not found\nExpected path:\n{}",
                path.display()
            ),
            Self::Corrupt { path, message } => {
                write!(f, "Error: invalid JSON at {} ({message})", path.display())
            }
            Self::IntegrityFailed { .. } => {
                write!(f, "cryptographic verification failed")
            }
        }
    }
}

impl std::error::Error for EventEvidenceVerifyError {}

#[derive(Debug, Clone)]
pub struct EventEvidenceReport {
    pub integrity: EvidenceIntegrityResult,
    pub lifecycle: LifecycleStatus,
    pub tsa_status: TsaStatus,
    pub registered_root: String,
    pub recomputed_root: Option<String>,
}

impl EventEvidenceReport {
    pub fn format_report(&self) -> String {
        let mut out = String::new();
        out.push_str("Evidence Verification\n");
        out.push_str(&format!(
            "Event Found:          {}\n",
            pass_fail(self.integrity.event_found)
        ));
        out.push_str(&format!(
            "Parent Chain Valid:   {}\n",
            pass_fail(self.integrity.parent_chain_valid)
        ));
        out.push_str(&format!(
            "Merkle Root Valid:    {}\n",
            pass_fail(self.integrity.merkle_root_valid)
        ));
        out.push_str(&format!(
            "Signature Valid:      {}\n",
            pass_fail(self.integrity.signature_valid)
        ));
        out.push_str("Merkle Root:\n");
        out.push_str(&format!("registered: {}\n", self.registered_root));
        match &self.recomputed_root {
            Some(r) => out.push_str(&format!("recomputed: {r}\n")),
            None => out.push_str("recomputed: not recomputed\n"),
        }
        if !self.integrity.errors.is_empty() {
            out.push_str("Verification Errors:\n");
            for err in &self.integrity.errors {
                out.push_str(&format!("- {err}\n"));
            }
        }
        out.push_str(&format!(
            "TSA Status:           {}\n",
            tsa_label(self.tsa_status)
        ));
        let life = lifecycle_label(self.lifecycle);
        out.push_str(&format!("Lifecycle:            {life}\n"));
        out.push_str(&format!("Overall:              {life}\n"));
        out
    }
}

fn pass_fail(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

fn lifecycle_label(status: LifecycleStatus) -> &'static str {
    match status {
        LifecycleStatus::Created => "CREATED",
        LifecycleStatus::Registered => "REGISTERED",
        LifecycleStatus::TsaConfirmed => "TSA_CONFIRMED",
        LifecycleStatus::Certified => "CERTIFIED",
        LifecycleStatus::Revoked => "REVOKED",
    }
}

fn tsa_label(status: TsaStatus) -> &'static str {
    match status {
        TsaStatus::Pending => "PENDING",
        TsaStatus::Confirmed => "CONFIRMED",
        TsaStatus::Failed => "FAILED",
        TsaStatus::Absent => "ABSENT",
    }
}

pub fn evidence_file_path(evidence_dir: &Path, event_id: Uuid) -> PathBuf {
    evidence_path(evidence_dir, &evidence_id_for_event(event_id))
}

pub fn proof_file_path(proofs_root: &Path, chain_id: Uuid, event_id: Uuid) -> PathBuf {
    proofs_root
        .join(chain_id.to_string())
        .join(format!("{event_id}.json"))
}

fn load_evidence(path: &Path) -> Result<EvidenceRecord, EventEvidenceVerifyError> {
    if !path.is_file() {
        return Err(EventEvidenceVerifyError::EvidenceNotFound {
            path: path.to_path_buf(),
        });
    }
    let raw = fs::read_to_string(path).map_err(|e| EventEvidenceVerifyError::Corrupt {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    serde_json::from_str(&raw).map_err(|e| EventEvidenceVerifyError::Corrupt {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

fn load_proof(path: &Path) -> Result<ProofFile, EventEvidenceVerifyError> {
    if !path.is_file() {
        return Err(EventEvidenceVerifyError::ProofNotFound {
            path: path.to_path_buf(),
        });
    }
    let raw = fs::read_to_string(path).map_err(|e| EventEvidenceVerifyError::Corrupt {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    serde_json::from_str(&raw).map_err(|e| EventEvidenceVerifyError::Corrupt {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Verify event evidence under the given directories (testable; no hardcoded HOME).
///
/// On integrity success: `refresh_lifecycle` + `write_evidence_record`.
/// On integrity failure: returns [`EventEvidenceVerifyError::IntegrityFailed`]
/// with a fully populated report (no lifecycle write).
pub fn verify_event_evidence(
    evidence_dir: &Path,
    proofs_root: &Path,
    event_id: Uuid,
    chain_id: Uuid,
) -> Result<EventEvidenceReport, EventEvidenceVerifyError> {
    let evidence_path = evidence_file_path(evidence_dir, event_id);
    let proof_path = proof_file_path(proofs_root, chain_id, event_id);

    let mut record = load_evidence(&evidence_path)?;
    let proof = load_proof(&proof_path)?;

    let integrity = verify_evidence_integrity(&record, &proof);
    let registered_root = proof.proof.root.clone();
    let recomputed_root = integrity.recomputed_root.clone();

    if integrity.is_valid() {
        refresh_lifecycle(&mut record, &proof);
        write_evidence_record(evidence_dir, &record).map_err(|e| {
            EventEvidenceVerifyError::Corrupt {
                path: evidence_path,
                message: e.to_string(),
            }
        })?;
        Ok(EventEvidenceReport {
            integrity,
            lifecycle: record.lifecycle_status,
            tsa_status: record.tsa_status,
            registered_root,
            recomputed_root,
        })
    } else {
        let report = EventEvidenceReport {
            integrity,
            lifecycle: record.lifecycle_status,
            tsa_status: record.tsa_status,
            registered_root,
            recomputed_root,
        };
        Err(EventEvidenceVerifyError::IntegrityFailed { report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_record::LifecycleStatus;
    use std::path::PathBuf;
    use tempfile::tempdir;

    const EVENT_ID: &str = "22d29a6a-4cb4-469f-bbce-1d07e49694ce";
    const CHAIN_ID: &str = "c0bafd33-6807-4fb7-b480-c454ecabdd5d";

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stage2_3")
    }

    fn stage_dirs() -> (tempfile::TempDir, Uuid, Uuid) {
        let tmp = tempdir().unwrap();
        let evidence_dir = tmp.path().join("evidence");
        let proofs_root = tmp.path().join("proofs");
        let event_id = Uuid::parse_str(EVENT_ID).unwrap();
        let chain_id = Uuid::parse_str(CHAIN_ID).unwrap();
        fs::create_dir_all(&evidence_dir).unwrap();
        fs::create_dir_all(proofs_root.join(chain_id.to_string())).unwrap();
        fs::copy(
            fixture_dir().join("evidence.json"),
            evidence_file_path(&evidence_dir, event_id),
        )
        .unwrap();
        fs::copy(
            fixture_dir().join("proof.json"),
            proof_file_path(&proofs_root, chain_id, event_id),
        )
        .unwrap();
        // Keep tmp alive by leaking paths into verify via returning tmp
        (tmp, event_id, chain_id)
    }

    #[test]
    fn valid_fixture_event_passes_and_overall_matches_lifecycle() {
        let (tmp, event_id, chain_id) = stage_dirs();
        let evidence_dir = tmp.path().join("evidence");
        let proofs_root = tmp.path().join("proofs");

        let report = verify_event_evidence(&evidence_dir, &proofs_root, event_id, chain_id)
            .expect("fixture must verify");
        assert!(report.integrity.event_found);
        assert!(report.integrity.parent_chain_valid);
        assert!(report.integrity.merkle_root_valid);
        assert!(report.integrity.signature_valid);

        let text = report.format_report();
        assert!(text.contains("Event Found:          PASS"));
        assert!(text.contains("Parent Chain Valid:   PASS"));
        assert!(text.contains("Merkle Root Valid:    PASS"));
        assert!(text.contains("Signature Valid:      PASS"));

        let life = lifecycle_label(report.lifecycle);
        assert!(
            text.contains(&format!("Lifecycle:            {life}")),
            "lifecycle line missing: {text}"
        );
        assert!(
            text.contains(&format!("Overall:              {life}")),
            "Overall must mirror lifecycle_status, got:\n{text}"
        );

        // Persisted record matches reported lifecycle.
        let reloaded = crate::evidence_record::read_evidence_record(
            &evidence_dir,
            &evidence_id_for_event(event_id),
        )
        .unwrap();
        assert_eq!(reloaded.lifecycle_status, report.lifecycle);
    }

    #[test]
    fn missing_evidence_record_is_clear_error() {
        let (tmp, event_id, chain_id) = stage_dirs();
        let evidence_dir = tmp.path().join("evidence");
        let proofs_root = tmp.path().join("proofs");
        let path = evidence_file_path(&evidence_dir, event_id);
        fs::remove_file(&path).unwrap();

        let err = verify_event_evidence(&evidence_dir, &proofs_root, event_id, chain_id)
            .expect_err("must fail");
        match err {
            EventEvidenceVerifyError::EvidenceNotFound { path: ref p } => {
                assert_eq!(p, &path);
                let msg = err.to_string();
                assert!(msg.contains("Evidence record not found"));
                assert!(msg.contains("Expected path:"));
                assert!(msg.contains(&path.display().to_string()));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn broken_signature_fails_with_exit_style_error() {
        let (tmp, event_id, chain_id) = stage_dirs();
        let evidence_dir = tmp.path().join("evidence");
        let proofs_root = tmp.path().join("proofs");
        let proof_path = proof_file_path(&proofs_root, chain_id, event_id);

        let mut proof: ProofFile =
            serde_json::from_str(&fs::read_to_string(&proof_path).unwrap()).unwrap();
        // Flip one hex nibble so Ed25519 verify fails.
        let mut chars: Vec<char> = proof.proof.signature.chars().collect();
        chars[0] = if chars[0] == '0' { '1' } else { '0' };
        proof.proof.signature = chars.into_iter().collect();
        fs::write(&proof_path, serde_json::to_string_pretty(&proof).unwrap()).unwrap();

        let err = verify_event_evidence(&evidence_dir, &proofs_root, event_id, chain_id)
            .expect_err("tampered sig must fail");
        match err {
            EventEvidenceVerifyError::IntegrityFailed { report } => {
                assert!(!report.integrity.signature_valid);
                let text = report.format_report();
                assert!(text.contains("Signature Valid:      FAIL"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_verify_certified_is_idempotent() {
        let (tmp, event_id, chain_id) = stage_dirs();
        let evidence_dir = tmp.path().join("evidence");
        let proofs_root = tmp.path().join("proofs");

        let first = verify_event_evidence(&evidence_dir, &proofs_root, event_id, chain_id)
            .expect("first verify");
        assert!(first.integrity.is_valid());
        assert_eq!(first.lifecycle, LifecycleStatus::Certified);

        let before = crate::evidence_record::read_evidence_record(
            &evidence_dir,
            &evidence_id_for_event(event_id),
        )
        .unwrap();
        assert_eq!(before.lifecycle_status, LifecycleStatus::Certified);
        let before_json = serde_json::to_value(&before).unwrap();

        let second = verify_event_evidence(&evidence_dir, &proofs_root, event_id, chain_id)
            .expect("second verify");
        assert!(second.integrity.is_valid());
        assert_eq!(second.lifecycle, LifecycleStatus::Certified);
        assert_eq!(second.lifecycle, first.lifecycle);

        let after = crate::evidence_record::read_evidence_record(
            &evidence_dir,
            &evidence_id_for_event(event_id),
        )
        .unwrap();
        assert_eq!(after.lifecycle_status, before.lifecycle_status);
        assert_eq!(after.lifecycle_status, LifecycleStatus::Certified);
        let after_json = serde_json::to_value(&after).unwrap();
        assert_eq!(
            before_json, after_json,
            "idempotent verify must not alter EvidenceRecord fields"
        );
    }
}
