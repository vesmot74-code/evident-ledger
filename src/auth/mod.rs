pub mod api_key;

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::state::AppState;

pub struct AuthedAccount {
    pub account_id: Uuid,
    pub key_hash: String,
}

pub enum AuthError {
    Missing,
    Invalid,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let msg = match self {
            AuthError::Missing => "Missing X-API-KEY header",
            AuthError::Invalid => "Invalid or revoked API key",
        };
        (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg }))).into_response()
    }
}

#[async_trait]
impl FromRequestParts<AppState> for AuthedAccount {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let raw_key = parts
            .headers
            .get("X-API-KEY")
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthError::Missing)?;

        let key_hash = api_key::hash_api_key_for_lookup(raw_key);

        let row = sqlx::query!(
            r#"
            SELECT account_id, key_hash
            FROM api_keys
            WHERE key_hash = $1 AND revoked_at IS NULL
            "#,
            key_hash
        )
        .fetch_optional(&state.db)
        .await
        .map_err(|_| AuthError::Invalid)?;

        let row = row.ok_or(AuthError::Invalid)?;

        Ok(AuthedAccount {
            account_id: row.account_id,
            key_hash: row.key_hash,
        })
    }
}
