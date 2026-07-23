//! Stage 13.4 — Desktop Bearer token bridge.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use evident_ledger::api::{auth, dashboard, dashboard_desktop, v1};
use evident_ledger::auth::desktop_token;
use evident_ledger::state::rate_limiter::LoginRateLimitState;
use evident_ledger::state::AppState;
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
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
        .nest(
            "/dashboard",
            dashboard::router(state.clone()).merge(dashboard_desktop::api_router(state.clone())),
        )
        .nest("/v1", v1::router(state))
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    bearer: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
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
        DELETE FROM desktop_tokens
        WHERE account_id IN (SELECT account_id FROM accounts WHERE email = $1)
        "#,
    )
    .bind(email)
    .execute(pool)
    .await;
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

async fn register_and_login(app: &axum::Router, email: &str) -> String {
    call(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
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
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            None,
        ),
    )
    .await;

    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

#[tokio::test]
async fn logged_user_can_create_desktop_token() {
    let pool = test_pool().await;
    let email = format!("desktop-create-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/desktop/connect",
            None,
            Some(&cookie),
            None,
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token");
    assert!(token.starts_with("desktop_"));
    assert!(body["expires_at"].is_string());
    assert!(body["token_id"].is_string());

    let stored: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT token_hash FROM desktop_tokens
        WHERE account_id = (SELECT account_id FROM accounts WHERE email = $1)
        "#,
    )
    .bind(&email)
    .fetch_optional(&pool)
    .await
    .expect("query");
    let hash = stored.expect("row").0;
    assert_ne!(hash, token);
    assert_eq!(
        desktop_token::hash_desktop_token_for_lookup(token).as_deref(),
        Some(hash.as_str())
    );

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn desktop_bearer_can_call_v1_me() {
    let pool = test_pool().await;
    let email = format!("desktop-me-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (_, created, _) = call(
        app.clone(),
        peer_request(
            "POST",
            "/dashboard/desktop/connect",
            None,
            Some(&cookie),
            None,
        ),
    )
    .await;
    let token = created["token"].as_str().expect("token").to_string();

    let (status, body, _) = call(
        app,
        peer_request("GET", "/v1/me", None, None, Some(&token)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], email);
    assert!(body["plan"].is_string());
    assert!(body["plan_display"].is_string());

    let last_used: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        r#"
        SELECT last_used_at FROM desktop_tokens
        WHERE token_hash = $1
        "#,
    )
    .bind(desktop_token::hash_desktop_token_for_lookup(&token).unwrap())
    .fetch_optional(&pool)
    .await
    .expect("query");
    assert!(last_used.expect("row").0.is_some());

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn expired_desktop_token_returns_401() {
    let pool = test_pool().await;
    let email = format!("desktop-exp-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (_, created, _) = call(
        app.clone(),
        peer_request(
            "POST",
            "/dashboard/desktop/connect",
            None,
            Some(&cookie),
            None,
        ),
    )
    .await;
    let token = created["token"].as_str().expect("token").to_string();
    let token_hash = desktop_token::hash_desktop_token_for_lookup(&token).unwrap();

    sqlx::query(
        r#"
        UPDATE desktop_tokens
        SET expires_at = now() - interval '1 hour'
        WHERE token_hash = $1
        "#,
    )
    .bind(&token_hash)
    .execute(&pool)
    .await
    .expect("expire");

    let (status, _, _) = call(
        app,
        peer_request("GET", "/v1/me", None, None, Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn revoked_desktop_token_returns_401() {
    let pool = test_pool().await;
    let email = format!("desktop-rev-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = combined_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (_, created, _) = call(
        app.clone(),
        peer_request(
            "POST",
            "/dashboard/desktop/connect",
            None,
            Some(&cookie),
            None,
        ),
    )
    .await;
    let token = created["token"].as_str().expect("token").to_string();
    let token_id = created["token_id"].as_str().expect("token_id");

    let (revoke_status, _, _) = call(
        app.clone(),
        peer_request(
            "POST",
            &format!("/dashboard/desktop/tokens/{token_id}/revoke"),
            None,
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(revoke_status, StatusCode::NO_CONTENT);

    let (status, _, _) = call(
        app,
        peer_request("GET", "/v1/me", None, None, Some(&token)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn connect_without_session_returns_401() {
    let pool = test_pool().await;
    let app = combined_app(test_state(pool));

    let (status, _, _) = call(
        app,
        peer_request("POST", "/dashboard/desktop/connect", None, None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
