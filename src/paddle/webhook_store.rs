//! Persistence for Paddle webhook idempotency and audit (Stage 8.2b).

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct WebhookEventRow {
    pub id: Uuid,
    pub paddle_event_id: String,
    pub payload_hash: String,
    pub status: String,
    pub event_occurred_at: DateTime<Utc>,
}

pub fn payload_hash(raw_body: &[u8]) -> String {
    hex::encode(Sha256::digest(raw_body))
}

pub async fn find_by_paddle_event_id(
    pool: &PgPool,
    paddle_event_id: &str,
) -> Result<Option<WebhookEventRow>, sqlx::Error> {
    sqlx::query_as::<_, WebhookEventRow>(
        r#"
        SELECT id, paddle_event_id, payload_hash, status, event_occurred_at
        FROM paddle_webhook_events
        WHERE paddle_event_id = $1
        "#,
    )
    .bind(paddle_event_id)
    .fetch_optional(pool)
    .await
}

pub async fn last_processed_occurred_at(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT MAX(event_occurred_at)
        FROM paddle_webhook_events
        WHERE account_id = $1 AND status = 'processed'
        "#,
    )
    .bind(account_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn insert_received(
    tx: &mut Transaction<'_, Postgres>,
    paddle_event_id: &str,
    event_type: &str,
    payload_hash: &str,
    account_id: Uuid,
    subscription_id: Option<&str>,
    event_occurred_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO paddle_webhook_events (
            paddle_event_id, event_type, payload_hash, account_id,
            subscription_id, event_occurred_at, status
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'received')
        RETURNING id
        "#,
    )
    .bind(paddle_event_id)
    .bind(event_type)
    .bind(payload_hash)
    .bind(account_id)
    .bind(subscription_id)
    .bind(event_occurred_at)
    .fetch_one(&mut **tx)
    .await
}

pub async fn mark_processing(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE paddle_webhook_events
        SET status = 'processing', processing_started_at = now()
        WHERE id = $1 AND status IN ('received', 'failed')
        "#,
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn mark_processed(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE paddle_webhook_events
        SET status = 'processed', processed_at = now(), error_message = NULL
        WHERE id = $1 AND status = 'processing'
        "#,
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_received_unlinked(
    tx: &mut Transaction<'_, Postgres>,
    paddle_event_id: &str,
    event_type: &str,
    payload_hash: &str,
    subscription_id: Option<&str>,
    event_occurred_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO paddle_webhook_events (
            paddle_event_id, event_type, payload_hash, account_id,
            subscription_id, event_occurred_at, status
        )
        VALUES ($1, $2, $3, NULL, $4, $5, 'received')
        RETURNING id
        "#,
    )
    .bind(paddle_event_id)
    .bind(event_type)
    .bind(payload_hash)
    .bind(subscription_id)
    .bind(event_occurred_at)
    .fetch_one(&mut **tx)
    .await
}

pub async fn mark_waiting_for_account_link(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE paddle_webhook_events
        SET status = 'waiting_for_account_link', processed_at = now(), error_message = NULL
        WHERE id = $1 AND status = 'processing'
        "#,
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_pending_link(
    tx: &mut Transaction<'_, Postgres>,
    paddle_customer_id: &str,
    paddle_email: &str,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO paddle_pending_links (paddle_customer_id, paddle_email)
        VALUES ($1, $2)
        RETURNING id
        "#,
    )
    .bind(paddle_customer_id)
    .bind(paddle_email)
    .fetch_one(&mut **tx)
    .await
}

pub async fn pending_link_exists(
    pool: &PgPool,
    paddle_customer_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM paddle_pending_links
            WHERE paddle_customer_id = $1
              AND resolved_at IS NULL
        )
        "#,
    )
    .bind(paddle_customer_id)
    .fetch_one(pool)
    .await
}

pub async fn mark_failed(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE paddle_webhook_events
        SET status = 'failed', error_message = $2, processed_at = now()
        WHERE id = $1 AND status = 'processing'
        "#,
    )
    .bind(id)
    .bind(message)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for WebhookEventRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            paddle_event_id: row.try_get("paddle_event_id")?,
            payload_hash: row.try_get("payload_hash")?,
            status: row.try_get("status")?,
            event_occurred_at: row.try_get("event_occurred_at")?,
        })
    }
}
