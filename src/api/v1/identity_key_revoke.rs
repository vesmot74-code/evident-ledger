//! POST /v1/identity/keys/{id}/revoke — transactional key revocation (Stage 9.6).

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::models::identity_key::IdentityKey;
use crate::service::identity_audit::{IdentityAuditError, IdentityAuditService};
use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;

#[derive(Debug, Serialize)]
pub struct RevokeKeyResponse {
    pub key_id: Uuid,
    pub status: String,
    pub revoked_at: DateTime<Utc>,
}

pub async fn revoke_key_handler(
    State(state): State<AppState>,
    auth: V1Auth,
    Path(key_id): Path<Uuid>,
) -> Result<Json<RevokeKeyResponse>, ApiError> {
    let response = revoke_identity_key(&state.db, auth.0.account_id, key_id).await?;
    Ok(Json(response))
}

pub async fn revoke_identity_key(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    key_id: Uuid,
) -> Result<RevokeKeyResponse, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;

    let key = sqlx::query_as::<_, IdentityKey>(
        r#"
        SELECT
            id,
            account_id,
            public_key,
            fingerprint,
            label,
            created_at,
            verified_at,
            revoked_at
        FROM identity_keys
        WHERE id = $1
        FOR UPDATE
        "#,
    )
    .bind(key_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    let key = match key {
        None => return Err(ApiError::IdentityKeyNotFound),
        Some(k) if k.account_id != account_id => return Err(ApiError::IdentityKeyNotFound),
        Some(k) if k.revoked_at.is_some() => return Err(ApiError::IdentityKeyAlreadyRevoked),
        Some(k) => k,
    };

    let revoked_at: DateTime<Utc> = sqlx::query_scalar(
        r#"
        UPDATE identity_keys
        SET revoked_at = now()
        WHERE id = $1
        RETURNING revoked_at
        "#,
    )
    .bind(key.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    IdentityAuditService::append(&mut *tx, key.id, "account", account_id, "revoked", None)
        .await
        .map_err(map_audit_error)?;

    tx.commit().await.map_err(|_| ApiError::Internal)?;

    Ok(RevokeKeyResponse {
        key_id: key.id,
        status: "revoked".to_string(),
        revoked_at,
    })
}

fn map_audit_error(err: IdentityAuditError) -> ApiError {
    match err {
        IdentityAuditError::InvalidActorType => ApiError::Internal,
        IdentityAuditError::Database(_) => ApiError::Internal,
    }
}
