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

/// Resolve a valid session from the Cookie header, if present.
pub async fn optional_session_user(
    state: &AppState,
    cookie_header: Option<&str>,
) -> Option<SessionUser> {
    let cookie_header = cookie_header?;
    let token = parse_session_cookie(cookie_header)?;
    let account_id = resolve_session_account_id(&state.db, &token).await.ok()??;
    Some(SessionUser { account_id })
}

pub async fn session_auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let cookie_header = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());

    let Some(user) = optional_session_user(&state, cookie_header).await else {
        return ApiError::Unauthorized.into_response();
    };

    request.extensions_mut().insert(user);
    next.run(request).await
}

pub async fn session_ui_auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    use axum::response::Redirect;

    let cookie_header = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());

    let Some(user) = optional_session_user(&state, cookie_header).await else {
        return Redirect::to("/login").into_response();
    };

    request.extensions_mut().insert(user);
    next.run(request).await
}

#[async_trait]
impl FromRequestParts<AppState> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SessionUser>()
            .copied()
            .ok_or(ApiError::Unauthorized)
    }
}
