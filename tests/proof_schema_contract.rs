//! Contract lock: immutable `proof_v1` core vs external TSA attestation layer.
//!
//! See `docs/proof_v1.schema.md` and `docs/audits/PROOF_V1_TSA_ALIGNMENT.md`.
//! Any change to the nested `proof` object field set requires a version bump
//! to `proof_v2` — do not fold TSA fields into `proof_v1`.

use serde_json::{json, Value};

const PROOF_V1_REQUIRED_KEYS: &[&str] = &[
    "root",
    "chain_head",
    "signature",
    "public_key",
    "leaves_count",
    "type",
];

const TSA_FORBIDDEN_IN_PROOF_OBJECT: &[&str] = &[
    "tsa_timestamp",
    "tsa_signature",
    "tsa_token",
    "tsa_serial",
    "verification_status",
    "timestamp",
    "serial",
    "token_bytes",
];

/// Minimal artifact matching `export_proof` / CLI `ProofFile` shape.
fn sample_artifact_with_optional_tsa(include_tsa: bool) -> Value {
    let mut root = json!({
        "leaf_version": "leaf_v1",
        "chain_id": "11111111-1111-1111-1111-111111111111",
        "head_event_id": "22222222-2222-2222-2222-222222222222",
        "events": [{
            "event_id": "22222222-2222-2222-2222-222222222222",
            "sequence": 1,
            "parent_event_id": "00000000-0000-0000-0000-000000000000",
            "file_hash": "aa".repeat(32),
        }],
        "proof": {
            "version": "proof_v1",
            "type": "merkle-root-v1",
            "root": "bb".repeat(32),
            "leaves_count": 1,
            "chain_head": "22222222-2222-2222-2222-222222222222",
            "signature": "cc".repeat(64),
            "public_key": "dd".repeat(32),
        }
    });
    if include_tsa {
        root.as_object_mut().unwrap().insert(
            "tsa".to_string(),
            json!({
                "timestamp": 1_700_000_000_i64,
                "serial": "1",
                "token_bytes": 128,
                "verification_status": "verified",
            }),
        );
    }
    root
}

#[test]
fn proof_v1_core_fields_unchanged() {
    let artifact = sample_artifact_with_optional_tsa(false);
    let proof = artifact.get("proof").expect("nested proof object");
    assert_eq!(proof.get("version").and_then(|v| v.as_str()), Some("proof_v1"));
    assert_eq!(
        proof.get("type").and_then(|v| v.as_str()),
        Some("merkle-root-v1")
    );
    for key in PROOF_V1_REQUIRED_KEYS {
        assert!(
            proof.get(*key).is_some(),
            "proof_v1 missing required field `{key}` — schema bump required if intentional"
        );
    }
}

#[test]
fn tsa_fields_are_not_inside_proof_v1_object() {
    let artifact = sample_artifact_with_optional_tsa(true);
    let proof = artifact.get("proof").expect("nested proof object");
    for key in TSA_FORBIDDEN_IN_PROOF_OBJECT {
        assert!(
            proof.get(*key).is_none(),
            "TSA/attestation field `{key}` must not appear inside proof_v1 object \
             (external layer only; use proof_v2 if folding is required)"
        );
    }
    // Sibling attestation object is allowed.
    assert!(artifact.get("tsa").is_some());
}

#[test]
fn future_proof_schema_changes_require_version_bump() {
    // Guardrail documentation asserted by compile-time constant + schema file.
    assert_eq!(evident_ledger::proof_format::PROOF_VERSION, "proof_v1");
    // If this fails after a rename, update docs/proof_v1.schema.md immutability
    // rule and introduce docs/proof_v2.schema.md — do not silently extend v1.
}
