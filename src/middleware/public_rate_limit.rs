//! Axum middleware for public verification rate limiting (Stage 6.5).
//!
//! Rate limiting is IP-based and per-instance (in-memory).
//! It is a mitigating control against casual abuse and scraping,
//! not a hard guarantee against distributed or botnet-based probing.
//!
//! `X-Forwarded-For` / `X-Real-IP` are ignored unless `TRUST_PROXY_HEADERS=true`.

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use uuid::Uuid;

use crate::state::rate_limiter::{
    rate_limit_client_key, FixedWindowLimiter, PublicRateLimitState, RateLimitDecision,
};

#[derive(Clone)]
pub struct PublicRateLimitMiddlewareState {
    pub limiter: Arc<FixedWindowLimiter>,
    pub trust_proxy_headers: bool,
    pub include_user_agent_in_key: bool,
}

impl PublicRateLimitMiddlewareState {
    pub fn verify(state: &PublicRateLimitState) -> Self {
        Self {
            limiter: state.verify.clone(),
            trust_proxy_headers: state.trust_proxy_headers,
            include_user_agent_in_key: state.include_user_agent_in_key,
        }
    }

    pub fn certificate(state: &PublicRateLimitState) -> Self {
        Self {
            limiter: state.certificate.clone(),
            trust_proxy_headers: state.trust_proxy_headers,
            include_user_agent_in_key: state.include_user_agent_in_key,
        }
    }
}

#[derive(Debug, Serialize)]
struct RateLimitErrorEnvelope {
    error: RateLimitErrorBody,
}

#[derive(Debug, Serialize)]
struct RateLimitErrorBody {
    code: String,
    message: String,
    request_id: String,
}

pub fn rate_limited_response(decision: RateLimitDecision) -> Response {
    let request_id = Uuid::new_v4().to_string();
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, decision.retry_after_secs.to_string())],
        Json(RateLimitErrorEnvelope {
            error: RateLimitErrorBody {
                code: "rate_limited".to_string(),
                message: "Too many requests. Please try again later.".to_string(),
                request_id,
            },
        }),
    )
        .into_response()
}

fn apply_rate_limit_headers(response: &mut Response, limit: u32, decision: RateLimitDecision) {
    let headers = response.headers_mut();
    if let Ok(value) = limit.to_string().parse() {
        headers.insert("X-RateLimit-Limit", value);
    }
    if let Ok(value) = decision.remaining.to_string().parse() {
        headers.insert("X-RateLimit-Remaining", value);
    }
    if let Ok(value) = decision.reset_unix.to_string().parse() {
        headers.insert("X-RateLimit-Reset", value);
    }
}

pub fn client_ip_from_request(
    request: &Request<Body>,
    peer: SocketAddr,
    trust_proxy_headers: bool,
) -> IpAddr {
    if trust_proxy_headers {
        if let Some(ip) = forwarded_client_ip(request) {
            return ip;
        }
    }
    peer.ip()
}

fn forwarded_client_ip(request: &Request<Body>) -> Option<IpAddr> {
    if let Some(value) = request
        .headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = value.split(',').next() {
            if let Ok(ip) = first.trim().parse() {
                return Some(ip);
            }
        }
    }
    request
        .headers()
        .get("X-Real-IP")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

pub async fn public_rate_limit_middleware(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<PublicRateLimitMiddlewareState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let user_agent = if state.include_user_agent_in_key {
        request
            .headers()
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
    } else {
        None
    };
    let ip = client_ip_from_request(&request, peer, state.trust_proxy_headers);
    let client_key = rate_limit_client_key(ip, user_agent);
    let decision = state.limiter.check(client_key, std::time::Instant::now());

    if !decision.allowed {
        return rate_limited_response(decision);
    }

    let limit = state.limiter.config().max_requests;
    let mut response = next.run(request).await;
    apply_rate_limit_headers(&mut response, limit, decision);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn ignores_forwarded_headers_by_default() {
        let mut req = Request::builder()
            .header("X-Forwarded-For", "203.0.113.50")
            .body(Body::empty())
            .unwrap();
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let ip = client_ip_from_request(&req, peer, false);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        let ip = client_ip_from_request(&req, peer, true);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)));
    }

    #[test]
    fn rate_limited_response_has_envelope() {
        let response = rate_limited_response(RateLimitDecision {
            allowed: false,
            retry_after_secs: 42,
            remaining: 0,
            reset_unix: 1,
        });
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
