//! Login rate limiting for POST /auth/login (Stage 8.3.0).

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{header, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;

use crate::api::v1::errors::ApiError;
use crate::middleware::public_rate_limit::client_ip_from_request;
use crate::state::rate_limiter::{rate_limit_scoped_client_key, LoginRateLimitState};

pub async fn login_rate_limit_middleware(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<LoginRateLimitState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = client_ip_from_request(&request, peer, state.trust_proxy_headers);
    let client_key = rate_limit_scoped_client_key(ip, None, Some("login"));
    let decision = state.login.check(client_key, std::time::Instant::now());

    if !decision.allowed {
        return ApiError::RateLimited.into_response();
    }

    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if let Ok(value) = state.login.config().max_requests.to_string().parse() {
        headers.insert("X-RateLimit-Limit", value);
    }
    if let Ok(value) = decision.remaining.to_string().parse() {
        headers.insert("X-RateLimit-Remaining", value);
    }
    if let Ok(value) = decision.reset_unix.to_string().parse() {
        headers.insert("X-RateLimit-Reset", value);
    }
    response
}
