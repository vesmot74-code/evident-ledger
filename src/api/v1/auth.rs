use axum::{
    async_trait,
    body::Body,
    extract::{FromRequestParts, State},
    http::{request::Parts, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::auth::{resolve_authed_account, AuthedAccount};
use crate::state::AppState;

use super::errors::ApiError;

/// v1 API authentication — reuses legacy `X-API-KEY` → `AuthedAccount` resolution.
pub struct V1Auth(pub AuthedAccount);

#[async_trait]
impl FromRequestParts<AppState> for V1Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(auth) = parts.extensions.get::<AuthedAccount>() {
            return Ok(V1Auth(auth.clone()));
        }

        resolve_authed_account(&parts.headers, state)
            .await
            .map(V1Auth)
            .map_err(|_| ApiError::Unauthorized)
    }
}

/// Authenticates `/v1/*` requests once and stores `AuthedAccount` for downstream middleware/handlers.
pub async fn v1_auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match resolve_authed_account(request.headers(), &state).await {
        Ok(auth) => {
            request.extensions_mut().insert(auth);
            next.run(request).await
        }
        Err(_) => ApiError::Unauthorized.into_response(),
    }
}
