//! Stage 9.7 — dashboard identity key revoke UI tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use ed25519_dalek::SigningKey;
use evident_ledger::api::auth;
use evident_ledger::auth::api_key;
use evident_ledger::models::identity_key::IdentityKey;
use evident_ledger::service::identity_keys::IdentityKeyRepository;
use evident_ledger::state::rate_limiter::LoginRateLimitState;
use evident_ledger::state::AppState;
use evident_ledger::web::dashboard as dashboard_ui;
use rand::rngs::OsRng;
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

fn dashboard_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/login", axum::routing::get(dashboard_ui::login_page))
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest("/dashboard", dashboard_ui::router(state))
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
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

async fn call_text(app: axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn cookie_header_from_set_cookie(set_cookies: &[String]) -> Option<String> {
    set_cookies
        .iter()
        .find_map(|line| line.split(';').next().map(str::trim))
        .map(str::to_string)
}

async fn plan_id(pool: &sqlx::PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("plan")
}

struct TestAccount {
    account_id: Uuid,
    api_key: String,
}

async fn create_test_account(pool: &sqlx::PgPool, label: &str) -> TestAccount {
    let account_id = Uuid::new_v4();
    let plan = plan_id(pool, "identity").await;
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, $3, 'active')
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@{label}.test"))
    .bind(plan)
    .execute(pool)
    .await
    .expect("account");

    let generated = api_key::generate_api_key();
    sqlx::query(
        r#"
        INSERT INTO api_keys (api_key_id, account_id, key_hash, key_prefix, label)
        VALUES ($1, $2, $3, $4, 'test')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(&generated.key_hash)
    .bind(&generated.key_prefix)
    .execute(pool)
    .await
    .expect("api key");

    TestAccount {
        account_id,
        api_key: generated.full_key,
    }
}

async fn create_identity_key(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    signing_key: &SigningKey,
) -> Uuid {
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let fingerprint = IdentityKeyRepository::fingerprint_from_public_key_hex(&public_key_hex)
        .expect("fingerprint");
    IdentityKeyRepository::create(
        pool,
        account_id,
        &public_key_hex,
        &fingerprint,
        Some("ui-revoke-test"),
    )
    .await
    .expect("identity key")
    .id
}

async fn set_web_password(pool: &sqlx::PgPool, account_id: Uuid, password: &str) {
    let hash = evident_ledger::auth::password::hash_password(password).expect("hash");
    sqlx::query("UPDATE accounts SET password_hash = $1 WHERE account_id = $2")
        .bind(hash)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("password");
}

async fn login_session(app: axum::Router, email: &str, password: &str) -> String {
    let svc = app.clone().into_service();
    let response = svc
        .oneshot(peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": password })),
            None,
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

async fn fetch_key(pool: &sqlx::PgPool, key_id: Uuid) -> Option<IdentityKey> {
    IdentityKeyRepository::find_by_id(pool, key_id)
        .await
        .expect("fetch key")
}

async fn audit_count(pool: &sqlx::PgPool, key_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM identity_key_audit_events WHERE key_id = $1")
        .bind(key_id)
        .fetch_one(pool)
        .await
        .expect("audit count")
}

async fn audit_actor_type(pool: &sqlx::PgPool, key_id: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT actor_type FROM identity_key_audit_events WHERE key_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(key_id)
    .fetch_one(pool)
    .await
    .expect("actor type")
}

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query(
        "DELETE FROM identity_key_audit_events WHERE key_id IN (SELECT id FROM identity_keys WHERE account_id = $1)",
    )
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

async fn setup_session_account(
    pool: &sqlx::PgPool,
    label: &str,
) -> (TestAccount, String, axum::Router) {
    let account = create_test_account(pool, label).await;
    set_web_password(pool, account.account_id, "dashboard-pass").await;
    let email = format!("{}@{label}.test", account.account_id);
    let app = dashboard_app(test_state(pool.clone()));
    let cookie = login_session(app.clone(), &email, "dashboard-pass").await;
    (account, cookie, app)
}

#[tokio::test]
async fn dashboard_revoke_active_key_returns_success_page() {
    let pool = test_pool().await;
    let (account, cookie, app) = setup_session_account(&pool, "ui-revoke-ok").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let (status, body) = call_text(
        app,
        peer_request(
            "POST",
            &format!("/dashboard/identity/{key_id}/revoke"),
            None,
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Identity Key Revoked"));

    let key = fetch_key(&pool, key_id).await.expect("key");
    assert!(key.revoked_at.is_some());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn dashboard_revoke_already_revoked_key_returns_conflict() {
    let pool = test_pool().await;
    let (account, cookie, app) = setup_session_account(&pool, "ui-revoke-twice").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let (first_status, _) = call_text(
        app.clone(),
        peer_request(
            "POST",
            &format!("/dashboard/identity/{key_id}/revoke"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK);

    let (status, body) = call_text(
        app,
        peer_request(
            "POST",
            &format!("/dashboard/identity/{key_id}/revoke"),
            None,
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("already revoked"));

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn dashboard_revoke_foreign_key_returns_not_found() {
    let pool = test_pool().await;
    let owner = create_test_account(&pool, "ui-revoke-owner").await;
    let (other, cookie, app) = setup_session_account(&pool, "ui-revoke-other").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, owner.account_id, &signing_key).await;

    let (status, _) = call_text(
        app,
        peer_request(
            "POST",
            &format!("/dashboard/identity/{key_id}/revoke"),
            None,
            Some(&cookie),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup_account(&pool, owner.account_id).await;
    cleanup_account(&pool, other.account_id).await;
}

#[tokio::test]
async fn dashboard_revoke_without_session_redirects_to_login() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "ui-revoke-unauth").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let app = dashboard_app(test_state(pool.clone()));

    let svc = app.into_service();
    let response = svc
        .oneshot(peer_request(
            "POST",
            &format!("/dashboard/identity/{key_id}/revoke"),
            None,
            None,
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "/login"
    );

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn dashboard_revoke_creates_audit_event() {
    let pool = test_pool().await;
    let (account, cookie, app) = setup_session_account(&pool, "ui-revoke-audit").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    assert_eq!(audit_count(&pool, key_id).await, 0);

    let (status, _) = call_text(
        app,
        peer_request(
            "POST",
            &format!("/dashboard/identity/{key_id}/revoke"),
            None,
            Some(&cookie),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(audit_count(&pool, key_id).await, 1);
    assert_eq!(audit_actor_type(&pool, key_id).await, "account");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn dashboard_identity_list_hides_revoke_button_for_revoked_key() {
    let pool = test_pool().await;
    let (account, cookie, app) = setup_session_account(&pool, "ui-revoke-hidden").await;
    let active_key = SigningKey::generate(&mut OsRng);
    let revoked_key = SigningKey::generate(&mut OsRng);
    let active_id = create_identity_key(&pool, account.account_id, &active_key).await;
    let revoked_id = create_identity_key(&pool, account.account_id, &revoked_key).await;

    IdentityKeyRepository::revoke(&pool, revoked_id, account.account_id)
        .await
        .expect("revoke");

    let (status, body) = call_text(
        app,
        peer_request("GET", "/dashboard/identity", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains(&format!("/dashboard/identity/{revoked_id}/revoke")));
    assert!(body.contains(&format!("/dashboard/identity/{active_id}/revoke")));

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn dashboard_identity_list_shows_revoke_button_for_active_key() {
    let pool = test_pool().await;
    let (account, cookie, app) = setup_session_account(&pool, "ui-revoke-visible").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let (status, body) = call_text(
        app,
        peer_request("GET", "/dashboard/identity", None, Some(&cookie)),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(&format!("/dashboard/identity/{key_id}/revoke")));
    assert!(body.contains("Revoke this identity key?"));

    cleanup_account(&pool, account.account_id).await;
}
