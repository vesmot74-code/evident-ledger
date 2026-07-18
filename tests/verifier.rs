mod verifier_harness;

use verifier_harness::{load_proof, run_verifier};

#[test]
fn valid_proof_exits_0() {
    let proof = load_proof();
    let (_, code) = run_verifier(&proof);
    assert_eq!(code, 0);
}

#[test]
fn tampered_signature_exits_2() {
    let mut proof = load_proof();
    proof["proof"]["signature"] = serde_json::json!("ff".repeat(64));
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 2);
    assert!(output.contains("signature invalid"));
}

#[test]
fn tampered_head_event_id_exits_2() {
    let mut proof = load_proof();
    proof["head_event_id"] = serde_json::json!("00000000-0000-0000-0000-000000000000");
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 2);
    assert!(output.contains("head_event_id"));
}

#[test]
fn missing_event_exits_2() {
    let mut proof = load_proof();
    let events = proof["events"].as_array_mut().unwrap();
    events.pop();
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 2);
    assert!(output.contains("leaves_count"));
}

#[test]
fn tampered_event_hash_causes_merkle_root_mismatch() {
    let mut proof = load_proof();
    let original_signature = proof["proof"]["signature"].clone();
    let events = proof["events"].as_array_mut().unwrap();
    let index = events.len() / 2;
    events[index]["file_hash"] = serde_json::json!("0".repeat(64));
    assert_eq!(proof["proof"]["signature"], original_signature);
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 2);
    assert!(output.contains("merkle root mismatch"));
    assert!(!output.contains("signature invalid"));
}

#[test]
fn unversioned_legacy_proof_rejected_with_clear_message() {
    let mut proof = load_proof();
    let obj = proof.as_object_mut().unwrap();
    obj.remove("leaf_version");
    proof["proof"].as_object_mut().unwrap().remove("version");
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 4);
    assert!(output.contains("unversioned legacy proof format — unsupported, please regenerate"));
    assert!(!output.contains("merkle root mismatch"));
}

#[test]
fn missing_proof_version_rejected_as_unsupported_format() {
    let mut proof = load_proof();
    proof["proof"].as_object_mut().unwrap().remove("version");
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 4);
    assert!(output.contains("unsupported proof format"));
    assert!(!output.contains("merkle root mismatch"));
}

#[test]
fn tampered_event_id_causes_merkle_root_mismatch() {
    let mut proof = load_proof();
    let original_signature = proof["proof"]["signature"].clone();
    let events = proof["events"].as_array_mut().unwrap();
    // Tamper head leaf only: parent chain and signed head fields stay unchanged,
    // but merkle recompute must include the new event_id.
    let index = events.len() - 1;
    events[index]["event_id"] = serde_json::json!("11111111-1111-1111-1111-111111111111");
    assert_eq!(proof["proof"]["signature"], original_signature);
    let (output, code) = run_verifier(&proof);
    assert_eq!(code, 2);
    assert!(output.contains("merkle root mismatch"));
    assert!(!output.contains("signature invalid"));
}
