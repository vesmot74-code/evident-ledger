//! Stage 8.3.1b — Dashboard web UI tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use evident_ledger::api::{auth, dashboard as dashboard_api};
use evident_ledger::config::AppConfig;
use evident_ledger::state::rate_limiter::LoginRateLimitState;
use evident_ledger::state::AppState;
use evident_ledger::web::dashboard as dashboard_ui;
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn full_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/login", axum::routing::get(dashboard_ui::login_page))
        .route("/register", axum::routing::get(dashboard_ui::register_page))
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest(
            "/dashboard",
            dashboard_ui::router(state.clone()).merge(dashboard_api::router(state)),
        )
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
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

async fn call_text(app: axum::Router, req: Request<Body>) -> (StatusCode, String, Vec<String>) {
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
    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        cookies,
    )
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

async fn register_and_login(app: &axum::Router, email: &str) -> String {
    call_text(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            &[],
        ),
    )
    .await;

    let (_, _, cookies) = call_text(
        app.clone(),
        peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            &[],
        ),
    )
    .await;

    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

#[tokio::test]
async fn dashboard_without_session_redirects_to_login() {
    let pool = test_pool().await;
    let app = full_app(test_state(pool));

    let (status, body, _) =
        call_text(app, peer_request("GET", "/dashboard/ui", None, None, &[])).await;

    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(body.is_empty() || body.contains("/login"));
}

#[tokio::test]
async fn dashboard_home_renders_profile_after_login() {
    let pool = test_pool().await;
    let email = format!("ui-home-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call_text(
        app,
        peer_request("GET", "/dashboard/ui", None, Some(&cookie), &[]),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(&email));
    assert!(body.contains("FREE"));
    assert!(body.contains("Manage"));
    assert!(body.contains("cdn.paddle.com/paddle/v2/paddle.js"));
    assert!(body.contains(r#"name="paddle-client-token""#));
    assert!(body.contains("Paddle.Setup"));
    assert!(!body.contains("pdl_sdbx_apikey_"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_subscription_page_renders_plan() {
    let pool = test_pool().await;
    let email = format!("ui-sub-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call_text(
        app,
        peer_request(
            "GET",
            "/dashboard/ui/subscription",
            None,
            Some(&cookie),
            &[],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Subscription"));
    assert!(body.contains("FREE"));
    assert!(body.contains("none"));
    assert!(body.contains("cdn.paddle.com/paddle/v2/paddle.js"));
    assert!(body.contains(r#"name="paddle-client-token""#));
    assert!(body.contains("Paddle.Setup"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_usage_page_renders_usage() {
    let pool = test_pool().await;
    let email = format!("ui-usage-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call_text(
        app,
        peer_request("GET", "/dashboard/ui/usage", None, Some(&cookie), &[]),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Usage"));
    assert!(body.contains("0 / 100"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_api_keys_page_lists_prefixes_only() {
    let pool = test_pool().await;
    let email = format!("ui-keys-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call_text(
        app,
        peer_request("GET", "/dashboard/ui/api-keys", None, Some(&cookie), &[]),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("API Keys"));
    assert!(body.contains("ev_"));
    assert!(!body.contains("password_hash"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn create_api_key_ui_requires_htmx_headers() {
    let pool = test_pool().await;
    let email = format!("ui-csrf-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, _, _) = call_text(
        app.clone(),
        peer_request("POST", "/dashboard/ui/api-keys", None, Some(&cookie), &[]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body, _) = call_text(
        app,
        peer_request(
            "POST",
            "/dashboard/ui/api-keys",
            None,
            Some(&cookie),
            &[("hx-request", "true"), ("origin", "http://localhost")],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("API key created"));
    assert!(body.contains("ev_"));
    assert!(body.contains("This secret will not be shown again"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn revoke_api_key_ui_returns_revoked_fragment() {
    let pool = test_pool().await;
    let email = format!("ui-revoke-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (_, create_body, _) = call_text(
        app.clone(),
        peer_request(
            "POST",
            "/dashboard/ui/api-keys",
            None,
            Some(&cookie),
            &[("hx-request", "true"), ("origin", "http://localhost")],
        ),
    )
    .await;

    let key_id: Uuid = sqlx::query_scalar(
        r#"
        SELECT api_key_id
        FROM api_keys
        WHERE account_id = (SELECT account_id FROM accounts WHERE email = $1)
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .expect("key id");

    let (status, body, _) = call_text(
        app,
        peer_request(
            "DELETE",
            &format!("/dashboard/ui/api-keys/{key_id}"),
            None,
            Some(&cookie),
            &[("hx-request", "true"), ("origin", "http://localhost")],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Revoked"));
    assert!(!create_body.contains("password_hash"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn login_page_renders_form() {
    let pool = test_pool().await;
    let app = full_app(test_state(pool));

    let (status, body, _) = call_text(app, peer_request("GET", "/login", None, None, &[])).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Sign in"));
    assert!(body.contains("/auth/login"));
    assert!(body.contains("/register"));
    assert!(body.contains("Create account"));
    assert!(!body.contains("password_hash"));
}

#[tokio::test]
async fn register_page_renders_form() {
    let pool = test_pool().await;
    let app = full_app(test_state(pool));

    let (status, body, _) = call_text(app, peer_request("GET", "/register", None, None, &[])).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Create your account"));
    assert!(body.contains("id=\"email\""));
    assert!(body.contains("id=\"password\""));
    assert!(body.contains("type=\"submit\"") || body.contains("Create account"));
    assert!(body.contains("/auth/register"));
    assert!(!body.contains("password_hash"));
}

#[tokio::test]
async fn register_flow_creates_account_and_duplicate_returns_conflict() {
    let pool = test_pool().await;
    let email = format!("ui-register-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));

    let (status, body, cookies) = call_text(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            &[],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    let json: Value = serde_json::from_str(&body).expect("register json");
    assert_eq!(json["email"], email);
    assert_eq!(json["plan"], "free");
    assert!(json.get("account_id").is_some());
    assert!(
        cookies.iter().all(|c| !c.contains("evident_session=")),
        "registration must not set a session cookie"
    );

    let (status, body, _) = call_text(
        app,
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass1" })),
            None,
            &[],
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    let json: Value = serde_json::from_str(&body).expect("conflict json");
    assert_eq!(json["error"]["code"], "conflict");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_html_does_not_leak_session_token() {
    let pool = test_pool().await;
    let email = format!("ui-leak-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;
    let token = cookie.split('=').nth(1).expect("token");

    for path in [
        "/dashboard/ui",
        "/dashboard/ui/subscription",
        "/dashboard/ui/usage",
        "/dashboard/ui/api-keys",
    ] {
        let (_, body, _) = call_text(
            app.clone(),
            peer_request("GET", path, None, Some(&cookie), &[]),
        )
        .await;
        assert!(!body.contains(token), "session token leaked on {path}");
        assert!(
            !body.to_lowercase().contains("password_hash"),
            "password_hash leaked on {path}"
        );
    }
    cleanup_email(&pool, &email).await;
}
