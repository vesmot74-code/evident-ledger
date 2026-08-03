//! Preserve desktop connect redirect_uri across anonymous → /login → return.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use evident_ledger::api::{auth, dashboard_desktop};
use evident_ledger::state::rate_limiter::LoginRateLimitState;
use evident_ledger::state::AppState;
use evident_ledger::web;
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

/// Mirrors production nesting: UI desktop connect under /dashboard + /login + /auth.
fn app(state: AppState) -> axum::Router {
    let dashboard_ui = web::dashboard::router(state.clone())
        .merge(dashboard_desktop::ui_router(state.clone()));

    axum::Router::new()
        .route("/login", axum::routing::get(web::dashboard::login_page))
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest("/dashboard", dashboard_ui)
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

async fn call_raw(
    app: axum::Router,
    req: Request<Body>,
) -> (StatusCode, Option<String>, String, Vec<String>) {
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body = String::from_utf8_lossy(&bytes).into_owned();
    (status, location, body, cookies)
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
    let _ = call_raw(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass12" })),
            None,
        ),
    )
    .await;

    let (_, _, _, cookies) = call_raw(
        app.clone(),
        peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass12" })),
            None,
        ),
    )
    .await;

    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

#[tokio::test]
async fn anonymous_desktop_connect_preserves_redirect_uri_in_login_next() {
    let pool = test_pool().await;
    let app = app(test_state(pool));

    let connect_uri =
        "/dashboard/desktop/connect?redirect_uri=http://127.0.0.1:5555/callback";
    let (status, location, _, _) =
        call_raw(app, peer_request("GET", connect_uri, None, None)).await;

    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.expect("Location header");
    let expected_next = percent_encode_test(connect_uri);
    assert_eq!(location, format!("/login?next={expected_next}"));
}

#[tokio::test]
async fn logged_in_desktop_connect_renders_confirmation_page() {
    let pool = test_pool().await;
    let email = format!("desktop-connect-ui-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = app(test_state(pool.clone()));
    let cookie = register_and_login(&app, &email).await;

    let connect_uri =
        "/dashboard/desktop/connect?redirect_uri=http://127.0.0.1:5555/callback";
    let (status, location, body, _) = call_raw(
        app,
        peer_request("GET", connect_uri, None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(location.is_none());
    assert!(
        body.contains("Connect this computer") || body.contains("Connect Desktop"),
        "expected connect confirmation HTML, got: {}",
        &body[..body.len().min(200)]
    );
    assert!(body.contains("http://127.0.0.1:5555/callback"));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn login_still_works_when_page_opened_with_next_query() {
    let pool = test_pool().await;
    let email = format!("desktop-connect-login-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = app(test_state(pool.clone()));

    // Case 3 (partial): /login serves HTML with next in browser URL; POST /auth/login
    // is unaffected by the query string on the login page.
    let next = percent_encode_test(
        "/dashboard/desktop/connect?redirect_uri=http://127.0.0.1:5555/callback",
    );
    let (status, _, body, _) =
        call_raw(app.clone(), peer_request("GET", &format!("/login?next={next}"), None, None))
            .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("login-form") || body.contains("Sign in"));

    let _ = call_raw(
        app.clone(),
        peer_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass12" })),
            None,
        ),
    )
    .await;

    let (login_status, _, _, cookies) = call_raw(
        app,
        peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass12" })),
            None,
        ),
    )
    .await;
    assert_eq!(login_status, StatusCode::OK);
    assert!(cookie_header_from_set_cookie(&cookies).is_some());

    cleanup_email(&pool, &email).await;
}

fn percent_encode_test(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
