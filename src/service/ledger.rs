use sqlx::PgPool;
use serde_json::{json, Value};
use uuid::Uuid;
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;
use axum::Json;

use crate::models::event::SubmitEventRequest;

#[derive(Debug)]
pub enum LedgerError {
    ChainNotFound,
    ParentMismatch,
    DuplicateIdempotencyKey,
    DatabaseError(sqlx::Error),
}

impl From<sqlx::Error> for LedgerError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db_err) = err {
            if db_err.constraint() == Some("uniq_idempotency") {
                return LedgerError::DuplicateIdempotencyKey;
            }
        }
        LedgerError::DatabaseError(err)
    }
}

impl IntoResponse for LedgerError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            LedgerError::ChainNotFound => (StatusCode::NOT_FOUND, "Chain not found"),
            LedgerError::ParentMismatch => (StatusCode::CONFLICT, "Parent hash mismatch — fork detected"),
            LedgerError::DuplicateIdempotencyKey => (StatusCode::CONFLICT, "Duplicate idempotency key"),
            LedgerError::DatabaseError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub async fn submit_event(
    pool: &PgPool,
    req: SubmitEventRequest,
) -> Result<Value, LedgerError> {

    let mut tx = pool.begin().await?;

    // 1. LOCK chain
    let chain = sqlx::query!(
        r#"
        SELECT chain_id, head_event_id
        FROM chains
        WHERE chain_id = $1
        FOR UPDATE
        "#,
        req.chain_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(LedgerError::ChainNotFound)?;

    // 2. CHECK idempotency
    if let Some(existing) = sqlx::query!(
        r#"
        SELECT event_id, file_hash
        FROM events
        WHERE chain_id = $1 AND idempotency_key = $2
        "#,
        req.chain_id,
        req.idempotency_key
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        return Ok(json!({
            "event_id": existing.event_id,
            "chain_id": req.chain_id,
            "head_event_id": chain.head_event_id,
            "cached": true
        }));
    }

    // 3. validate parent pointer (allow NULL for first event)
    match (chain.head_event_id, req.parent_event_id) {
        (None, parent) if parent == Uuid::nil() => {
            // First event - valid
        }
        (Some(head), parent) if head == parent => {
            // Valid parent
        }
        _ => {
            return Err(LedgerError::ParentMismatch);
        }
    }

    // 4. insert event
    let event_id = Uuid::new_v4();

    sqlx::query!(
        r#"
        INSERT INTO events (
            event_id,
            chain_id,
            parent_event_id,
            file_hash,
            idempotency_key,
            signature
        )
        VALUES ($1,$2,$3,$4,$5,$6)
        "#,
        event_id,
        req.chain_id,
        req.parent_event_id,
        req.file_hash,
        req.idempotency_key,
        req.signature
    )
    .execute(&mut *tx)
    .await?;

    // 5. update head
    sqlx::query!(
        r#"
        UPDATE chains
        SET head_event_id = $1
        WHERE chain_id = $2
        "#,
        event_id,
        req.chain_id
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // async TSA stamp
    {
        let pool_clone = pool.clone();
        let chain_id = req.chain_id;
        tokio::spawn(async move {
            if let Some(root) = compute_chain_root(&pool_clone, chain_id).await {
                crate::tsa_worker::stamp_chain(&pool_clone, chain_id, &root, event_id).await;
            }
        });
    }

    Ok(json!({
        "event_id": event_id,
        "chain_id": req.chain_id,
        "head_event_id": event_id,
        "cached": false
    }))
}

pub fn spawn_tsa_stamp(pool: PgPool, chain_id: Uuid, merkle_root: String, head_event_id: Uuid) {
    tokio::spawn(async move {
        crate::tsa_worker::stamp_chain(&pool, chain_id, &merkle_root, head_event_id).await;
    });
}

async fn compute_chain_root(pool: &PgPool, chain_id: Uuid) -> Option<String> {
    let events = sqlx::query_as!(
        crate::db::EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await
    .ok()?;

    if events.is_empty() { return None; }
    Some(crate::merkle::MerkleTree::recompute_root_from_events(&events))
}
