//! Proof artifact assembly for API v1.
//!
//! ## Snapshot semantics (evidence model)
//!
//! `GET /v1/proof/{event_id}` and commit-time proof both use the **chain state at the
//! target event's commit moment**:
//! - Merkle root is computed from events with `sequence <= target.sequence` only.
//! - `chain_head` in the signature message is the **target event's id** (not the
//!   current chain head).
//!
//! Recompute on read is safe: Ed25519 is deterministic and inputs are historical.
//!
//! ## POST /v1/events proof flow (synchronous)
//!
//! 1. validate request
//! 2. create immutable event (inside transaction)
//! 3. derive proof snapshot for that event (`sequence` prefix + sign)
//! 4. persist signature on the event row + idempotency record; commit transaction
//! 5. return response (TSA stamp runs after commit, optional)

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::db::EventRow;
use crate::merkle::MerkleTree;
use crate::proof_format::{LEAF_VERSION, PROOF_TYPE, PROOF_VERSION};
use crate::signing::{verify_root, ServerSigner};
use crate::tsa::{verify_tsa_attestation, TsaAttestation, TsaStatus, TsaTrustLevel};

use super::errors::ApiError;
use super::event_access::Event;
use super::proof_status::{derive_proof_status, ProofContext, ProofStatus};

/// Runtime failure conditions 1 and 2 (Stage 4 §3 PR1). TSA (condition 4) is PR2.
pub(crate) fn detect_failure_signal(ctx: &ProofContext) -> bool {
    (ctx.signature_present && !ctx.signature_valid)
        || (!ctx.merkle_root_present && ctx.signature_present)
}

fn proof_context_from_parts(
    merkle_root_present: bool,
    signature_present: bool,
    signature_valid: bool,
) -> ProofContext {
    let partial = ProofContext {
        merkle_root_present,
        signature_present,
        signature_valid,
        failure_signal: false,
    };
    ProofContext {
        failure_signal: detect_failure_signal(&partial),
        ..partial
    }
}

/// Runtime failure condition 4 (Stage 4 §3 PR2). Separate from conditions 1+2.
pub(crate) fn tsa_validation_failure_signal(
    tsa_row_present: bool,
    validation_status: TsaStatus,
) -> bool {
    tsa_row_present && validation_status == TsaStatus::Failed
}

fn tsa_validation_status(att: &TsaAttestation, bundle_hash: &str) -> TsaStatus {
    verify_tsa_attestation(att, bundle_hash)
}

/// Evident stub tokens are JSON objects from `create_stub_attestation` only.
/// RFC3161 DER (FreeTSA) rows are not validated for failure_signal in PR2 (PR3).
fn is_evident_stub_json_token(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.contains("\"stub\":true"))
}

fn stub_sha256_from_token_bytes(token: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(token).ok()?;
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    payload.get("sha256")?.as_str().map(str::to_string)
}

fn tsa_attestation_from_stub_row(row: &TsaRow) -> Option<TsaAttestation> {
    if !is_evident_stub_json_token(&row.tsa_token) {
        return None;
    }
    let tsr_hash = stub_sha256_from_token_bytes(&row.tsa_token)?;
    Some(TsaAttestation {
        provider: "stub".to_string(),
        timestamp: row.tsa_timestamp,
        tsr_hash,
        // signature_valid=true here does NOT mean an Ed25519/cryptographic signature
        // was verified — stub tokens have no such signature. It signals only that
        // this material is eligible for the stub verification path
        // (validate_stub_token), which checks binding via JSON content instead.
        // External RFC3161 tokens are not validated in this PR (see PR3).
        signature_valid: true,
        raw_token_b64: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &row.tsa_token,
        ),
        trust_level: TsaTrustLevel::Stub,
    })
}

fn proof_context_with_tsa(
    base: ProofContext,
    tsa_row_present: bool,
    validation_status: TsaStatus,
) -> ProofContext {
    ProofContext {
        failure_signal: base.failure_signal
            || tsa_validation_failure_signal(tsa_row_present, validation_status),
        ..base
    }
}

#[derive(Debug, sqlx::FromRow)]
struct EventChainRow {
    event_id: Uuid,
    parent_event_id: Uuid,
    file_hash: String,
    created_at: DateTime<Utc>,
    sequence: i64,
}

impl From<EventChainRow> for EventRow {
    fn from(row: EventChainRow) -> Self {
        Self {
            event_id: row.event_id,
            parent_event_id: row.parent_event_id,
            file_hash: row.file_hash,
            created_at: row.created_at,
            sequence: row.sequence,
        }
    }
}

/// Events in the chain prefix ending at `target_sequence` (inclusive).
pub async fn load_event_prefix(
    conn: &mut PgConnection,
    chain_id: Uuid,
    target_sequence: i64,
) -> Result<Vec<EventRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EventChainRow>(
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1 AND sequence <= $2
        ORDER BY sequence ASC
        "#,
    )
    .bind(chain_id)
    .bind(target_sequence)
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows.into_iter().map(EventRow::from).collect())
}

pub struct ProofSnapshot {
    pub merkle_root: String,
    pub signature: String,
    pub public_key: String,
    pub context: ProofContext,
}

/// Builds proof crypto material for the commit-time snapshot of `target_event_id`.
pub fn build_proof_snapshot(
    signer: &ServerSigner,
    chain_id: Uuid,
    target_event_id: Uuid,
    prefix_events: &[EventRow],
) -> ProofSnapshot {
    let merkle_root = MerkleTree::recompute_root_from_events(prefix_events);
    let merkle_root_present = !prefix_events.is_empty() && !merkle_root.is_empty();

    let chain_head = target_event_id.to_string();
    let signature = signer.sign_root(&chain_id.to_string(), &merkle_root, &chain_head);
    let signature_present = !signature.is_empty();
    let public_key = signer.public_key_hex();
    let signature_valid = signature_present
        && verify_root(
            &chain_id.to_string(),
            &merkle_root,
            &chain_head,
            &signature,
            &public_key,
        );

    // Commit path: failure conditions are structurally impossible after
    // atomic signing (merkle + signature always both present and valid
    // by construction). Helper is applied here to keep the invariant
    // aligned with the read path, not because commit-time failures are
    // expected.
    let context = proof_context_from_parts(merkle_root_present, signature_present, signature_valid);

    ProofSnapshot {
        merkle_root,
        signature,
        public_key,
        context,
    }
}

/// Builds a read-path snapshot using the persisted commit-time signature (no re-sign).
pub fn build_proof_snapshot_read(
    chain_id: Uuid,
    target_event_id: Uuid,
    prefix_events: &[EventRow],
    persisted_signature: &str,
    public_key: &str,
) -> ProofSnapshot {
    let merkle_root = MerkleTree::recompute_root_from_events(prefix_events);
    let merkle_root_present = !prefix_events.is_empty() && !merkle_root.is_empty();

    let chain_head = target_event_id.to_string();
    let signature = persisted_signature.to_string();
    let signature_present = !signature.is_empty();
    let signature_valid = signature_present
        && verify_root(
            &chain_id.to_string(),
            &merkle_root,
            &chain_head,
            &signature,
            public_key,
        );

    let context = proof_context_from_parts(merkle_root_present, signature_present, signature_valid);

    ProofSnapshot {
        merkle_root,
        signature,
        public_key: public_key.to_string(),
        context,
    }
}

pub async fn proof_snapshot_at_event(
    conn: &mut PgConnection,
    signer: &ServerSigner,
    chain_id: Uuid,
    target_event_id: Uuid,
    target_sequence: i64,
) -> Result<ProofSnapshot, sqlx::Error> {
    let prefix = load_event_prefix(conn, chain_id, target_sequence).await?;
    Ok(build_proof_snapshot(
        signer,
        chain_id,
        target_event_id,
        &prefix,
    ))
}

/// Writes the commit-time server signature for an event (inside the open transaction).
pub async fn persist_event_signature(
    conn: &mut PgConnection,
    event_id: Uuid,
    signature: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE events SET signature = $1 WHERE event_id = $2")
        .bind(signature)
        .bind(event_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn proof_context_at_event(
    conn: &mut PgConnection,
    signer: &ServerSigner,
    chain_id: Uuid,
    target_event_id: Uuid,
    target_sequence: i64,
) -> Result<ProofContext, sqlx::Error> {
    let prefix = load_event_prefix(conn, chain_id, target_sequence).await?;
    Ok(build_proof_snapshot(signer, chain_id, target_event_id, &prefix).context)
}

#[derive(Debug, sqlx::FromRow)]
struct TsaRow {
    tsa_timestamp: i64,
    tsa_serial: String,
    tsa_token: Vec<u8>,
}

async fn load_tsa_row_for_root(
    pool: &PgPool,
    chain_id: Uuid,
    merkle_root: &str,
) -> Result<Option<TsaRow>, sqlx::Error> {
    sqlx::query_as::<_, TsaRow>(
        r#"
        SELECT tsa_timestamp, tsa_serial, tsa_token
        FROM tsa_tokens
        WHERE chain_id = $1 AND merkle_root = $2
        "#,
    )
    .bind(chain_id)
    .bind(merkle_root)
    .fetch_optional(pool)
    .await
}

fn pending_proof_response(event: &Event, request_id: Uuid) -> Value {
    json!({
        "event_id": event.event_id,
        "chain_id": event.chain_id,
        "sequence": event.sequence,
        "proof_status": ProofStatus::Pending.as_str(),
        "request_id": request_id,
    })
}

fn anchored_proof_response(
    event: &Event,
    snapshot: &ProofSnapshot,
    tsa: Option<Value>,
    request_id: Uuid,
) -> Value {
    json!({
        "proof_version": PROOF_VERSION,
        "proof_type": PROOF_TYPE,
        "leaf_version": LEAF_VERSION,
        "event_id": event.event_id,
        "chain_id": event.chain_id,
        "sequence": event.sequence,
        "parent_event_id": event.parent_event_id,
        "file_hash": event.file_hash,
        "merkle_root": snapshot.merkle_root,
        "signature": snapshot.signature,
        "public_key": snapshot.public_key,
        "tsa": tsa,
        "created_at": event.created_at.to_rfc3339(),
        "proof_status": ProofStatus::Anchored.as_str(),
        "request_id": request_id,
    })
}

/// `GET /v1/proof/{event_id}` — ownership must be verified before calling.
pub async fn build_proof_response(
    pool: &PgPool,
    signer: &ServerSigner,
    event: &Event,
) -> Result<Value, ApiError> {
    let request_id = ApiError::request_id();
    let mut conn = pool.acquire().await.map_err(|_| ApiError::Internal)?;

    let prefix = load_event_prefix(&mut *conn, event.chain_id, event.sequence)
        .await
        .map_err(|_| ApiError::Internal)?;

    let public_key = signer.public_key_hex();
    let snapshot = build_proof_snapshot_read(
        event.chain_id,
        event.event_id,
        &prefix,
        &event.signature,
        &public_key,
    );

    // Stage 4 §3 PR2: TSA validation added to proof read path.
    // Latency impact not measured in this PR; if GET /v1/proof read
    // path becomes a bottleneck, consider caching TSA validation
    // results keyed by (chain_id, merkle_root).
    let tsa_row = load_tsa_row_for_root(pool, event.chain_id, &snapshot.merkle_root)
        .await
        .map_err(|_| ApiError::Internal)?;
    let stub_attestation = tsa_row.as_ref().and_then(tsa_attestation_from_stub_row);
    let validation_status = stub_attestation
        .as_ref()
        .map(|att| tsa_validation_status(att, &snapshot.merkle_root))
        .unwrap_or(TsaStatus::NotProvided);
    let context = proof_context_with_tsa(
        snapshot.context.clone(),
        stub_attestation.is_some(),
        validation_status,
    );
    let status = derive_proof_status(&context);

    match status {
        ProofStatus::Pending => Ok(pending_proof_response(event, request_id)),
        ProofStatus::Failed => {
            // Reserved: explicit failure_signal only; treat like pending envelope for GET.
            Ok(json!({
                "event_id": event.event_id,
                "chain_id": event.chain_id,
                "sequence": event.sequence,
                "proof_status": ProofStatus::Failed.as_str(),
                "request_id": request_id,
            }))
        }
        ProofStatus::Anchored => {
            let tsa = tsa_row.map(|t| {
                json!({
                    "timestamp": t.tsa_timestamp,
                    "serial": t.tsa_serial,
                    "token_bytes": t.tsa_token.len() as i64,
                })
            });
            Ok(anchored_proof_response(event, &snapshot, tsa, request_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn row(id: Uuid, parent: Uuid, seq: i64, hash: &str) -> EventRow {
        EventRow {
            event_id: id,
            parent_event_id: parent,
            file_hash: hash.to_string(),
            created_at: Utc::now(),
            sequence: seq,
        }
    }

    #[test]
    fn prefix_root_excludes_later_events() {
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let parent = Uuid::nil();
        let hash = "aa".repeat(32);

        let prefix_only = vec![row(e1, parent, 1, &hash)];
        let full_chain = vec![row(e1, parent, 1, &hash), row(e2, e1, 2, &hash)];

        let root_prefix = MerkleTree::recompute_root_from_events(&prefix_only);
        let root_full = MerkleTree::recompute_root_from_events(&full_chain);
        assert_ne!(root_prefix, root_full);
    }

    #[test]
    fn empty_prefix_yields_pending_proof_status() {
        let signer = ServerSigner::load_or_create("target/test_proof_pending_signing.key");
        let chain_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();

        let snapshot =
            build_proof_snapshot_read(chain_id, event_id, &[], "", &signer.public_key_hex());
        assert_eq!(derive_proof_status(&snapshot.context), ProofStatus::Pending);

        let event = Event {
            event_id,
            chain_id,
            account_id: Uuid::new_v4(),
            parent_event_id: Uuid::nil(),
            file_hash: "cc".repeat(32),
            sequence: 1,
            created_at: Utc::now(),
            signature: String::new(),
        };
        let body = pending_proof_response(&event, Uuid::new_v4());
        assert_eq!(body["proof_status"], "pending");
        assert_eq!(body["event_id"], event_id.to_string());
        assert!(body.get("merkle_root").is_none());
    }

    #[test]
    fn empty_prefix_non_empty_signature_is_failed() {
        let signer = ServerSigner::load_or_create("target/test_empty_prefix_sig.key");
        let chain_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();

        let snapshot = build_proof_snapshot_read(
            chain_id,
            event_id,
            &[],
            "bb".repeat(64).as_str(),
            &signer.public_key_hex(),
        );
        assert_eq!(derive_proof_status(&snapshot.context), ProofStatus::Failed);
    }

    #[test]
    fn detect_failure_signal_invalid_signature() {
        let ctx = ProofContext {
            merkle_root_present: true,
            signature_present: true,
            signature_valid: false,
            failure_signal: false,
        };
        assert!(detect_failure_signal(&ctx));
    }

    #[test]
    fn detect_failure_signal_merkle_missing_with_signature() {
        let ctx = ProofContext {
            merkle_root_present: false,
            signature_present: true,
            signature_valid: true,
            failure_signal: false,
        };
        assert!(detect_failure_signal(&ctx));
    }

    #[test]
    fn detect_failure_signal_incomplete_material_is_not_failure() {
        let ctx = ProofContext {
            merkle_root_present: false,
            signature_present: false,
            signature_valid: false,
            failure_signal: false,
        };
        assert!(!detect_failure_signal(&ctx));
    }

    #[test]
    fn detect_failure_signal_valid_signature_is_not_failure() {
        let ctx = ProofContext {
            merkle_root_present: true,
            signature_present: true,
            signature_valid: true,
            failure_signal: false,
        };
        assert!(!detect_failure_signal(&ctx));
    }

    #[test]
    fn invalid_persisted_signature_read_path_is_failed() {
        let signer = ServerSigner::load_or_create("target/test_invalid_persisted_sig.key");
        let chain_id = Uuid::new_v4();
        let e1 = Uuid::new_v4();
        let parent = Uuid::nil();
        let hash = "ee".repeat(32);
        let prefix = vec![row(e1, parent, 1, &hash)];

        let snapshot = build_proof_snapshot_read(
            chain_id,
            e1,
            &prefix,
            "aa".repeat(64).as_str(),
            &signer.public_key_hex(),
        );
        assert_eq!(derive_proof_status(&snapshot.context), ProofStatus::Failed);
    }

    #[test]
    fn snapshot_at_event_one_is_stable_when_chain_grows() {
        let signer = ServerSigner::load_or_create("target/test_proof_snapshot_signing.key");
        let chain_id = Uuid::new_v4();
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let parent = Uuid::nil();
        let hash = "bb".repeat(32);

        let at_commit = vec![row(e1, parent, 1, &hash)];
        let snap1 = build_proof_snapshot(&signer, chain_id, e1, &at_commit);

        let later_chain = vec![row(e1, parent, 1, &hash), row(e2, e1, 2, &hash)];
        let snap1_recomputed = build_proof_snapshot(&signer, chain_id, e1, &at_commit);
        let snap2_head = build_proof_snapshot(&signer, chain_id, e2, &later_chain);

        assert_eq!(snap1.merkle_root, snap1_recomputed.merkle_root);
        assert_eq!(snap1.signature, snap1_recomputed.signature);
        assert_ne!(snap1.merkle_root, snap2_head.merkle_root);
    }

    #[test]
    fn persisted_signature_matches_commit_time_recompute() {
        let signer = ServerSigner::load_or_create("target/test_persisted_sig_signing.key");
        let chain_id = Uuid::new_v4();
        let e1 = Uuid::new_v4();
        let parent = Uuid::nil();
        let hash = "dd".repeat(32);
        let prefix = vec![row(e1, parent, 1, &hash)];

        let at_commit = build_proof_snapshot(&signer, chain_id, e1, &prefix);
        let from_persisted = build_proof_snapshot_read(
            chain_id,
            e1,
            &prefix,
            &at_commit.signature,
            &signer.public_key_hex(),
        );

        assert_eq!(at_commit.merkle_root, from_persisted.merkle_root);
        assert_eq!(at_commit.signature, from_persisted.signature);
        assert_eq!(from_persisted.context.signature_valid, true);
    }

    #[test]
    fn tsa_validation_failure_signal_absent_row_is_not_failure() {
        assert!(!tsa_validation_failure_signal(false, TsaStatus::Failed));
    }

    #[test]
    fn tsa_validation_failure_signal_valid_status_is_not_failure() {
        assert!(!tsa_validation_failure_signal(true, TsaStatus::Verified));
    }

    #[test]
    fn tsa_validation_failure_signal_failed_status_is_failure() {
        assert!(tsa_validation_failure_signal(true, TsaStatus::Failed));
    }

    #[test]
    fn tsa_validation_status_valid_stub_is_verified() {
        use crate::tsa::create_stub_attestation;

        let hash = "bb".repeat(64);
        let att = create_stub_attestation(&hash, "stub");
        assert_eq!(tsa_validation_status(&att, &hash), TsaStatus::Verified);
    }

    #[test]
    fn tsa_validation_status_invalid_stub_is_failed() {
        use crate::tsa::create_stub_attestation;

        let hash = "cc".repeat(64);
        let mut att = create_stub_attestation(&hash, "stub");
        att.tsr_hash = "dd".repeat(64);
        assert_eq!(tsa_validation_status(&att, &hash), TsaStatus::Failed);
    }

    #[test]
    fn tsa_attestation_from_stub_row_uses_hash_from_token_not_lookup_key() {
        use crate::tsa::create_stub_attestation;

        let merkle_root = "ee".repeat(64);
        let att = create_stub_attestation(&merkle_root, "stub");
        let token_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            att.raw_token_b64.trim(),
        )
        .unwrap();
        let row = TsaRow {
            tsa_timestamp: att.timestamp,
            tsa_serial: "stub-serial".to_string(),
            tsa_token: token_bytes,
        };

        let parsed = tsa_attestation_from_stub_row(&row).expect("stub row");
        assert_eq!(parsed.tsr_hash, merkle_root);

        let wrong_merkle = "ff".repeat(64);
        assert_eq!(
            tsa_validation_status(&parsed, &wrong_merkle),
            TsaStatus::Failed
        );
    }

    #[test]
    fn non_stub_tsa_row_does_not_produce_stub_attestation() {
        let row = TsaRow {
            tsa_timestamp: 1,
            tsa_serial: "external".to_string(),
            tsa_token: vec![0x30, 0x03, 0x01, 0x01],
        };
        assert!(tsa_attestation_from_stub_row(&row).is_none());
    }

    #[test]
    fn proof_context_with_tsa_valid_signature_and_no_row_is_not_failure() {
        let base = proof_context_from_parts(true, true, true);
        let merged = proof_context_with_tsa(base, false, TsaStatus::NotProvided);
        assert!(!merged.failure_signal);
        assert_eq!(derive_proof_status(&merged), ProofStatus::Anchored);
    }
}
