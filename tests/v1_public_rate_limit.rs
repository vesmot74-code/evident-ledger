//! Stage 6.5 — public verification rate limiting tests.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode as HttpStatusCode};
use evident_ledger::api::public_verify::public_router;
use evident_ledger::middleware::public_rate_limit::{
    public_rate_limit_middleware, PublicRateLimitMiddlewareState,
};
use evident_ledger::state::rate_limiter::{
    FixedWindowLimiter, PublicRateLimitState, RateLimitConfig,
};
use evident_ledger::state::AppState;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tower::util::ServiceExt;

fn canonical_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn test_rate_limits(verify_max: u32, cert_max: u32, window_secs: u64) -> PublicRateLimitState {
    PublicRateLimitState {
        verify: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: verify_max,
            window_secs,
            max_entries: 1_000,
        })),
        certificate: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: cert_max,
            window_secs,
            max_entries: 1_000,
        })),
        register: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: 10,
            window_secs,
            max_entries: 1_000,
        })),
        trust_proxy_headers: false,
        include_user_agent_in_key: false,
    }
}

fn test_state_with_pool(pool: sqlx::PgPool) -> AppState {
    AppState {
        db: pool,
        signer: Arc::new(
            evident_ledger::signing::ServerSigner::load_or_create("signing_key.bin"),
        ),
        config: evident_ledger::config::AppConfig::from_env(),
    }
}

fn peer_request(uri: &str) -> Request<Body> {
    let mut req = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            0,
        )));
    req
}

fn certificate_stub_app(rate_limits: &PublicRateLimitState) -> axum::Router {
    axum::Router::new()
        .route(
            "/verify/:public_proof_id/certificate.pdf",
            axum::routing::get(|| async { "pdf" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            PublicRateLimitMiddlewareState::certificate(rate_limits),
            public_rate_limit_middleware,
        ))
}

async fn status_for(app: axum::Router, uri: &str) -> HttpStatusCode {
    let mut svc = app.into_service();
    svc.oneshot(peer_request(uri))
        .await
        .expect("response")
        .status()
}

fn assert_rate_limited_envelope(body: &Value) {
    assert_eq!(body["error"]["code"], "rate_limited");
    assert!(body["error"]["request_id"].is_string());
    let serialized = body.to_string();
    assert!(!serialized.contains("file_hash"));
    assert!(!serialized.contains("public_proof_id"));
}

#[tokio::test]
async fn verify_endpoint_allows_first_100_then_blocks() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(
        test_state_with_pool(pool),
        test_rate_limits(100, 20, 60),
    );
    let uri = "/verify?file_hash=not-a-valid-hash";

    for i in 0..100 {
        let status = status_for(app.clone(), uri).await;
        assert_ne!(
            status,
            HttpStatusCode::TOO_MANY_REQUESTS,
            "request {i} should not be rate limited"
        );
    }
    assert_eq!(status_for(app, uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn verify_endpoint_429_has_retry_after_and_envelope() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(
        test_state_with_pool(pool),
        test_rate_limits(1, 20, 60),
    );
    let uri = "/verify?file_hash=not-a-valid-hash";
    let _ = status_for(app.clone(), uri).await;
    let mut svc = app.into_service();
    let resp = svc
        .oneshot(peer_request(uri))
        .await
        .expect("blocked response");
    assert_eq!(resp.status(), HttpStatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().get("retry-after").is_some());
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let parsed: Value = serde_json::from_slice(&body).expect("json");
    assert_rate_limited_envelope(&parsed);
}

#[tokio::test]
async fn certificate_endpoint_allows_first_20_then_blocks() {
    let limiter = Arc::new(FixedWindowLimiter::new(RateLimitConfig {
        max_requests: 20,
        window_secs: 60,
        max_entries: 100,
    }));
    let middleware_state = PublicRateLimitMiddlewareState {
        limiter,
        trust_proxy_headers: false,
        include_user_agent_in_key: false,
        request_type: evident_ledger::public_verification_audit::PublicVerificationRequestType::CertificatePdf,
        rate_limit_scope: None,
        rate_limit_message: "Too many requests. Please try again later.",
        audit_enabled: true,
    };
    let app = axum::Router::new()
        .route(
            "/verify/:public_proof_id/certificate.pdf",
            axum::routing::get(|| async { "pdf" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            middleware_state,
            public_rate_limit_middleware,
        ));

    let uri = "/verify/pv_test123/certificate.pdf";
    for i in 0..20 {
        let status = status_for(app.clone(), uri).await;
        assert_ne!(
            status,
            HttpStatusCode::TOO_MANY_REQUESTS,
            "request {i} should not be rate limited"
        );
    }
    assert_eq!(status_for(app, uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn verify_and_certificate_limits_are_independent() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let rate_limits = test_rate_limits(3, 20, 60);
    let app = public_router(test_state_with_pool(pool), rate_limits.clone());
    let verify_uri = "/verify?file_hash=not-a-valid-hash";
    for _ in 0..3 {
        assert_ne!(
            status_for(app.clone(), verify_uri).await,
            HttpStatusCode::TOO_MANY_REQUESTS
        );
    }
    assert_eq!(
        status_for(app, verify_uri).await,
        HttpStatusCode::TOO_MANY_REQUESTS
    );
    assert_ne!(
        status_for(
            certificate_stub_app(&rate_limits),
            "/verify/pv_independent/certificate.pdf",
        )
        .await,
        HttpStatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn rate_limit_isolates_different_peer_ips() {
    let limiter = Arc::new(FixedWindowLimiter::new(RateLimitConfig {
        max_requests: 2,
        window_secs: 60,
        max_entries: 100,
    }));
    let app = axum::Router::new()
        .route("/verify", axum::routing::get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            PublicRateLimitMiddlewareState {
                limiter,
                trust_proxy_headers: false,
                include_user_agent_in_key: false,
                request_type: evident_ledger::public_verification_audit::PublicVerificationRequestType::Verify,
                rate_limit_scope: None,
                rate_limit_message: "Too many requests. Please try again later.",
                audit_enabled: true,
            },
            public_rate_limit_middleware,
        ));

    async fn request_with_peer(app: axum::Router, ip: IpAddr) -> HttpStatusCode {
        let mut svc = app.into_service();
        let mut req = Request::builder()
            .uri("/verify")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(ip, 0)));
        svc.oneshot(req).await.expect("response").status()
    }

    let ip_a = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10));
    let ip_b = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 11));

    assert_eq!(request_with_peer(app.clone(), ip_a).await, HttpStatusCode::OK);
    assert_eq!(request_with_peer(app.clone(), ip_a).await, HttpStatusCode::OK);
    assert_eq!(
        request_with_peer(app.clone(), ip_a).await,
        HttpStatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(request_with_peer(app, ip_b).await, HttpStatusCode::OK);
}

#[tokio::test]
async fn rate_limit_is_hash_independent_for_same_ip() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(
        test_state_with_pool(pool),
        test_rate_limits(2, 20, 60),
    );

    for _ in 0..2 {
        let _ = status_for(
            app.clone(),
            "/verify?file_hash=not-a-valid-hash-x",
        )
        .await;
    }
    assert_eq!(
        status_for(app, "/verify?file_hash=not-a-valid-hash-y").await,
        HttpStatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn rate_limit_window_resets_after_elapsed_time() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(
        test_state_with_pool(pool),
        test_rate_limits(1, 20, 1),
    );
    let uri = "/verify?file_hash=not-a-valid-hash";
    assert_ne!(status_for(app.clone(), uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
    assert_eq!(status_for(app.clone(), uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_ne!(status_for(app, uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn rate_limit_blocks_before_registry_lookup() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(
        test_state_with_pool(pool),
        test_rate_limits(1, 20, 60),
    );
    let hash = canonical_hash("rate-limit-no-db");
    let uri = format!("/verify?file_hash={hash}");
    assert_ne!(
        status_for(app.clone(), "/verify?file_hash=not-a-valid-hash").await,
        HttpStatusCode::TOO_MANY_REQUESTS
    );
    let blocked = status_for(app, &uri).await;
    assert_eq!(blocked, HttpStatusCode::TOO_MANY_REQUESTS);
}
