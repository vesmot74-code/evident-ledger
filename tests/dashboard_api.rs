//! Stage 8.3.1a — Dashboard API contract tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use evident_ledger::api::{auth, dashboard};
use evident_ledger::auth::session_store::SESSION_COOKIE_NAME;
use evident_ledger::config::AppConfig;
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

fn combined_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest("/dashboard", dashboard::router(state))
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
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

async fn register_and_login(_pool: &sqlx::PgPool, app: &axum::Router, email: &str) -> String {
    call(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
        ),
    )
    .await;

    let (_, _, cookies) = call(
        app.clone(),
        peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
        ),
    )
    .await;

    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

#[tokio::test]
async fn get_dashboard_me_with_valid_session_returns_profile() {
    let pool = test_pool().await;
    let email = format!("dash-me-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request("GET", "/dashboard/me", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], email);
    assert_eq!(body["plan"], "free");
    assert_eq!(body["plan_display"], "Free");
    assert_eq!(body["subscription_status"], "none");
    assert_eq!(body["email_verified"], false);
    assert!(body["account_id"].is_string());
    assert!(body["created_at"].is_string());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn get_dashboard_me_without_cookie_returns_unauthorized() {
    let pool = test_pool().await;
    let app = combined_app(test_state(pool));

    let (status, body, _) = call(app, peer_request("GET", "/dashboard/me", None, None)).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn get_dashboard_subscription_with_valid_session_returns_state() {
    let pool = test_pool().await;
    let email = format!("dash-sub-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request("GET", "/dashboard/subscription", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["plan"], "free");
    assert_eq!(body["plan_display"], "Free");
    assert_eq!(body["subscription_status"], "none");
    assert!(body["current_period_end"].is_null());
    assert!(body["pending_plan"].is_null());
    assert!(body["pending_plan_display"].is_null());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn get_dashboard_usage_with_valid_session_returns_usage() {
    let pool = test_pool().await;
    let email = format!("dash-usage-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request("GET", "/dashboard/usage", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["period"].is_string());
    assert_eq!(body["server_commits"], 0);
    assert_eq!(body["monthly_limit"], 100);
    assert_eq!(body["percentage"], 0);
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn get_dashboard_api_keys_with_valid_session_returns_prefixes_only() {
    let pool = test_pool().await;
    let email = format!("dash-keys-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request("GET", "/dashboard/api-keys", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let keys = body["api_keys"].as_array().expect("api_keys array");
    assert!(!keys.is_empty());
    for key in keys {
        assert!(key["key_id"].is_string());
        assert!(key["prefix"].as_str().unwrap().starts_with("ev_"));
        assert!(key["created_at"].is_string());
        assert!(key["last_used_at"].is_null());
        assert!(key["is_active"].is_boolean());
        assert!(key.get("secret").is_none());
        assert!(key.get("key_hash").is_none());
    }
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_api_keys_with_valid_session_returns_new_key() {
    let pool = test_pool().await;
    let email = format!("dash-create-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request("POST", "/dashboard/api-keys", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["api_key"].as_str().unwrap().starts_with("ev_"));
    assert!(body["key_id"].is_string());
    assert!(body["created_at"].is_string());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn delete_dashboard_api_key_with_valid_session_revokes_key() {
    let pool = test_pool().await;
    let email = format!("dash-revoke-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    let (_, create_body, _) = call(
        app.clone(),
        peer_request("POST", "/dashboard/api-keys", None, Some(&cookie)),
    )
    .await;
    let key_id = create_body["key_id"].as_str().unwrap();

    let (status, _, _) = call(
        app,
        peer_request(
            "DELETE",
            &format!("/dashboard/api-keys/{key_id}"),
            None,
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NO_CONTENT);

    let revoked_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT revoked_at FROM api_keys WHERE api_key_id = $1",
    )
    .bind(Uuid::parse_str(key_id).unwrap())
    .fetch_one(&pool)
    .await
    .expect("revoked_at");
    assert!(revoked_at.is_some());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn delete_dashboard_api_key_for_foreign_key_returns_not_found() {
    let pool = test_pool().await;
    let email_a = format!("dash-owner-{}@example.com", Uuid::new_v4());
    let email_b = format!("dash-victim-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email_a).await;
    cleanup_email(&pool, &email_b).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie_a = register_and_login(&pool, &app, &email_a).await;
    register_and_login(&pool, &app, &email_b).await;

    let victim_key_id: Uuid = sqlx::query_scalar(
        r#"
        SELECT api_key_id
        FROM api_keys
        WHERE account_id = (SELECT account_id FROM accounts WHERE email = $1)
        LIMIT 1
        "#,
    )
    .bind(&email_b)
    .fetch_one(&pool)
    .await
    .expect("victim key");

    let (status, body, _) = call(
        app,
        peer_request(
            "DELETE",
            &format!("/dashboard/api-keys/{victim_key_id}"),
            None,
            Some(&cookie_a),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    cleanup_email(&pool, &email_a).await;
    cleanup_email(&pool, &email_b).await;
}

#[tokio::test]
async fn dashboard_responses_do_not_leak_password_hash() {
    let pool = test_pool().await;
    let email = format!("dash-leak-pw-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;

    for path in [
        "/dashboard/me",
        "/dashboard/subscription",
        "/dashboard/usage",
        "/dashboard/api-keys",
    ] {
        let (_, body, _) = call(
            app.clone(),
            peer_request("GET", path, None, Some(&cookie)),
        )
        .await;
        let serialized = body.to_string().to_lowercase();
        assert!(
            !serialized.contains("password_hash"),
            "leaked password_hash on {path}"
        );
    }
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_responses_do_not_leak_session_token() {
    let pool = test_pool().await;
    let email = format!("dash-leak-sess-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&pool, &app, &email).await;
    let token = cookie
        .split('=')
        .nth(1)
        .expect("token in cookie header");

    for path in [
        "/dashboard/me",
        "/dashboard/subscription",
        "/dashboard/usage",
        "/dashboard/api-keys",
    ] {
        let (_, body, _) = call(
            app.clone(),
            peer_request("GET", path, None, Some(&cookie)),
        )
        .await;
        let serialized = body.to_string();
        assert!(
            !serialized.contains(token),
            "leaked session token on {path}"
        );
        assert!(
            !serialized.to_lowercase().contains("session_token"),
            "leaked session_token field on {path}"
        );
    }
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn session_cookie_has_required_security_attributes() {
    let pool = test_pool().await;
    let email = format!("dash-cookie-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));

    call(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
        ),
    )
    .await;

    let (_, _, cookies) = call(
        app,
        peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
        ),
    )
    .await;

    let session_cookie = cookies
        .iter()
        .find(|c| c.starts_with(&format!("{SESSION_COOKIE_NAME}=")))
        .expect("session cookie");
    assert!(session_cookie.contains("HttpOnly"));
    assert!(session_cookie.contains("SameSite=Lax"));
    cleanup_email(&pool, &email).await;
}
