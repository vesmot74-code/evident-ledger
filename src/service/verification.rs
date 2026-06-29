use sqlx::PgPool;
use serde_json::json;
use uuid::Uuid;
use std::sync::Arc;
use crate::merkle::MerkleTree;
use crate::db::EventRow;
use crate::signing::ServerSigner;

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

    for (i, event) in events.iter().enumerate() {
        if i == 0 {
            if event.parent_event_id != Uuid::nil() {
                valid = false;
                errors.push(format!(
                    "First event {} has parent {} instead of nil",
                    event.event_id, event.parent_event_id
                ));
            }
        } else {
            let prev = &events[i - 1];
            if event.parent_event_id != prev.event_id {
                valid = false;
                errors.push(format!(
                    "Event {} has parent {} but previous is {}",
                    event.event_id, event.parent_event_id, prev.event_id
                ));
            }
        }
    }

    let merkle_root = MerkleTree::recompute_root_from_events(&events);
    let chain_head = events.last().map(|e| e.event_id).unwrap();
    let signature = signer.sign_root(&merkle_root, &chain_head.to_string());
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
    let signature = signer.sign_root(&merkle_root, &chain_head.to_string());
    let public_key = signer.public_key_hex();

    let leaves: Vec<serde_json::Value> = events.iter().map(|e| json!({
        "sequence": e.sequence,
        "event_id": e.event_id,
        "parent_event_id": e.parent_event_id,
        "file_hash": e.file_hash,
    })).collect();

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
        }
    }))
}
