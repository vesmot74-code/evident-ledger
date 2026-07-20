//! Stage 8.3.0 — web authentication tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use evident_ledger::api::auth;
use evident_ledger::auth::password;
use evident_ledger::auth::session_store::{hash_session_token, SESSION_COOKIE_NAME};
use evident_ledger::config::AppConfig;
use evident_ledger::service::accounts;
use evident_ledger::state::rate_limiter::LoginRateLimitState;
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

fn auth_app(state: AppState) -> axum::Router {
    auth::router(state, LoginRateLimitState::from_config(false))
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    api_key: Option<&str>,
    cookie: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = api_key {
        builder = builder.header("X-API-KEY", key);
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
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
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 60)),
        0,
    )));
    req
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value, Vec<String>) {
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({ "raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, json, cookies)
}

fn cookie_header_from_set_cookie(set_cookies: &[String]) -> Option<String> {
    set_cookies
        .iter()
        .find_map(|line| line.split(';').next().map(str::trim))
        .map(str::to_string)
}

async fn cleanup_email(pool: &sqlx::PgPool, email: &str) {
    let _ = sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE account_id IN (SELECT account_id FROM accounts WHERE email = $1)
        "#,
    )
    .bind(email)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM api_keys WHERE account_id IN (SELECT account_id FROM accounts WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn register_new_email_creates_account_with_password_hash() {
    let pool = test_pool().await;
    let email = format!("web-new-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["plan"], "free");

    let hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM accounts WHERE email = $1")
            .bind(&email)
            .fetch_one(&pool)
            .await
            .expect("hash");
    assert!(hash.is_some());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn register_existing_api_only_email_returns_conflict() {
    let pool = test_pool().await;
    let email = format!("api-only-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    accounts::register_account(&pool, &email)
        .await
        .expect("api register");

    let app = auth_app(test_state(pool.clone()));
    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "conflict");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn register_existing_web_email_returns_conflict() {
    let pool = test_pool().await;
    let email = format!("web-dup-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));

    let req = peer_request(
        "POST",
        "/register",
        Some(json!({ "email": email, "password": "securepass1" })),
        None,
        None,
    );
    let _ = call(app.clone(), req).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "otherpass1" })),
            None,
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "conflict");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn set_password_with_api_key_succeeds_for_api_only_account() {
    let pool = test_pool().await;
    let email = format!("set-pass-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let registered = accounts::register_account(&pool, &email)
        .await
        .expect("register");

    let app = auth_app(test_state(pool.clone()));
    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/set-password",
            Some(json!({ "password": "newsecure1" })),
            Some(&registered.api_key),
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"], "Password set successfully");

    let hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM accounts WHERE account_id = $1")
            .bind(registered.account_id)
            .fetch_one(&pool)
            .await
            .expect("hash");
    assert!(hash.is_some());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn set_password_without_api_key_returns_unauthorized() {
    let pool = test_pool().await;
    let app = auth_app(test_state(pool));
    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/set-password",
            Some(json!({ "password": "newsecure1" })),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn set_password_when_already_set_returns_conflict() {
    let pool = test_pool().await;
    let email = format!("already-pass-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let registered = accounts::register_account(&pool, &email)
        .await
        .expect("register");

    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/set-password",
            Some(json!({ "password": "firstpass1" })),
            Some(&registered.api_key),
            None,
        ),
    )
    .await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/set-password",
            Some(json!({ "password": "secondpass1" })),
            Some(&registered.api_key),
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "conflict");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn login_with_valid_password_sets_cookie() {
    let pool = test_pool().await;
    let email = format!("login-ok-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    let (status, body, cookies) = call(
        app,
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], email);
    assert!(cookies
        .iter()
        .any(|c| c.starts_with(&format!("{SESSION_COOKIE_NAME}="))));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn login_with_wrong_password_returns_unauthorized() {
    let pool = test_pool().await;
    let email = format!("login-bad-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "wrongpass1" })),
            None,
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn login_with_unknown_email_returns_unauthorized() {
    let pool = test_pool().await;
    let app = auth_app(test_state(pool));
    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": "missing@example.com", "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn login_api_only_account_returns_unauthorized() {
    let pool = test_pool().await;
    let email = format!("login-api-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    accounts::register_account(&pool, &email)
        .await
        .expect("register");

    let app = auth_app(test_state(pool.clone()));
    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn me_with_valid_session_returns_profile() {
    let pool = test_pool().await;
    let email = format!("me-ok-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    let (_, _, cookies) = call(
        app.clone(),
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    let cookie = cookie_header_from_set_cookie(&cookies).expect("cookie");

    let (status, body, _) = call(app, peer_request("GET", "/me", None, None, Some(&cookie))).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], email);
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn me_without_session_returns_unauthorized() {
    let pool = test_pool().await;
    let app = auth_app(test_state(pool));
    let (status, body, _) = call(app, peer_request("GET", "/me", None, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn logout_deletes_session() {
    let pool = test_pool().await;
    let email = format!("logout-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    let (_, _, cookies) = call(
        app.clone(),
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    let cookie = cookie_header_from_set_cookie(&cookies).expect("cookie");

    let (status, _, _) = call(
        app.clone(),
        peer_request("POST", "/logout", None, None, Some(&cookie)),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body, _) = call(app, peer_request("GET", "/me", None, None, Some(&cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn me_with_expired_session_returns_unauthorized() {
    let pool = test_pool().await;
    let email = format!("expired-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    let (_, _, cookies) = call(
        app.clone(),
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;
    let cookie = cookie_header_from_set_cookie(&cookies).expect("cookie");
    let token = cookie
        .strip_prefix(&format!("{SESSION_COOKIE_NAME}="))
        .expect("token");

    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 hour' WHERE token_hash = $1")
        .bind(hash_session_token(token))
        .execute(&pool)
        .await
        .expect("expire");

    let (status, body, _) = call(app, peer_request("GET", "/me", None, None, Some(&cookie))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn login_rate_limit_blocks_eleventh_attempt() {
    let pool = test_pool().await;
    let email = format!("ratelimit-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = auth_app(test_state(pool.clone()));
    call(
        app.clone(),
        peer_request(
            "POST",
            "/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    for _ in 0..10 {
        let (status, _, _) = call(
            app.clone(),
            peer_request(
                "POST",
                "/login",
                Some(json!({ "email": email, "password": "wrongpass1" })),
                None,
                None,
            ),
        )
        .await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/login",
            Some(json!({ "email": email, "password": "wrongpass1" })),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    cleanup_email(&pool, &email).await;
}

#[test]
fn argon2_hash_does_not_contain_plaintext() {
    let password = "unit_test_password";
    let hash = password::hash_password(password).expect("hash");
    assert!(!hash.contains(password));
}
