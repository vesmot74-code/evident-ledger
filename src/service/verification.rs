use crate::db::EventRow;
use crate::merkle::MerkleTree;
use crate::signing::ServerSigner;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub const STRUCTURAL_INTEGRITY_ERROR: &str =
    "snapshot appears corrupted or incomplete — restore aborted";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralFailure {
    ParentChain { index: usize },
    Sequence { index: usize },
    EmptyMerkle,
}

/// Parent chain, monotonic sequence (starting at 1), and merkle recompute.
/// Returns the recomputed merkle root on success.
pub fn check_event_structure(events: &[EventRow]) -> Result<String, StructuralFailure> {
    if events.is_empty() {
        return Ok("empty".to_string());
    }

    for (i, event) in events.iter().enumerate() {
        if i == 0 {
            if event.parent_event_id != Uuid::nil() {
                return Err(StructuralFailure::ParentChain { index: i });
            }
            if event.sequence != 1 {
                return Err(StructuralFailure::Sequence { index: i });
            }
        } else {
            let prev = &events[i - 1];
            if event.sequence != prev.sequence + 1 {
                return Err(StructuralFailure::Sequence { index: i });
            }
            if event.parent_event_id != prev.event_id {
                return Err(StructuralFailure::ParentChain { index: i });
            }
        }
    }

    let merkle_root = MerkleTree::recompute_root_from_events(events);
    if merkle_root.is_empty() {
        return Err(StructuralFailure::EmptyMerkle);
    }

    Ok(merkle_root)
}

pub async fn verify_chain(
    pool: &PgPool,
    signer: &Arc<ServerSigner>,
    chain_id: Uuid,
) -> Result<serde_json::Value, sqlx::Error> {
    let events: Vec<EventRow> = sqlx::query_as!(
        EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await?;

    if events.is_empty() {
        return Ok(json!({
            "chain_id": chain_id,
            "valid": true,
            "blocks": 0,
            "message": "Chain is empty",
            "proof": null,
        }));
    }

    let mut valid = true;
    let mut errors = Vec::new();

    if let Err(failure) = check_event_structure(&events) {
        valid = false;
        errors.push(format!("Structural check failed: {failure:?}"));
    }

    let merkle_root = MerkleTree::recompute_root_from_events(&events);
    let chain_head = events.last().map(|e| e.event_id).unwrap();
    let signature = signer.sign_root(&chain_id.to_string(), &merkle_root, &chain_head.to_string());
    let public_key = signer.public_key_hex();

    Ok(json!({
        "chain_id": chain_id,
        "valid": valid,
        "blocks": events.len(),
        "errors": errors,
        "head_event_id": chain_head,
        "proof": {
            "version": "proof_v1", "type": "merkle-root-v1",
            "root": merkle_root,
            "leaves_count": events.len(),
            "chain_head": chain_head,
            "signature": signature,
            "public_key": public_key,
        }
    }))
}

pub async fn export_proof(
    pool: &PgPool,
    signer: &Arc<ServerSigner>,
    chain_id: Uuid,
) -> Result<serde_json::Value, sqlx::Error> {
    let events: Vec<EventRow> = sqlx::query_as!(
        EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await?;

    if events.is_empty() {
        return Ok(json!({ "chain_id": chain_id, "events": [], "proof": null }));
    }

    let merkle_root = MerkleTree::recompute_root_from_events(&events);
    let chain_head = events.last().unwrap().event_id;
    let signature = signer.sign_root(&chain_id.to_string(), &merkle_root, &chain_head.to_string());
    let public_key = signer.public_key_hex();

    let tsa = sqlx::query!(
        r#"SELECT tsa_timestamp, tsa_serial, length(tsa_token) as token_bytes
           FROM tsa_tokens WHERE chain_id = $1 AND merkle_root = $2"#,
        chain_id,
        merkle_root
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let leaves: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            json!({
                "sequence": e.sequence,
                "event_id": e.event_id,
                "parent_event_id": e.parent_event_id,
                "file_hash": e.file_hash,
            })
        })
        .collect();

    Ok(json!({
        "chain_id": chain_id,
        "head_event_id": chain_head,
        "events": leaves,
        "proof": {
            "version": "proof_v1", "type": "merkle-root-v1",
            "root": merkle_root,
            "leaves_count": events.len(),
            "chain_head": chain_head,
            "signature": signature,
            "public_key": public_key,
        },
        "tsa": tsa.map(|t| json!({
            "timestamp": t.tsa_timestamp,
            "serial": t.tsa_serial,
            "token_bytes": t.token_bytes,
        })),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_rows() -> Vec<EventRow> {
        let e1 = Uuid::new_v4();
        let e2 = Uuid::new_v4();
        let now = Utc::now();
        vec![
            EventRow {
                event_id: e1,
                parent_event_id: Uuid::nil(),
                file_hash: "aa".repeat(32),
                created_at: now,
                sequence: 1,
            },
            EventRow {
                event_id: e2,
                parent_event_id: e1,
                file_hash: "bb".repeat(32),
                created_at: now,
                sequence: 2,
            },
        ]
    }

    #[test]
    fn valid_events_pass() {
        assert!(check_event_structure(&sample_rows()).is_ok());
    }

    #[test]
    fn broken_parent_fails() {
        let mut rows = sample_rows();
        rows[1].parent_event_id = Uuid::new_v4();
        assert!(matches!(
            check_event_structure(&rows),
            Err(StructuralFailure::ParentChain { index: 1 })
        ));
    }

    #[test]
    fn broken_sequence_fails() {
        let mut rows = sample_rows();
        rows[1].sequence = 99;
        assert!(matches!(
            check_event_structure(&rows),
            Err(StructuralFailure::Sequence { index: 1 })
        ));
    }

    #[test]
    fn broken_merkle_via_file_hash_fails_mismatch_at_consumer() {
        let rows = sample_rows();
        let root = check_event_structure(&rows).unwrap();
        let mut tampered = rows.clone();
        tampered[1].file_hash = "cc".repeat(32);
        let recomputed = check_event_structure(&tampered).unwrap();
        assert_ne!(root, recomputed);
    }
}
