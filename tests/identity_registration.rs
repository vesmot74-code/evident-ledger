//! Stage 9.2 — identity challenge registration tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use evident_ledger::api::accounts;
use evident_ledger::service::accounts as account_service;
use evident_ledger::service::identity_keys::IdentityKeyRepository;
use evident_ledger::state::rate_limiter::{
    FixedWindowLimiter, PublicRateLimitState, RateLimitConfig,
};
use evident_ledger::state::AppState;
use rand::rngs::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("db");
    sqlx::migrate!().run(&pool).await.expect("migrate");
    pool
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn rate_limits() -> PublicRateLimitState {
    PublicRateLimitState {
        verify: Arc::new(FixedWindowLimiter::new(RateLimitConfig::verify())),
        certificate: Arc::new(FixedWindowLimiter::new(RateLimitConfig::certificate())),
        register: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: 100,
            window_secs: 60,
            max_entries: 1_000,
        })),
        trust_proxy_headers: false,
        include_user_agent_in_key: false,
    }
}

fn peer_request(method: &str, uri: &str, body: Option<Value>, api_key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = api_key {
        builder = builder.header("X-API-KEY", key);
    }
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let mut req = builder.body(body).expect("request");
    req.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 55)),
        0,
    )));
    req
}

async fn response_json(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let mut svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let parsed = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({ "raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, parsed)
}

async fn create_account_with_plan(pool: &sqlx::PgPool, plan_name: &str) -> Uuid {
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, (SELECT plan_id FROM tariff_plans WHERE name = $3), 'none')
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@identity-reg.test"))
    .bind(plan_name)
    .execute(pool)
    .await
    .expect("insert account");
    account_id
}

async fn create_api_key(pool: &sqlx::PgPool, account_id: Uuid) -> String {
    let (generated, _) = account_service::create_api_key(pool, account_id, "test")
        .await
        .expect("create api key");
    generated.full_key
}

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query("DELETE FROM identity_challenges WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM identity_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM api_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
}

async fn request_challenge(app: &axum::Router, api_key: &str) -> (StatusCode, Value) {
    response_json(
        app.clone(),
        peer_request(
            "POST",
            "/identity/keys/challenge",
            None,
            Some(api_key),
        ),
    )
    .await
}

fn sign_challenge(signing_key: &SigningKey, challenge_hex: &str) -> String {
    let raw = hex::decode(challenge_hex).expect("challenge hex");
    hex::encode(signing_key.sign(&raw).to_bytes())
}

async fn register_key(
    app: &axum::Router,
    api_key: &str,
    challenge_id: Uuid,
    challenge_hex: &str,
    signing_key: &SigningKey,
    signature_override: Option<&str>,
) -> (StatusCode, Value) {
    let public_key = hex::encode(signing_key.verifying_key().to_bytes());
    let signature = signature_override
        .map(str::to_string)
        .unwrap_or_else(|| sign_challenge(signing_key, challenge_hex));

    response_json(
        app.clone(),
        peer_request(
            "POST",
            "/identity/keys/register",
            Some(json!({
                "challenge_id": challenge_id,
                "public_key": public_key,
                "signature": signature,
                "label": "test-key"
            })),
            Some(api_key),
        ),
    )
    .await
}

#[tokio::test]
async fn challenge_with_valid_api_key_returns_challenge() {
    let pool = test_pool().await;
    let account_id = create_account_with_plan(&pool, "identity").await;
    let api_key = create_api_key(&pool, account_id).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (status, body) = request_challenge(&app, &api_key).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["challenge_id"].is_string());
    assert_eq!(body["challenge"].as_str().unwrap().len(), 64);
    assert!(body["expires_at"].is_string());

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn challenge_without_api_key_returns_unauthorized() {
    let pool = test_pool().await;
    let app = accounts::router(test_state(pool), rate_limits());

    let (status, _) = response_json(
        app,
        peer_request("POST", "/identity/keys/challenge", None, None),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn challenge_without_identity_entitlement_returns_forbidden() {
    let pool = test_pool().await;
    let account_id = create_account_with_plan(&pool, "free").await;
    let api_key = create_api_key(&pool, account_id).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (status, body) = request_challenge(&app, &api_key).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "entitlement_missing");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn register_with_valid_signature_creates_key() {
    let pool = test_pool().await;
    let account_id = create_account_with_plan(&pool, "identity").await;
    let api_key = create_api_key(&pool, account_id).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (_, challenge_body) = request_challenge(&app, &api_key).await;
    let challenge_id = Uuid::parse_str(challenge_body["challenge_id"].as_str().unwrap()).unwrap();
    let challenge_hex = challenge_body["challenge"].as_str().unwrap();

    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let expected_fingerprint = hex::encode(Sha256::digest(
        &hex::decode(&public_key_hex).unwrap(),
    ));

    let (status, body) = register_key(
        &app,
        &api_key,
        challenge_id,
        challenge_hex,
        &signing_key,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["fingerprint"], expected_fingerprint);
    assert!(body["created_at"].is_string());

    let key_id = Uuid::parse_str(body["key_id"].as_str().unwrap()).unwrap();
    let stored = IdentityKeyRepository::find_by_id(&pool, key_id)
        .await
        .expect("db")
        .expect("key stored");
    assert_eq!(stored.account_id, account_id);
    assert_eq!(stored.public_key, public_key_hex);

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn register_with_invalid_signature_returns_unauthorized() {
    let pool = test_pool().await;
    let account_id = create_account_with_plan(&pool, "identity").await;
    let api_key = create_api_key(&pool, account_id).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (_, challenge_body) = request_challenge(&app, &api_key).await;
    let challenge_id = Uuid::parse_str(challenge_body["challenge_id"].as_str().unwrap()).unwrap();
    let challenge_hex = challenge_body["challenge"].as_str().unwrap();
    let signing_key = SigningKey::generate(&mut OsRng);

    let (status, body) = register_key(
        &app,
        &api_key,
        challenge_id,
        challenge_hex,
        &signing_key,
        Some("00".repeat(64).as_str()),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_signature");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn register_with_expired_challenge_returns_gone() {
    let pool = test_pool().await;
    let account_id = create_account_with_plan(&pool, "identity").await;
    let api_key = create_api_key(&pool, account_id).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (_, challenge_body) = request_challenge(&app, &api_key).await;
    let challenge_id = Uuid::parse_str(challenge_body["challenge_id"].as_str().unwrap()).unwrap();
    let challenge_hex = challenge_body["challenge"].as_str().unwrap();

    sqlx::query(
        "UPDATE identity_challenges SET expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(challenge_id)
    .execute(&pool)
    .await
    .expect("expire challenge");

    let signing_key = SigningKey::generate(&mut OsRng);
    let (status, body) = register_key(
        &app,
        &api_key,
        challenge_id,
        challenge_hex,
        &signing_key,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["error"]["code"], "challenge_expired");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn register_with_used_challenge_returns_conflict() {
    let pool = test_pool().await;
    let account_id = create_account_with_plan(&pool, "identity").await;
    let api_key = create_api_key(&pool, account_id).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (_, challenge_body) = request_challenge(&app, &api_key).await;
    let challenge_id = Uuid::parse_str(challenge_body["challenge_id"].as_str().unwrap()).unwrap();
    let challenge_hex = challenge_body["challenge"].as_str().unwrap();
    let signing_key = SigningKey::generate(&mut OsRng);

    let (first_status, _) = register_key(
        &app,
        &api_key,
        challenge_id,
        challenge_hex,
        &signing_key,
        None,
    )
    .await;
    assert_eq!(first_status, StatusCode::OK);

    let other_key = SigningKey::generate(&mut OsRng);
    let (status, body) = register_key(
        &app,
        &api_key,
        challenge_id,
        challenge_hex,
        &other_key,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "challenge_already_used");

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn register_with_foreign_challenge_returns_not_found() {
    let pool = test_pool().await;
    let owner = create_account_with_plan(&pool, "identity").await;
    let other = create_account_with_plan(&pool, "identity").await;
    let owner_key = create_api_key(&pool, owner).await;
    let other_key = create_api_key(&pool, other).await;
    let app = accounts::router(test_state(pool.clone()), rate_limits());

    let (_, challenge_body) = request_challenge(&app, &owner_key).await;
    let challenge_id = Uuid::parse_str(challenge_body["challenge_id"].as_str().unwrap()).unwrap();
    let challenge_hex = challenge_body["challenge"].as_str().unwrap();
    let signing_key = SigningKey::generate(&mut OsRng);

    let (status, body) = register_key(
        &app,
        &other_key,
        challenge_id,
        challenge_hex,
        &signing_key,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "challenge_not_found");

    cleanup_account(&pool, owner).await;
    cleanup_account(&pool, other).await;
}
