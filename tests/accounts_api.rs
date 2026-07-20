//! Stage 8.1 — self-service registration and API key management tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use evident_ledger::api::accounts;
use evident_ledger::auth::api_key;
use evident_ledger::state::rate_limiter::{
    FixedWindowLimiter, PublicRateLimitState, RateLimitConfig,
};
use evident_ledger::state::AppState;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("db");
    sqlx::migrate!().run(&pool).await.expect("migrate");
    pool
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn rate_limits(register_max: u32) -> PublicRateLimitState {
    PublicRateLimitState {
        verify: Arc::new(FixedWindowLimiter::new(RateLimitConfig::verify())),
        certificate: Arc::new(FixedWindowLimiter::new(RateLimitConfig::certificate())),
        register: Arc::new(FixedWindowLimiter::new(RateLimitConfig {
            max_requests: register_max,
            window_secs: 60,
            max_entries: 1_000,
        })),
        trust_proxy_headers: false,
        include_user_agent_in_key: false,
    }
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    api_key: Option<&str>,
) -> Request<Body> {
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
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 50)),
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

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query("DELETE FROM api_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
}

async fn cleanup_email(pool: &sqlx::PgPool, email: &str) {
    if let Ok(Some(account_id)) =
        sqlx::query_scalar::<_, Uuid>("SELECT account_id FROM accounts WHERE email = $1")
            .bind(email)
            .fetch_optional(pool)
            .await
    {
        cleanup_account(pool, account_id).await;
    }
}

fn unique_email(label: &str) -> String {
    format!("{label}-{}@example.com", Uuid::new_v4())
}

async fn register_account(app: &axum::Router, email: &str) -> (StatusCode, Value) {
    response_json(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "company_name": "Test Co" })),
            None,
        ),
    )
    .await
}

#[tokio::test]
async fn register_valid_email_returns_account_and_api_key() {
    let pool = test_pool().await;
    let email = unique_email("register-valid");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (status, body) = register_account(&app, &email).await;

    assert_eq!(status, StatusCode::CREATED);
    assert!(body["account_id"].is_string());
    assert!(body["api_key"]
        .as_str()
        .unwrap()
        .starts_with(api_key::API_KEY_PREFIX));
    assert_eq!(body["plan_name"], "free");
    assert!(body["tariff_plan_id"].is_string());
    assert!(body["created_at"].is_string());

    let account_id = Uuid::parse_str(body["account_id"].as_str().unwrap()).unwrap();
    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn register_duplicate_email_returns_conflict() {
    let pool = test_pool().await;
    let email = unique_email("register-dup");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (first, _) = register_account(&app, &email).await;
    assert_eq!(first, StatusCode::CREATED);

    let (second, body) = register_account(&app, &email).await;
    assert_eq!(second, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "conflict");

    if let Ok(account_id) =
        sqlx::query_scalar::<_, Uuid>("SELECT account_id FROM accounts WHERE email = $1")
            .bind(&email)
            .fetch_one(&pool)
            .await
    {
        cleanup_account(&pool, account_id).await;
    }
}

#[tokio::test]
async fn register_invalid_email_returns_bad_request() {
    let pool = test_pool().await;
    let app = accounts::router(test_state(pool), rate_limits(100));

    let (status, body) = response_json(
        app,
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": "not-an-email" })),
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn register_rate_limit_returns_429() {
    let pool = test_pool().await;
    let app = accounts::router(test_state(pool), rate_limits(2));

    for _ in 0..2 {
        let email = unique_email("register-rate");
        let (status, _) = register_account(&app, &email).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = register_account(&app, &unique_email("register-rate-blocked")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    assert_eq!(
        body["error"]["message"],
        "Too many registration attempts. Please try again later."
    );
}

#[tokio::test]
async fn accounts_me_requires_api_key() {
    let pool = test_pool().await;
    let app = accounts::router(test_state(pool), rate_limits(100));

    let (status, _) = response_json(app, peer_request("GET", "/me", None, None)).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn accounts_me_returns_profile_with_valid_key() {
    let pool = test_pool().await;
    let email = unique_email("accounts-me");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let api_key = registered["api_key"].as_str().unwrap();

    let (status, body) =
        response_json(app.clone(), peer_request("GET", "/me", None, Some(api_key))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], email);
    assert_eq!(body["plan_name"], "free");
    assert_eq!(body["subscription_status"], "none");

    cleanup_account(
        &pool,
        Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn list_api_keys_returns_prefixes_only() {
    let pool = test_pool().await;
    let email = unique_email("list-keys");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let api_key = registered["api_key"].as_str().unwrap();
    let full_key = api_key.to_string();

    let (status, body) = response_json(
        app.clone(),
        peer_request("GET", "/api-keys", None, Some(api_key)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["api_keys"].as_array().unwrap().len() >= 1);
    let serialized = body.to_string();
    assert!(!serialized.contains(&full_key[api_key::API_KEY_PREFIX.len()..]));
    assert!(body["api_keys"][0]["key_prefix"]
        .as_str()
        .unwrap()
        .starts_with("ev_"));

    cleanup_account(
        &pool,
        Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn create_api_key_returns_new_key_once() {
    let pool = test_pool().await;
    let email = unique_email("create-key");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let api_key = registered["api_key"].as_str().unwrap();

    let (status, body) = response_json(
        app.clone(),
        peer_request(
            "POST",
            "/api-keys",
            Some(json!({ "label": "ci" })),
            Some(api_key),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert!(body["api_key"]
        .as_str()
        .unwrap()
        .starts_with(api_key::API_KEY_PREFIX));
    assert_ne!(body["api_key"].as_str().unwrap(), api_key);

    cleanup_account(
        &pool,
        Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn revoke_own_api_key_returns_no_content() {
    let pool = test_pool().await;
    let email = unique_email("revoke-key");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let primary_key = registered["api_key"].as_str().unwrap().to_string();

    let (_, created) = response_json(
        app.clone(),
        peer_request("POST", "/api-keys", Some(json!({})), Some(&primary_key)),
    )
    .await;
    let revoke_id = created["id"].as_str().unwrap();

    let (status, _) = response_json(
        app.clone(),
        peer_request(
            "DELETE",
            &format!("/api-keys/{revoke_id}"),
            None,
            Some(&primary_key),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = response_json(
        app.clone(),
        peer_request(
            "GET",
            "/me",
            None,
            Some(created["api_key"].as_str().unwrap()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) =
        response_json(app, peer_request("GET", "/me", None, Some(&primary_key))).await;
    assert_eq!(status, StatusCode::OK);

    cleanup_account(
        &pool,
        Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn revoke_foreign_api_key_returns_not_found() {
    let pool = test_pool().await;
    let email_a = unique_email("foreign-a");
    let email_b = unique_email("foreign-b");
    cleanup_email(&pool, &email_a).await;
    cleanup_email(&pool, &email_b).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, account_a) = register_account(&app, &email_a).await;
    let (_, account_b) = register_account(&app, &email_b).await;

    let key_a = account_a["api_key"].as_str().unwrap();
    let key_b_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT api_key_id FROM api_keys WHERE account_id = $1 LIMIT 1",
    )
    .bind(Uuid::parse_str(account_b["account_id"].as_str().unwrap()).unwrap())
    .fetch_one(&pool)
    .await
    .expect("key b");

    let (status, body) = response_json(
        app,
        peer_request(
            "DELETE",
            &format!("/api-keys/{key_b_id}"),
            None,
            Some(key_a),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    cleanup_account(
        &pool,
        Uuid::parse_str(account_a["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
    cleanup_account(
        &pool,
        Uuid::parse_str(account_b["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn revoke_last_active_api_key_returns_conflict() {
    let pool = test_pool().await;
    let email = unique_email("last-key");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let api_key = registered["api_key"].as_str().unwrap();

    let key_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT api_key_id FROM api_keys WHERE account_id = $1 LIMIT 1",
    )
    .bind(Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap())
    .fetch_one(&pool)
    .await
    .expect("key id");

    let (status, body) = response_json(
        app,
        peer_request(
            "DELETE",
            &format!("/api-keys/{key_id}"),
            None,
            Some(api_key),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "last_api_key");

    cleanup_account(
        &pool,
        Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn revoked_api_key_is_rejected() {
    let pool = test_pool().await;
    let email = unique_email("revoked-use");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let primary_key = registered["api_key"].as_str().unwrap().to_string();

    let (_, created) = response_json(
        app.clone(),
        peer_request("POST", "/api-keys", Some(json!({})), Some(&primary_key)),
    )
    .await;

    let primary_key_id =
        sqlx::query_scalar::<_, Uuid>("SELECT api_key_id FROM api_keys WHERE key_hash = $1")
            .bind(api_key::hash_api_key_for_lookup(&primary_key))
            .fetch_one(&pool)
            .await
            .expect("primary key id");

    let (status, _) = response_json(
        app.clone(),
        peer_request(
            "DELETE",
            &format!("/api-keys/{primary_key_id}"),
            None,
            Some(created["api_key"].as_str().unwrap()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = response_json(
        app.clone(),
        peer_request("GET", "/me", None, Some(&primary_key)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = response_json(
        app,
        peer_request(
            "GET",
            "/me",
            None,
            Some(created["api_key"].as_str().unwrap()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    cleanup_account(
        &pool,
        Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap(),
    )
    .await;
}

#[tokio::test]
async fn list_api_keys_shows_legacy_placeholder_for_pre_stage81_rows() {
    let pool = test_pool().await;
    let email = unique_email("legacy-prefix-list");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let account_id = Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap();
    let new_key = registered["api_key"].as_str().unwrap();

    let legacy_plaintext = format!("dev-legacy-{}", Uuid::new_v4());
    let legacy_hash = api_key::hash_api_key_for_lookup(&legacy_plaintext);
    sqlx::query(
        r#"
        INSERT INTO api_keys (account_id, key_hash, key_prefix, label)
        VALUES ($1, $2, $3, 'dev-legacy')
        "#,
    )
    .bind(account_id)
    .bind(&legacy_hash)
    .bind(api_key::LEGACY_KEY_PREFIX_STORED)
    .execute(&pool)
    .await
    .expect("insert legacy key");

    let (status, body) = response_json(
        app.clone(),
        peer_request("GET", "/api-keys", None, Some(new_key)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let prefixes: Vec<&str> = body["api_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["key_prefix"].as_str().unwrap())
        .collect();
    assert!(prefixes.contains(&api_key::LEGACY_KEY_PREFIX_DISPLAY));
    assert!(!prefixes
        .iter()
        .any(|p| *p == api_key::LEGACY_KEY_PREFIX_STORED));
    assert!(!prefixes.iter().any(|p| p.starts_with("ev_legacy")));

    let (status, _) = response_json(
        app,
        peer_request("GET", "/me", None, Some(&legacy_plaintext)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn list_api_keys_normalizes_ev_legacy_sentinel_from_first_migration() {
    let pool = test_pool().await;
    let email = unique_email("ev-legacy-sentinel");
    cleanup_email(&pool, &email).await;

    let app = accounts::router(test_state(pool.clone()), rate_limits(100));
    let (_, registered) = register_account(&app, &email).await;
    let account_id = Uuid::parse_str(registered["account_id"].as_str().unwrap()).unwrap();
    let new_key = registered["api_key"].as_str().unwrap();

    sqlx::query(
        r#"
        UPDATE api_keys
        SET key_prefix = 'ev_legacy'
        WHERE account_id = $1
          AND key_hash = $2
        "#,
    )
    .bind(account_id)
    .bind(api_key::hash_api_key_for_lookup(new_key))
    .execute(&pool)
    .await
    .expect("simulate first migration backfill");

    let (status, body) =
        response_json(app, peer_request("GET", "/api-keys", None, Some(new_key))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["api_keys"][0]["key_prefix"].as_str().unwrap(),
        api_key::LEGACY_KEY_PREFIX_DISPLAY
    );

    cleanup_account(&pool, account_id).await;
}
