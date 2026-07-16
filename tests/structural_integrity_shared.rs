//! Shared structural-integrity cases: restore and evident-verify must agree.

use chrono::Utc;
use evident_ledger::db::EventRow;
use evident_ledger::service::backup_restore::{print_restore_summary, restore_snapshot_bytes};
use evident_ledger::service::backup_snapshot::{
    validate_structural_integrity, BackupSnapshot, EventSnapshot,
};
use evident_ledger::service::verification::{
    check_event_structure, StructuralFailure, STRUCTURAL_INTEGRITY_ERROR,
};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use uuid::Uuid;

fn setup_isolated_home() -> PathBuf {
    let home = PathBuf::from(format!("/tmp/evident_structural_home_{}", uuid_simple()));
    let evident_dir = home.join(".evident");
    std::fs::create_dir_all(&evident_dir).expect("create isolated home");

    let fixture_key =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server_identity.pub");
    std::fs::copy(&fixture_key, evident_dir.join("server_identity.pub"))
        .expect("tests/fixtures/server_identity.pub missing");

    home
}

fn run_verifier(proof_path: &str) -> (String, i32) {
    let home = setup_isolated_home();
    let output = Command::new("cargo")
        .args(["run", "--bin", "evident-verify", "--", proof_path])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HOME", &home)
        .output()
        .expect("failed to run verifier");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr);
    let code = output.status.code().unwrap_or(-1);
    (combined, code)
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        .to_string()
}

fn load_proof() -> serde_json::Value {
    let content = std::fs::read_to_string("tests/fixtures/proof.json")
        .expect("tests/fixtures/proof.json missing");
    serde_json::from_str(&content).expect("invalid JSON")
}

fn write_temp(value: &serde_json::Value) -> String {
    let path = format!("/tmp/evident_structural_{}.json", uuid_simple());
    std::fs::write(&path, serde_json::to_string(value).unwrap()).unwrap();
    path
}

fn proof_events_to_rows(proof: &serde_json::Value) -> Vec<EventRow> {
    proof["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| EventRow {
            event_id: Uuid::parse_str(e["event_id"].as_str().unwrap()).unwrap(),
            parent_event_id: Uuid::parse_str(e["parent_event_id"].as_str().unwrap()).unwrap(),
            file_hash: e["file_hash"].as_str().unwrap().to_string(),
            created_at: Utc::now(),
            sequence: e["sequence"].as_i64().unwrap(),
        })
        .collect()
}

fn proof_to_snapshot_bytes(proof: &serde_json::Value) -> Vec<u8> {
    let chain_id = Uuid::parse_str(proof["chain_id"].as_str().unwrap()).unwrap();
    let now = Utc::now();
    let events: Vec<EventSnapshot> = proof["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| EventSnapshot {
            event_id: Uuid::parse_str(e["event_id"].as_str().unwrap()).unwrap(),
            chain_id,
            parent_event_id: Uuid::parse_str(e["parent_event_id"].as_str().unwrap()).unwrap(),
            file_hash: e["file_hash"].as_str().unwrap().to_string(),
            idempotency_key: "k".into(),
            signature: String::new(),
            created_at: now,
            sequence: e["sequence"].as_i64().unwrap(),
        })
        .collect();

    let snapshot = BackupSnapshot {
        chain_id,
        exported_at: now,
        events,
    };
    serde_json::to_vec(&snapshot).unwrap()
}

#[test]
fn restore_empty_home_succeeds_with_disclaimer() {
    let tmp = TempDir::new().unwrap();
    let proof = load_proof();
    let backup_id = Uuid::new_v4();
    let bytes = proof_to_snapshot_bytes(&proof);

    let summary = restore_snapshot_bytes(tmp.path(), backup_id, &bytes, false, |_| false).unwrap();

    let mut capture = Vec::new();
    {
        use std::io::Write;
        writeln!(
            capture,
            "Restored: chain {}, {} events.",
            summary.chain_id, summary.event_count
        )
        .unwrap();
        writeln!(
            capture,
            "This restore validates structural consistency only:"
        )
        .unwrap();
        writeln!(
            capture,
            "event ordering, sequence continuity, and parent linkage."
        )
        .unwrap();
        writeln!(
            capture,
            "It does not verify cryptographic authenticity or confirm"
        )
        .unwrap();
        writeln!(
            capture,
            "that event contents match the authoritative ledger state."
        )
        .unwrap();
        writeln!(capture, "Before relying on restored data, run:").unwrap();
        writeln!(capture, "  evident verify --chain {}", summary.chain_id).unwrap();
    }
    let out = String::from_utf8_lossy(&capture);
    assert!(out.contains("structural consistency only"));
    assert!(out.contains("authoritative ledger state"));
    assert!(out.contains("evident verify --chain"));

    print_restore_summary(&summary);
}

#[test]
fn shared_sequence_corruption_fails_restore_and_verify() {
    let mut proof = load_proof();
    let events = proof["events"].as_array_mut().unwrap();
    events[1]["sequence"] = serde_json::json!(99);

    let rows = proof_events_to_rows(&proof);
    assert!(matches!(
        check_event_structure(&rows),
        Err(StructuralFailure::Sequence { .. })
    ));

    let snapshot =
        serde_json::from_slice::<BackupSnapshot>(&proof_to_snapshot_bytes(&proof)).unwrap();
    assert_eq!(
        validate_structural_integrity(&snapshot).unwrap_err(),
        STRUCTURAL_INTEGRITY_ERROR
    );

    let tmp = TempDir::new().unwrap();
    let backup_id = Uuid::new_v4();
    let err = restore_snapshot_bytes(
        tmp.path(),
        backup_id,
        &proof_to_snapshot_bytes(&proof),
        false,
        |_| true,
    )
    .unwrap_err();
    assert!(format!("{err}").contains(STRUCTURAL_INTEGRITY_ERROR));
    assert!(!tmp
        .path()
        .join("backups")
        .join(format!("{backup_id}.json"))
        .exists());

    let path = write_temp(&proof);
    let (output, code) = run_verifier(&path);
    assert_eq!(code, 2);
    assert!(output.contains("sequence not monotonic"));
}

#[test]
fn shared_parent_corruption_fails_restore_and_verify() {
    let mut proof = load_proof();
    let events = proof["events"].as_array_mut().unwrap();
    events[1]["parent_event_id"] = serde_json::json!("00000000-0000-0000-0000-000000000000");

    let rows = proof_events_to_rows(&proof);
    assert!(matches!(
        check_event_structure(&rows),
        Err(StructuralFailure::ParentChain { .. })
    ));

    let snapshot =
        serde_json::from_slice::<BackupSnapshot>(&proof_to_snapshot_bytes(&proof)).unwrap();
    assert_eq!(
        validate_structural_integrity(&snapshot).unwrap_err(),
        STRUCTURAL_INTEGRITY_ERROR
    );

    let tmp = TempDir::new().unwrap();
    let backup_id = Uuid::new_v4();
    restore_snapshot_bytes(
        tmp.path(),
        backup_id,
        &proof_to_snapshot_bytes(&proof),
        false,
        |_| true,
    )
    .unwrap_err();

    let path = write_temp(&proof);
    let (output, code) = run_verifier(&path);
    assert_eq!(code, 2);
    assert!(output.contains("parent mismatch"));
}

#[test]
fn merkle_tamper_fails_verify_but_restore_is_structural_only() {
    let mut proof = load_proof();
    let events = proof["events"].as_array_mut().unwrap();
    events[1]["file_hash"] = serde_json::json!("ff".repeat(64));

    let rows = proof_events_to_rows(&proof);
    let valid_root = check_event_structure(&proof_events_to_rows(&load_proof())).unwrap();
    let recomputed = check_event_structure(&rows).unwrap();
    assert_ne!(valid_root, recomputed);
    assert!(check_event_structure(&rows).is_ok());

    let tmp = TempDir::new().unwrap();
    let backup_id = Uuid::new_v4();
    let summary = restore_snapshot_bytes(
        tmp.path(),
        backup_id,
        &proof_to_snapshot_bytes(&proof),
        false,
        |_| false,
    )
    .expect("restore allows structurally valid tampered leaf");
    assert!(summary.output_path.exists());

    let path = write_temp(&proof);
    let (output, code) = run_verifier(&path);
    assert_eq!(code, 2);
    assert!(output.contains("merkle root mismatch"));
}

#[test]
fn restore_requires_confirmation_when_local_data_exists() {
    let tmp = TempDir::new().unwrap();
    let proof = load_proof();
    let chain_id = Uuid::parse_str(proof["chain_id"].as_str().unwrap()).unwrap();
    let backup_id = Uuid::new_v4();
    let bytes = proof_to_snapshot_bytes(&proof);

    let proofs_dir = tmp.path().join("proofs").join(chain_id.to_string());
    std::fs::create_dir_all(&proofs_dir).unwrap();
    std::fs::write(
        proofs_dir.join("local.json"),
        serde_json::to_string(&proof).unwrap(),
    )
    .unwrap();

    restore_snapshot_bytes(tmp.path(), backup_id, &bytes, false, |_| false).unwrap_err();
    assert!(!tmp
        .path()
        .join("backups")
        .join(format!("{backup_id}.json"))
        .exists());

    restore_snapshot_bytes(tmp.path(), backup_id, &bytes, true, |_| false).unwrap();
    assert!(tmp
        .path()
        .join("backups")
        .join(format!("{backup_id}.json"))
        .exists());
}

#[test]
fn restore_snapshot_bytes_has_no_network_dependency() {
    let tmp = TempDir::new().unwrap();
    let proof = load_proof();
    let backup_id = Uuid::new_v4();
    restore_snapshot_bytes(
        tmp.path(),
        backup_id,
        &proof_to_snapshot_bytes(&proof),
        false,
        |_| false,
    )
    .unwrap();
}
