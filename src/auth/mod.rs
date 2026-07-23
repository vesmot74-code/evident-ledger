pub mod api_key;
pub mod desktop_token;
pub mod password;
pub mod session_store;
pub mod web_auth;
pub mod web_me;

pub use web_auth::router as web_auth_router;

use crate::service::desktop_tokens;
use crate::state::AppState;
use axum::{
    async_trait,
    body::Body,
    extract::{FromRequestParts, State},
    http::{header, request::Parts, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::Row;

#[derive(Clone)]
pub struct AuthedAccount {
    pub account_id: uuid::Uuid,
    pub key_hash: String,
}

pub enum AuthError {
    Missing,
    Invalid,
}

/// Resolve `Authorization: Bearer desktop_…` or `X-API-KEY` to an authenticated account.
pub async fn resolve_authed_account(
    headers: &axum::http::HeaderMap,
    state: &AppState,
) -> Result<AuthedAccount, AuthError> {
    if let Some(auth) = resolve_desktop_bearer(headers, state).await? {
        return Ok(auth);
    }

    let raw_key = headers
        .get("X-API-KEY")
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Missing)?;

    let key_hash = api_key::hash_api_key_for_lookup(raw_key);

    let row = sqlx::query(
        r#"
        SELECT account_id, key_hash
        FROM api_keys
        WHERE key_hash = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(&key_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| AuthError::Invalid)?;

    let row = row.ok_or(AuthError::Invalid)?;

    Ok(AuthedAccount {
        account_id: row.try_get("account_id").map_err(|_| AuthError::Invalid)?,
        key_hash: row.try_get("key_hash").map_err(|_| AuthError::Invalid)?,
    })
}

async fn resolve_desktop_bearer(
    headers: &axum::http::HeaderMap,
    state: &AppState,
) -> Result<Option<AuthedAccount>, AuthError> {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(None);
    };
    let Some(raw) = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };

    // Non-desktop Bearer schemes are ignored so API-key auth can still apply.
    let Some(token_hash) = desktop_token::hash_desktop_token_for_lookup(raw) else {
        return Ok(None);
    };

    let record = desktop_tokens::find_active_by_token_hash(&state.db, &token_hash)
        .await
        .map_err(|_| AuthError::Invalid)?
        .ok_or(AuthError::Invalid)?;

    let _ = desktop_tokens::touch_last_used(&state.db, record.id).await;

    Ok(Some(AuthedAccount {
        account_id: record.account_id,
        key_hash: record.token_hash,
    }))
}

/// Authenticates once and stores `AuthedAccount` for subscription middleware / handlers.
pub async fn api_key_auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match resolve_authed_account(request.headers(), &state).await {
        Ok(auth) => {
            request.extensions_mut().insert(auth);
            next.run(request).await
        }
        Err(err) => err.into_response(),
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let msg = match self {
            AuthError::Missing => "Missing credentials (X-API-KEY or Authorization Bearer)",
            AuthError::Invalid => "Invalid or revoked credentials",
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
        if let Some(auth) = parts.extensions.get::<AuthedAccount>() {
            return Ok(AuthedAccount {
                account_id: auth.account_id,
                key_hash: auth.key_hash.clone(),
            });
        }

        resolve_authed_account(&parts.headers, state).await
    }
}
