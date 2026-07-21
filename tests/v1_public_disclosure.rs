//! Stage 6.4 — public disclosure boundary tests.

mod common;

use axum::Router;
use evident_ledger::api::public_verify::verify_by_hash;
use evident_ledger::state::rate_limiter::{
    FixedWindowLimiter, PublicRateLimitState, RateLimitConfig,
};
use evident_ledger::state::AppState;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashSet;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use uuid::Uuid;

fn canonical_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

async fn test_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let database_url = common::test_database_url();
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("db")
}

async fn cleanup(pool: &sqlx::PgPool, file_hash: &str) {
    let _ = sqlx::query("DELETE FROM public_proof_materialization WHERE file_hash = $1")
        .bind(file_hash)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM public_proof_registry WHERE file_hash = $1")
        .bind(file_hash)
        .execute(pool)
        .await;
}

fn public_app(state: AppState, rate_limits: PublicRateLimitState) -> Router {
    evident_ledger::api::public_verify::public_router(state, rate_limits)
}

fn generous_rate_limits() -> PublicRateLimitState {
    PublicRateLimitState {
        verify: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: 100,
            window_secs: 60,
            max_entries: 1_000,
        })),
        certificate: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: 20,
            window_secs: 60,
            max_entries: 1_000,
        })),
        register: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: 10,
            window_secs: 60,
            max_entries: 1_000,
        })),
        trust_proxy_headers: false,
        include_user_agent_in_key: false,
    }
}

async fn spawn_server(app: Router) -> u16 {
    let listener = TcpListener::bind("0.0.0.0:0").await.expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    port
}

fn assert_no_forbidden_public_fields(body: &Value) {
    for key in [
        "chain_id",
        "event_id",
        "merkle_root",
        "head_event_id",
        "match_count",
        "matches",
        "chains",
        "account_id",
    ] {
        assert!(body.get(key).is_none(), "forbidden key leaked: {key}");
    }
}

#[tokio::test]
async fn verify_hash_endpoint_is_deprecated() {
    use evident_ledger::api::verify::router;

    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let state = common::test_app_state(pool);
    let app = Router::new().nest("/verify", router(state));
    let port = spawn_server(app).await;
    let url = format!("http://127.0.0.1:{port}/verify/hash");
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&json!({
            "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::GONE);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["code"], "endpoint_deprecated");
    assert!(body["error"]["request_id"].is_string());
}

#[tokio::test]
async fn legacy_hash_attestation_pdf_returns_410() {
    use evident_ledger::api::verify::router;

    let signer = Arc::new(evident_ledger::signing::ServerSigner::load_or_create(
        "signing_key.bin",
    ));
    let config = {
        common::setup_test_env();
        evident_ledger::config::AppConfig::from_env()
    };
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy");
    let state = common::test_app_state(pool);
    let app = Router::new().nest("/verify", router(state));
    let port = spawn_server(app).await;
    let hash = canonical_hash("legacy-410");
    let url = format!("http://127.0.0.1:{port}/verify/hash/{hash}/attestation.pdf");
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::GONE);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["code"], "endpoint_deprecated");
    assert!(body["error"]["request_id"].is_string());
}

#[tokio::test]
async fn public_verify_returns_existence_only_fields() {
    let pool = test_pool().await;
    let file_hash = canonical_hash("public-disclosure-exists");
    cleanup(&pool, &file_hash).await;

    evident_ledger::public_proof::on_proof_anchored(&pool, Uuid::new_v4(), &file_hash, "legal")
        .await
        .expect("anchor");

    let signer = Arc::new(evident_ledger::signing::ServerSigner::load_or_create(
        "signing_key.bin",
    ));
    let state = common::test_app_state(pool.clone());
    let port = spawn_server(public_app(state, generous_rate_limits())).await;

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/verify?file_hash={file_hash}");
    let resp = client.get(&url).send().await.expect("request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["exists"], true);
    assert!(body["public_proof_id"]
        .as_str()
        .is_some_and(|id| id.starts_with("pv_")));
    assert!(body["timestamp"].is_string());
    assert_eq!(body["tsa_class"], "legal");
    assert_eq!(body["integrity"], "VALID");
    assert_no_forbidden_public_fields(&body);

    let missing = canonical_hash("public-disclosure-missing");
    let missing_url = format!("http://127.0.0.1:{port}/verify?file_hash={missing}");
    let missing_resp = client.get(&missing_url).send().await.expect("request");
    let missing_body: Value = missing_resp.json().await.expect("json");
    assert_eq!(missing_body["exists"], false);
    assert!(missing_body["public_proof_id"].is_null());
    assert_no_forbidden_public_fields(&missing_body);

    cleanup(&pool, &file_hash).await;
}

#[tokio::test]
async fn public_certificate_pdf_has_no_private_fields() {
    let pool = test_pool().await;
    let file_hash = canonical_hash("public-disclosure-pdf");
    cleanup(&pool, &file_hash).await;

    evident_ledger::public_proof::on_proof_anchored(&pool, Uuid::new_v4(), &file_hash, "legal")
        .await
        .expect("anchor");

    let public_proof_id: String = sqlx::query_scalar(
        "SELECT public_proof_id FROM public_proof_registry WHERE file_hash = $1",
    )
    .bind(&file_hash)
    .fetch_one(&pool)
    .await
    .expect("public_proof_id");

    let signer = Arc::new(evident_ledger::signing::ServerSigner::load_or_create(
        "signing_key.bin",
    ));
    let state = common::test_app_state(pool.clone());
    let port = spawn_server(public_app(state, generous_rate_limits())).await;

    let client = reqwest::Client::new();
    let pdf_url = format!("http://127.0.0.1:{port}/verify/{public_proof_id}/certificate.pdf");
    let resp = client.get(&pdf_url).send().await.expect("pdf");
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/pdf")
    );
    let bytes = resp.bytes().await.expect("bytes");
    assert!(bytes.starts_with(b"%PDF"));
    let text = String::from_utf8_lossy(&bytes);
    for forbidden in [
        "chain_id",
        "event_id",
        "merkle_root",
        "head_event_id",
        "Matches Found",
        "Chain ID:",
        "Event ID:",
        "Global Evidence Resolution Certificate",
    ] {
        assert!(
            !text.contains(forbidden),
            "PDF leaked forbidden field: {forbidden}"
        );
    }

    let hash_pdf_url = format!("http://127.0.0.1:{port}/verify/{file_hash}/certificate.pdf");
    let hash_resp = client.get(&hash_pdf_url).send().await.expect("hash pdf");
    assert_eq!(hash_resp.status(), StatusCode::BAD_REQUEST);
    let hash_body: Value = hash_resp.json().await.expect("json");
    assert_eq!(hash_body["error"]["code"], "invalid_request");

    cleanup(&pool, &file_hash).await;
}

#[tokio::test]
async fn cross_account_same_hash_reveals_no_multi_tenant_metadata() {
    let pool = test_pool().await;
    let file_hash = canonical_hash("public-disclosure-cross-account");
    cleanup(&pool, &file_hash).await;

    evident_ledger::public_proof::on_proof_anchored(&pool, Uuid::new_v4(), &file_hash, "basic")
        .await
        .expect("account A equivalent");
    evident_ledger::public_proof::on_proof_anchored(&pool, Uuid::new_v4(), &file_hash, "legal")
        .await
        .expect("account B equivalent");

    let signer = Arc::new(evident_ledger::signing::ServerSigner::load_or_create(
        "signing_key.bin",
    ));
    let state = common::test_app_state(pool.clone());
    let port = spawn_server(public_app(state, generous_rate_limits())).await;

    let resp = reqwest::Client::new()
        .get(format!(
            "http://127.0.0.1:{port}/verify?file_hash={file_hash}"
        ))
        .send()
        .await
        .expect("request");
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["exists"], true);
    assert_no_forbidden_public_fields(&body);
    let serialized = body.to_string();
    assert!(!serialized.contains("match"));
    assert!(!serialized.contains("chain"));
    assert!(!serialized.contains("event"));
    assert!(!serialized.contains("account"));

    cleanup(&pool, &file_hash).await;
}

#[tokio::test]
async fn public_proof_registry_schema_has_no_private_references() {
    let pool = test_pool().await;
    let cols: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT column_name
        FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'public_proof_registry'
        ORDER BY column_name
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("columns");

    let allowed: HashSet<&str> = [
        "public_proof_id",
        "file_hash",
        "proof_status",
        "registered_at",
        "tsa_class",
        "integrity_state",
        "enabled",
    ]
    .into_iter()
    .collect();
    let forbidden = [
        "chain_id",
        "event_id",
        "account_id",
        "owner_id",
        "proof_id",
        "id",
    ];

    for col in &cols {
        assert!(
            allowed.contains(col.as_str()),
            "unexpected column on public_proof_registry: {col}"
        );
        assert!(
            !forbidden.contains(&col.as_str()),
            "forbidden column on public_proof_registry: {col}"
        );
    }
}

#[tokio::test]
async fn invalid_hash_skips_database() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy pool");
    let response = verify_by_hash(&pool, Some("not-a-valid-hash".into()), None)
        .await
        .expect("pre-db validation");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
