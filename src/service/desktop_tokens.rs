//! Desktop token persistence (Stage 13.4).

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::desktop_token::{self, GeneratedDesktopToken};

pub const DEFAULT_TTL_DAYS: i64 = 30;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DesktopTokenRecord {
    pub id: Uuid,
    pub account_id: Uuid,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct CreatedDesktopToken {
    pub id: Uuid,
    pub plaintext: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_desktop_token(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<CreatedDesktopToken, sqlx::Error> {
    let generated: GeneratedDesktopToken = desktop_token::generate_desktop_token();
    let id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::days(DEFAULT_TTL_DAYS);

    sqlx::query(
        r#"
        INSERT INTO desktop_tokens (id, account_id, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(id)
    .bind(account_id)
    .bind(&generated.token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(CreatedDesktopToken {
        id,
        plaintext: generated.plaintext,
        expires_at,
    })
}

pub async fn find_active_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<DesktopTokenRecord>, sqlx::Error> {
    sqlx::query_as::<_, DesktopTokenRecord>(
        r#"
        SELECT id, account_id, token_hash, created_at, expires_at, revoked_at, last_used_at
        FROM desktop_tokens
        WHERE token_hash = $1
          AND revoked_at IS NULL
          AND expires_at > now()
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub async fn touch_last_used(pool: &PgPool, token_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE desktop_tokens
        SET last_used_at = now()
        WHERE id = $1
        "#,
    )
    .bind(token_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn revoke_desktop_token(
    pool: &PgPool,
    account_id: Uuid,
    token_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE desktop_tokens
        SET revoked_at = now()
        WHERE id = $1
          AND account_id = $2
          AND revoked_at IS NULL
        "#,
    )
    .bind(token_id)
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Test/ops helper: revoke by hash (no account check).
pub async fn revoke_by_token_hash(pool: &PgPool, token_hash: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE desktop_tokens
        SET revoked_at = now()
        WHERE token_hash = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}
