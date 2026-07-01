use sqlx::PgPool;
use serde_json::{json, Value};
use uuid::Uuid;
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;
use axum::Json;

use crate::{models::event::SubmitEventRequest, signing::ServerSigner};

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
    signer: &ServerSigner,
    req: SubmitEventRequest,
) -> Result<Value, LedgerError> {

    let mut tx = pool.begin().await?;

    #[derive(sqlx::FromRow)]
    struct ChainRow {
        chain_id: Uuid,
        head_event_id: Option<Uuid>,
    }

    let chain = sqlx::query_as::<_, ChainRow>(
        r#"
        INSERT INTO chains (chain_id, head_event_id)
        VALUES ($1, NULL)
        ON CONFLICT (chain_id) DO NOTHING
        RETURNING chain_id, head_event_id
        "#
    )
    .bind(req.chain_id)
    .fetch_optional(&mut *tx)
    .await?;

    let chain = match chain {
        Some(chain) => chain,
        None => {
            sqlx::query_as::<_, ChainRow>(
                r#"
                SELECT chain_id, head_event_id
                FROM chains
                WHERE chain_id = $1
                FOR UPDATE
                "#
            )
            .bind(req.chain_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(LedgerError::ChainNotFound)?
        }
    };

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

    let parent_event_id = chain.head_event_id.unwrap_or(Uuid::nil());

    let sequence = sqlx::query_scalar!(
        r#"
        SELECT COALESCE(MAX(sequence), 0) + 1
        FROM events
        WHERE chain_id = $1
        "#,
        req.chain_id
    )
    .fetch_one(&mut *tx)
    .await?;

    let event_id = Uuid::new_v4();

    sqlx::query!(
        r#"
        INSERT INTO events (
            event_id,
            chain_id,
            parent_event_id,
            file_hash,
            idempotency_key,
            signature,
            sequence
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7)
        "#,
        event_id,
        req.chain_id,
        parent_event_id,
        req.file_hash,
        req.idempotency_key,
        "",
        sequence
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

    // Сначала делаем TSA stamp (синхронно)
    if let Some(root) = compute_chain_root(pool, req.chain_id).await {
        crate::tsa_worker::stamp_chain(pool, req.chain_id, &root, event_id).await;
    }

    // Потом получаем TSA из БД
    let tsa_record = sqlx::query!(
        r#"
        SELECT tsa_timestamp, tsa_serial, length(tsa_token) as token_bytes
        FROM tsa_tokens
        WHERE chain_id = $1 AND event_id = $2
        "#,
        req.chain_id,
        event_id
    )
    .fetch_optional(pool)
    .await?;

    let events = sqlx::query_as!(
        crate::db::EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        req.chain_id
    )
    .fetch_all(pool)
    .await?;

    let root = crate::merkle::MerkleTree::recompute_root_from_events(&events);
    let chain_head = event_id.to_string();
    let signature = signer.sign_root(&root, &chain_head);
    let public_key = signer.public_key_hex();

    let event_payloads: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "sequence": event.sequence,
                "event_id": event.event_id,
                "parent_event_id": event.parent_event_id,
                "file_hash": event.file_hash,
            })
        })
        .collect();

    Ok(json!({
        "event_id": event_id,
        "chain_id": req.chain_id,
        "head_event_id": event_id,
        "sequence": sequence,
        "cached": false,
        "proof": {
            "root": root,
            "chain_head": chain_head,
            "signature": signature,
            "public_key": public_key,
            "leaves_count": event_payloads.len()
        },
        "events": event_payloads,
        "tsa": tsa_record.map(|t| json!({
            "timestamp": t.tsa_timestamp,
            "serial": t.tsa_serial,
            "token_bytes": t.token_bytes,
        }))
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
