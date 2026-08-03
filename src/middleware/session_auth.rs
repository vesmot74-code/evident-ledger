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

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub async fn session_ui_auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    use axum::extract::OriginalUri;
    use axum::response::Redirect;

    let cookie_header = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());

    let Some(user) = optional_session_user(&state, cookie_header).await else {
        // Prefer OriginalUri: nested /dashboard routers strip the nest prefix from
        // request.uri(), which would lose /dashboard in the post-login return path.
        let uri = request
            .extensions()
            .get::<OriginalUri>()
            .map(|OriginalUri(u)| u)
            .unwrap_or_else(|| request.uri());
        let original = uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/dashboard/ui");
        let target = format!("/login?next={}", percent_encode(original));
        return Redirect::to(&target).into_response();
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
