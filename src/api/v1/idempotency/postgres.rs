use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::model::{AccountId, IdempotencyRecord};

#[derive(Debug, sqlx::FromRow)]
struct IdempotencyRecordRow {
    id: Uuid,
    account_id: Uuid,
    idempotency_key: String,
    request_hash: String,
    response_json: Value,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl From<IdempotencyRecordRow> for IdempotencyRecord {
    fn from(row: IdempotencyRecordRow) -> Self {
        Self {
            id: row.id,
            account_id: row.account_id,
            idempotency_key: row.idempotency_key,
            request_hash: row.request_hash,
            response_json: row.response_json,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

pub async fn find_active_in_tx(
    conn: &mut PgConnection,
    account_id: AccountId,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    let row = sqlx::query_as::<_, IdempotencyRecordRow>(
        r#"
        SELECT
            id,
            account_id,
            idempotency_key,
            request_hash,
            response_json,
            created_at,
            expires_at
        FROM idempotency_records
        WHERE account_id = $1
          AND idempotency_key = $2
          AND expires_at > now()
        FOR UPDATE
        "#,
    )
    .bind(account_id)
    .bind(idempotency_key)
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn insert_in_tx(
    conn: &mut PgConnection,
    record: &IdempotencyRecord,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO idempotency_records (
            id,
            account_id,
            idempotency_key,
            request_hash,
            response_json,
            created_at,
            expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(record.id)
    .bind(record.account_id)
    .bind(&record.idempotency_key)
    .bind(&record.request_hash)
    .bind(&record.response_json)
    .bind(record.created_at)
    .bind(record.expires_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub struct PostgresIdempotencyRepository {
    pool: PgPool,
}

impl PostgresIdempotencyRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
