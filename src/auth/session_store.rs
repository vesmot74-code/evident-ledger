//! Session persistence for web authentication (Stage 8.3.0).

use chrono::{Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub const SESSION_COOKIE_NAME: &str = "evident_session";
pub const SESSION_TTL_DAYS: i64 = 30;

pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_session_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub async fn cleanup_expired_sessions(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE expires_at < now()")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_sessions_for_account(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_session(pool: &PgPool, account_id: Uuid) -> Result<String, sqlx::Error> {
    cleanup_expired_sessions(pool).await?;
    delete_sessions_for_account(pool, account_id).await?;

    let token = generate_session_token();
    let token_hash = hash_session_token(&token);
    let expires_at = Utc::now() + Duration::days(SESSION_TTL_DAYS);

    sqlx::query(
        r#"
        INSERT INTO sessions (account_id, token_hash, expires_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(account_id)
    .bind(&token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(token)
}

pub async fn delete_session_by_token(pool: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    cleanup_expired_sessions(pool).await?;
    let token_hash = hash_session_token(token);
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn resolve_session_account_id(
    pool: &PgPool,
    token: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    cleanup_expired_sessions(pool).await?;
    let token_hash = hash_session_token(token);

    let row = sqlx::query(
        r#"
        SELECT id, account_id
        FROM sessions
        WHERE token_hash = $1 AND expires_at >= now()
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let session_id: Uuid = row.try_get("id")?;
    sqlx::query("UPDATE sessions SET last_used_at = now() WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;

    Ok(Some(row.try_get("account_id")?))
}

pub fn session_cookie_value(token: &str, secure: bool) -> String {
    let max_age = SESSION_TTL_DAYS * 24 * 60 * 60;
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE_NAME}={token}; HttpOnly; Path=/; Max-Age={max_age}; SameSite=Lax{secure_flag}"
    )
}

pub fn clear_session_cookie(secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE_NAME}=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax{secure_flag}")
}

pub fn parse_session_cookie(cookie_header: &str) -> Option<String> {
    cookie_header.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(&format!("{SESSION_COOKIE_NAME}="))
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    })
}
