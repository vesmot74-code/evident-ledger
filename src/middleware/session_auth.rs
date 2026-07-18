//! Session cookie authentication for Dashboard routes (Stage 8.3.1a).

use axum::{
    async_trait,
    body::Body,
    extract::{FromRequestParts, State},
    http::{header, request::Parts, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::api::v1::errors::ApiError;
use crate::auth::session_store::{parse_session_cookie, resolve_session_account_id};
use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
pub struct SessionUser {
    pub account_id: Uuid,
}

pub async fn session_auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let Some(cookie_header) = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
    else {
        return ApiError::Unauthorized.into_response();
    };

    let Some(token) = parse_session_cookie(cookie_header) else {
        return ApiError::Unauthorized.into_response();
    };

    let account_id = match resolve_session_account_id(&state.db, &token).await {
        Ok(Some(id)) => id,
        Ok(None) => return ApiError::Unauthorized.into_response(),
        Err(_) => return ApiError::Internal.into_response(),
    };

    request.extensions_mut().insert(SessionUser { account_id });
    next.run(request).await
}

#[async_trait]
impl FromRequestParts<AppState> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &AppState) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SessionUser>()
            .copied()
            .ok_or(ApiError::Unauthorized)
    }
}
