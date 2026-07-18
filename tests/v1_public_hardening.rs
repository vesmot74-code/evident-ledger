//! Stage 6.6 — public verification hardening tests.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode as HttpStatusCode};
use evident_ledger::api::public_verify::{public_router, verify_by_hash, verify_by_hash_with_lookup};
use evident_ledger::middleware::public_rate_limit::{
    public_rate_limit_middleware, PublicRateLimitMiddlewareState,
};
use evident_ledger::middleware::public_request::PublicRequestMetadata;
use evident_ledger::public_proof::{generate_public_id, PublicRegistryEntry};
use evident_ledger::public_verification_audit::{
    enable_test_capture, take_test_events, PublicVerificationOutcome,
    PublicVerificationRateLimitAction, PublicVerificationRequestType,
};
use evident_ledger::public_verify_validation::{validate_public_file_hash, validate_public_proof_id};
use evident_ledger::state::rate_limiter::{
    FixedWindowLimiter, PublicRateLimitState, RateLimitConfig,
};
use evident_ledger::state::AppState;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tower::util::ServiceExt;

fn canonical_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn test_rate_limits(verify_max: u32, window_secs: u64) -> PublicRateLimitState {
    PublicRateLimitState {
        verify: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: verify_max,
            window_secs,
            max_entries: 1_000,
        })),
        certificate: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: 20,
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

async fn status_for(app: axum::Router, uri: &str) -> HttpStatusCode {
    let mut svc = app.into_service();
    svc.oneshot(peer_request(uri))
        .await
        .expect("response")
        .status()
}

async fn body_for(app: axum::Router, uri: &str) -> Value {
    let mut svc = app.into_service();
    let resp = svc.oneshot(peer_request(uri)).await.expect("response");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

fn assert_invalid_request_envelope(body: &Value) {
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["message"], "Invalid request");
    assert!(body["error"]["request_id"].is_string());
    let message = body["error"]["message"].as_str().unwrap_or_default().to_lowercase();
    for forbidden in ["hex", "64", "length", "format", "expected"] {
        assert!(
            !message.contains(forbidden),
            "error message leaked validation hint: {forbidden}"
        );
    }
}

fn assert_audit_forbidden_fields(events: &[evident_ledger::public_verification_audit::PublicVerificationAuditEvent], probe_hash: &str) {
    for event in events {
        let serialized = serde_json::to_string(event).expect("audit json");
        assert!(!serialized.contains("file_hash"));
        assert!(!serialized.contains("public_proof_id"));
        assert!(!serialized.contains("\"exists\""));
        assert!(!serialized.contains("127.0.0.1"));
        assert!(!serialized.contains(probe_hash));
        assert!(
            !serialized.contains(&probe_hash.to_lowercase()),
            "audit must not contain probe hash"
        );
        assert!(event.timestamp.timestamp() > 0);
        assert!(event.request_id.len() >= 8);
    }
}

#[tokio::test]
async fn hash_format_validation_rejects_invalid_inputs() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(test_state_with_pool(pool.clone()), test_rate_limits(100, 60));

    for uri in [
        "/verify?file_hash=abc",
        &format!("/verify?file_hash={}", "a".repeat(63)),
        &format!("/verify?file_hash={}", "b".repeat(65)),
    ] {
        assert_eq!(status_for(app.clone(), uri).await, HttpStatusCode::BAD_REQUEST);
        assert_invalid_request_envelope(&body_for(app.clone(), uri).await);
    }

    let sql_injection = verify_by_hash(
        &pool,
        Some("'; DROP TABLE events;--".into()),
        None,
    )
    .await
    .expect("validation");
    assert_eq!(sql_injection.status(), HttpStatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(sql_injection.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_invalid_request_envelope(&serde_json::from_slice(&bytes).expect("json"));
}

#[tokio::test]
async fn hash_format_validation_accepts_uppercase_hex() {
    let hash = canonical_hash("uppercase-validation");
    let metadata = PublicRequestMetadata {
        client_ip_hash: Some("dd".repeat(64)),
        rate_limit_action: PublicVerificationRateLimitAction::Allowed,
    };
    let (resp, calls) = verify_by_hash_with_lookup(
        Some(hash.to_uppercase()),
        |_| Box::pin(async { Ok(None) }),
        Some(&metadata),
    )
    .await
    .expect("uppercase accepted");
    assert_eq!(calls, 1);
    assert_eq!(resp.status(), HttpStatusCode::OK);
}

#[tokio::test]
async fn public_proof_id_format_validation_rejects_invalid() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(test_state_with_pool(pool), test_rate_limits(100, 60));

    let invalid = status_for(app.clone(), "/verify/not-a-valid-id/certificate.pdf").await;
    assert_eq!(invalid, HttpStatusCode::BAD_REQUEST);
    assert_invalid_request_envelope(
        &body_for(app, "/verify/not-a-valid-id/certificate.pdf").await,
    );
}

#[test]
fn validation_unit_rules() {
    assert!(validate_public_file_hash(Some("abc".into())).is_err());
    assert!(validate_public_file_hash(Some("a".repeat(63))).is_err());
    assert!(validate_public_file_hash(Some("a".repeat(65))).is_err());
    assert!(validate_public_file_hash(Some("'; DROP TABLE events;--".into())).is_err());

    let lower = "c".repeat(64);
    assert_eq!(
        validate_public_file_hash(Some(lower.to_uppercase())).expect("upper"),
        lower
    );
    assert!(validate_public_file_hash(Some(lower.clone())).is_ok());

    assert!(!validate_public_proof_id("not-a-valid-id"));
    assert!(validate_public_proof_id(&generate_public_id()));
}

#[tokio::test]
async fn rate_limit_runs_before_format_validation() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(test_state_with_pool(pool), test_rate_limits(1, 60));
    let uri = "/verify?file_hash=not-a-valid-hash";
    assert_ne!(status_for(app.clone(), uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
    assert_eq!(status_for(app, uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn invalid_requests_count_toward_rate_limit() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(test_state_with_pool(pool), test_rate_limits(3, 60));
    let uri = "/verify?file_hash=bad";
    for _ in 0..3 {
        assert_eq!(status_for(app.clone(), uri).await, HttpStatusCode::BAD_REQUEST);
    }
    assert_eq!(status_for(app, uri).await, HttpStatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn valid_format_triggers_single_registry_lookup() {
    let hash = canonical_hash("registry-lookup-once");
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let metadata = PublicRequestMetadata {
        client_ip_hash: Some("bb".repeat(64)),
        rate_limit_action: PublicVerificationRateLimitAction::Allowed,
    };
    let (_resp, calls) = verify_by_hash_with_lookup(
        Some(hash),
        |_| Box::pin(async { Ok(None) }),
        Some(&metadata),
    )
    .await
    .expect("lookup");
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn audit_log_presence_and_outcomes() {
    enable_test_capture();
    let _ = take_test_events();

    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let app = public_router(test_state_with_pool(pool), test_rate_limits(1, 60));
    let uri = "/verify?file_hash=bad";

    assert_eq!(status_for(app.clone(), uri).await, HttpStatusCode::BAD_REQUEST);
    assert_eq!(status_for(app.clone(), uri).await, HttpStatusCode::TOO_MANY_REQUESTS);

    let probe = canonical_hash("audit-not-found");
    let metadata = PublicRequestMetadata {
        client_ip_hash: Some("ee".repeat(64)),
        rate_limit_action: PublicVerificationRateLimitAction::Allowed,
    };
    let _ = verify_by_hash_with_lookup(
        Some(probe.clone()),
        |_| Box::pin(async { Ok(None) }),
        Some(&metadata),
    )
    .await
    .expect("not found");

    let events = take_test_events();
    assert!(events.iter().any(|e| {
        e.request_type == PublicVerificationRequestType::Verify
            && e.outcome == PublicVerificationOutcome::NotFound
            && e.rate_limit_action == PublicVerificationRateLimitAction::Allowed
    }));
    assert!(events.iter().any(|e| {
        e.outcome == PublicVerificationOutcome::InvalidRequest
    }));
    assert!(events.iter().any(|e| {
        e.outcome == PublicVerificationOutcome::RateLimited
            && e.rate_limit_action == PublicVerificationRateLimitAction::Blocked
    }));

    assert_audit_forbidden_fields(&events, &probe);
}

#[tokio::test]
async fn audit_success_path_via_handler() {
    enable_test_capture();
    let _ = take_test_events();

    let metadata = PublicRequestMetadata {
        client_ip_hash: Some("cc".repeat(64)),
        rate_limit_action: PublicVerificationRateLimitAction::Allowed,
    };
    let hash = canonical_hash("audit-success");
    let entry = PublicRegistryEntry {
        public_proof_id: generate_public_id(),
        file_hash: hash.clone(),
        proof_status: "REGISTERED".into(),
        registered_at: chrono::Utc::now(),
        tsa_class: "basic".into(),
        integrity_state: "VALID".into(),
        enabled: true,
    };

    let _ = verify_by_hash_with_lookup(
        Some(hash.clone()),
        move |_| {
            let entry = entry.clone();
            Box::pin(async move { Ok(Some(entry)) })
        },
        Some(&metadata),
    )
    .await
    .expect("success");

    let events = take_test_events();
    assert!(events.iter().any(|e| e.outcome == PublicVerificationOutcome::Success));
    assert_audit_forbidden_fields(&events, &hash);
}

#[tokio::test]
async fn invalid_hash_never_reaches_database() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let response = verify_by_hash(&pool, Some("not-a-valid-hash".into()), None)
        .await
        .expect("validation");
    assert_eq!(response.status(), HttpStatusCode::BAD_REQUEST);
}
