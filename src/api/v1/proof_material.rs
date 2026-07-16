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
//! 4. store idempotency record + commit transaction
//! 5. return response (TSA stamp runs after commit, optional)

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::db::EventRow;
use crate::merkle::MerkleTree;
use crate::proof_format::{LEAF_VERSION, PROOF_TYPE, PROOF_VERSION};
use crate::signing::{verify_root, ServerSigner};

use super::event_access::Event;
use super::errors::ApiError;
use super::proof_status::{derive_proof_status, ProofContext, ProofStatus};

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

    let failure_signal = false; // Stage 3+: persisted failure sources

    let context = ProofContext {
        merkle_root_present,
        signature_present,
        signature_valid,
        failure_signal,
    };

    ProofSnapshot {
        merkle_root,
        signature,
        public_key,
        context,
    }
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
    token_bytes: i32,
}

async fn load_tsa_for_root(
    pool: &PgPool,
    chain_id: Uuid,
    merkle_root: &str,
) -> Result<Option<Value>, sqlx::Error> {
    let row = sqlx::query_as::<_, TsaRow>(
        r#"
        SELECT tsa_timestamp, tsa_serial, length(tsa_token) AS token_bytes
        FROM tsa_tokens
        WHERE chain_id = $1 AND merkle_root = $2
        "#,
    )
    .bind(chain_id)
    .bind(merkle_root)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|t| {
        json!({
            "timestamp": t.tsa_timestamp,
            "serial": t.tsa_serial,
            "token_bytes": t.token_bytes,
        })
    }))
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

    let snapshot = build_proof_snapshot(signer, event.chain_id, event.event_id, &prefix);
    let status = derive_proof_status(&snapshot.context);

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
            let tsa = load_tsa_for_root(pool, event.chain_id, &snapshot.merkle_root)
                .await
                .map_err(|_| ApiError::Internal)?;
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
        let full_chain = vec![
            row(e1, parent, 1, &hash),
            row(e2, e1, 2, &hash),
        ];

        let root_prefix = MerkleTree::recompute_root_from_events(&prefix_only);
        let root_full = MerkleTree::recompute_root_from_events(&full_chain);
        assert_ne!(root_prefix, root_full);
    }

    #[test]
    fn empty_prefix_yields_pending_proof_status() {
        let signer = ServerSigner::load_or_create("target/test_proof_pending_signing.key");
        let chain_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();

        let snapshot = build_proof_snapshot(&signer, chain_id, event_id, &[]);
        assert_eq!(derive_proof_status(&snapshot.context), ProofStatus::Pending);

        let event = Event {
            event_id,
            chain_id,
            account_id: Uuid::new_v4(),
            parent_event_id: Uuid::nil(),
            file_hash: "cc".repeat(32),
            sequence: 1,
            created_at: Utc::now(),
        };
        let body = pending_proof_response(&event, Uuid::new_v4());
        assert_eq!(body["proof_status"], "pending");
        assert_eq!(body["event_id"], event_id.to_string());
        assert!(body.get("merkle_root").is_none());
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
}
