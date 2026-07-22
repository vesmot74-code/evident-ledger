//! Stage 8.3.2 / 10.1 — Dashboard billing and checkout tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use evident_ledger::api::{auth, dashboard_billing};
use evident_ledger::paddle::client::MockPaddleClient;
use evident_ledger::service::billing;
use evident_ledger::state::rate_limiter::LoginRateLimitState;
use evident_ledger::state::AppState;
use evident_ledger::web::dashboard as dashboard_ui;
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use tower::util::ServiceExt;
use uuid::Uuid;

fn tariff_plan_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

fn billing_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest("/dashboard", dashboard_billing::router(state))
}

fn full_dashboard_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest(
            "/dashboard",
            dashboard_ui::router(state.clone()).merge(dashboard_billing::router(state)),
        )
}

fn state_with_paddle(pool: sqlx::PgPool, paddle: Arc<MockPaddleClient>) -> AppState {
    common::setup_test_env();
    AppState::with_paddle(
        pool,
        Arc::new(evident_ledger::signing::ServerSigner::load_or_create(
            "signing_key.bin",
        )),
        evident_ledger::config::AppConfig::from_env(),
        paddle,
    )
}

fn peer_request(
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
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

fn auth_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
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

async fn register_and_login(app: &axum::Router, email: &str) -> String {
    let _ = call(
        app.clone(),
        auth_request(
            "POST",
            "/auth/register",
            Some(json!({ "email": email, "password": "securepass1" })),
        ),
    )
    .await;

    let (_, _, cookies) = call(
        app.clone(),
        auth_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": "securepass1" })),
        ),
    )
    .await;
    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

async fn set_plan_price(pool: &sqlx::PgPool, plan_name: &str, price_id: &str) {
    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = $2")
        .bind(price_id)
        .bind(plan_name)
        .execute(pool)
        .await
        .expect("set plan price");
}

async fn create_unlinked_account(pool: &sqlx::PgPool, email: &str) -> Uuid {
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, (SELECT plan_id FROM tariff_plans WHERE name = 'free'), 'none')
        "#,
    )
    .bind(account_id)
    .bind(email)
    .execute(pool)
    .await
    .expect("insert account");
    account_id
}

async fn set_all_paid_plan_prices(pool: &sqlx::PgPool) {
    set_plan_price(pool, "legal", "pri_legal_test").await;
    set_plan_price(pool, "vault", "pri_vault_test").await;
    set_plan_price(pool, "identity", "price_identity_test").await;
}

async fn set_account_plan_by_email(pool: &sqlx::PgPool, email: &str, plan_name: &str) {
    sqlx::query(
        r#"
        UPDATE accounts
        SET tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = $1)
        WHERE email = $2
        "#,
    )
    .bind(plan_name)
    .bind(email)
    .execute(pool)
    .await
    .expect("set account plan");
}

async fn fetch_dashboard_html(app: axum::Router, cookie: &str) -> String {
    let svc = app.into_service();
    let response = svc
        .oneshot(peer_request("GET", "/dashboard/ui", Some(cookie), None))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn post_dashboard_upgrade_with_session_returns_checkout_url() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "legal", "pri_legal_test").await;
    let email = format!("bill-upgrade-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let paddle = MockPaddleClient::new();
    let app = billing_app(state_with_paddle(pool.clone(), paddle));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "legal" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["checkout_url"]
        .as_str()
        .unwrap()
        .starts_with("https://paddle.example/checkout/"));
    assert_eq!(
        body["transaction_id"].as_str().unwrap(),
        "txn_mock_pri_legal_test"
    );
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_upgrade_without_session_returns_unauthorized() {
    let pool = test_pool().await;
    let app = billing_app(state_with_paddle(pool, MockPaddleClient::new()));

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            None,
            Some(json!({ "plan": "legal" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn post_dashboard_upgrade_with_active_subscription_returns_conflict() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "legal", "pri_legal_test").await;
    let email = format!("bill-active-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = billing_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;
    let account_id: Uuid = sqlx::query_scalar("SELECT account_id FROM accounts WHERE email = $1")
        .bind(&email)
        .fetch_one(&pool)
        .await
        .expect("account");
    sqlx::query("UPDATE accounts SET subscription_status = 'active' WHERE account_id = $1")
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("activate");

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "legal" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["status"], "already_active");
    assert_eq!(
        body["message"],
        "You already have an active subscription. To change your plan, please contact support."
    );
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_upgrade_unknown_plan_returns_invalid_plan() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    let email = format!("bill-unknown-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = billing_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "hacked_plan" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_plan");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_upgrade_free_plan_returns_invalid_plan() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    let email = format!("bill-free-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = billing_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "free" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_plan");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_upgrade_identity_without_paddle_price_returns_not_purchasable() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    let original: Option<String> =
        sqlx::query_scalar("SELECT paddle_price_id FROM tariff_plans WHERE name = 'identity'")
            .fetch_one(&pool)
            .await
            .expect("identity price");

    sqlx::query("UPDATE tariff_plans SET paddle_price_id = NULL WHERE name = 'identity'")
        .execute(&pool)
        .await
        .expect("clear identity price");

    let email = format!("bill-no-price-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = billing_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "identity" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "plan_not_purchasable");

    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'identity'")
        .bind(original)
        .execute(&pool)
        .await
        .expect("restore identity price");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_upgrade_identity_plan_uses_configured_paddle_price() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "identity", "price_identity_test").await;
    let email = format!("bill-identity-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let paddle = MockPaddleClient::new();
    let app = billing_app(state_with_paddle(pool.clone(), paddle.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "identity" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["checkout_url"]
        .as_str()
        .unwrap()
        .contains("price_identity_test"));

    let (customer_id, price_id) = paddle.last_checkout().expect("checkout call");
    assert!(customer_id.starts_with("ctm_mock_"));
    assert_eq!(price_id, "price_identity_test");

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn post_dashboard_upgrade_vault_plan_returns_checkout_url() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "vault", "pri_vault_test").await;
    let email = format!("bill-vault-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let paddle = MockPaddleClient::new();
    let app = billing_app(state_with_paddle(pool.clone(), paddle.clone()));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "vault" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["checkout_url"]
        .as_str()
        .unwrap()
        .contains("pri_vault_test"));
    assert_eq!(
        body["transaction_id"].as_str().unwrap(),
        "txn_mock_pri_vault_test"
    );
    let (_, price_id) = paddle.last_checkout().expect("checkout");
    assert_eq!(price_id, "pri_vault_test");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_free_account_shows_all_purchasable_upgrade_plans() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_all_paid_plan_prices(&pool).await;
    let email = format!("ui-free-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_dashboard_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;

    let html = fetch_dashboard_html(app, &cookie).await;
    assert!(html.contains(r#"data-plan="legal""#));
    assert!(html.contains(r#"data-plan="vault""#));
    assert!(html.contains(r#"data-plan="identity""#));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_legal_account_hides_legal_upgrade_button() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_all_paid_plan_prices(&pool).await;
    let email = format!("ui-legal-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_dashboard_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;
    set_account_plan_by_email(&pool, &email, "legal").await;

    let html = fetch_dashboard_html(app, &cookie).await;
    assert!(!html.contains(r#"data-plan="legal""#));
    assert!(html.contains(r#"data-plan="vault""#));
    assert!(html.contains(r#"data-plan="identity""#));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_vault_account_shows_identity_upgrade_only() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_all_paid_plan_prices(&pool).await;
    let email = format!("ui-vault-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_dashboard_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;
    set_account_plan_by_email(&pool, &email, "vault").await;

    let html = fetch_dashboard_html(app, &cookie).await;
    assert!(!html.contains(r#"data-plan="legal""#));
    assert!(!html.contains(r#"data-plan="vault""#));
    assert!(html.contains(r#"data-plan="identity""#));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_identity_account_hides_upgrade_section() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_all_paid_plan_prices(&pool).await;
    let email = format!("ui-identity-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_dashboard_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;
    set_account_plan_by_email(&pool, &email, "identity").await;

    let html = fetch_dashboard_html(app, &cookie).await;
    assert!(!html.contains("upgrade-plan-btn"));
    assert!(!html.contains("No upgrade options available"));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_ignores_pending_downgrade_for_upgrade_list() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_all_paid_plan_prices(&pool).await;
    let email = format!("ui-pending-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_dashboard_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;
    set_account_plan_by_email(&pool, &email, "legal").await;
    sqlx::query(
        r#"
        UPDATE accounts
        SET pending_tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = 'free')
        WHERE email = $1
        "#,
    )
    .bind(&email)
    .execute(&pool)
    .await
    .expect("pending downgrade");

    let html = fetch_dashboard_html(app, &cookie).await;
    assert!(html.contains(r#"data-plan="vault""#));
    assert!(html.contains(r#"data-plan="identity""#));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn dashboard_hides_plans_without_paddle_price() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_all_paid_plan_prices(&pool).await;
    let original_vault: Option<String> =
        sqlx::query_scalar("SELECT paddle_price_id FROM tariff_plans WHERE name = 'vault'")
            .fetch_one(&pool)
            .await
            .expect("vault price");
    sqlx::query("UPDATE tariff_plans SET paddle_price_id = NULL WHERE name = 'vault'")
        .execute(&pool)
        .await
        .expect("clear vault price");

    let email = format!("ui-no-vault-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let app = full_dashboard_app(state_with_paddle(pool.clone(), MockPaddleClient::new()));
    let cookie = register_and_login(&app, &email).await;

    let html = fetch_dashboard_html(app, &cookie).await;
    assert!(html.contains(r#"data-plan="legal""#));
    assert!(!html.contains(r#"data-plan="vault""#));
    assert!(html.contains(r#"data-plan="identity""#));

    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'vault'")
        .bind(original_vault)
        .execute(&pool)
        .await
        .expect("restore vault price");
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn ensure_paddle_customer_creates_and_persists_customer_id() {
    let pool = test_pool().await;
    let email = format!("bill-ensure-new-{}@example.com", Uuid::new_v4());
    let account_id = create_unlinked_account(&pool, &email).await;
    let paddle = MockPaddleClient::new();

    let customer_id = billing::ensure_paddle_customer(&pool, paddle.as_ref(), account_id, &email)
        .await
        .expect("ensure customer");

    assert!(customer_id.starts_with("ctm_mock_"));
    assert_eq!(paddle.create_customer_calls(), 1);

    let stored: Option<String> =
        sqlx::query_scalar("SELECT paddle_customer_id FROM accounts WHERE account_id = $1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("stored customer");
    assert_eq!(stored.as_deref(), Some(customer_id.as_str()));

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn ensure_paddle_customer_returns_existing_without_new_api_call() {
    let pool = test_pool().await;
    let email = format!("bill-ensure-existing-{}@example.com", Uuid::new_v4());
    let account_id = create_unlinked_account(&pool, &email).await;
    sqlx::query("UPDATE accounts SET paddle_customer_id = $2 WHERE account_id = $1")
        .bind(account_id)
        .bind("ctm_existing_123")
        .execute(&pool)
        .await
        .expect("seed customer");
    let paddle = MockPaddleClient::new();

    let customer_id = billing::ensure_paddle_customer(&pool, paddle.as_ref(), account_id, &email)
        .await
        .expect("ensure customer");

    assert_eq!(customer_id, "ctm_existing_123");
    assert_eq!(paddle.create_customer_calls(), 0);
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn paddle_customer_id_saved_before_checkout_redirect() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "legal", "pri_legal_test").await;
    let email = format!("bill-persist-{}@example.com", Uuid::new_v4());
    let account_id = create_unlinked_account(&pool, &email).await;
    let paddle = MockPaddleClient::new();

    billing::initiate_upgrade(&pool, paddle.as_ref(), account_id, &email, "legal")
        .await
        .expect("upgrade");

    let stored: Option<String> =
        sqlx::query_scalar("SELECT paddle_customer_id FROM accounts WHERE account_id = $1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("stored customer");
    assert!(stored.as_deref().unwrap_or("").starts_with("ctm_mock_"));
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn paddle_api_timeout_returns_bad_gateway() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "legal", "pri_legal_test").await;
    let email = format!("bill-timeout-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;
    let paddle = MockPaddleClient::new();
    paddle.set_simulate_timeout(true);
    let app = billing_app(state_with_paddle(pool.clone(), paddle));
    let cookie = register_and_login(&app, &email).await;

    let (status, body, _) = call(
        app,
        peer_request(
            "POST",
            "/dashboard/upgrade",
            Some(&cookie),
            Some(json!({ "plan": "legal" })),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"]["code"], "paddle_unavailable");

    let stored: Option<String> =
        sqlx::query_scalar("SELECT paddle_customer_id FROM accounts WHERE email = $1")
            .bind(&email)
            .fetch_one(&pool)
            .await
            .expect("customer id");
    assert!(stored.is_none());
    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn concurrent_upgrade_requests_create_single_paddle_customer() {
    let _lock = tariff_plan_test_lock();
    let pool = test_pool().await;
    set_plan_price(&pool, "legal", "pri_legal_test").await;
    let email = format!("bill-concurrent-{}@example.com", Uuid::new_v4());
    let account_id = create_unlinked_account(&pool, &email).await;
    let paddle = MockPaddleClient::new();
    paddle.set_create_delay_ms(100);

    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let paddle_a = paddle.clone();
    let paddle_b = paddle.clone();
    let email_a = email.clone();
    let email_b = email.clone();

    let (left, right) = tokio::join!(
        async move {
            billing::ensure_paddle_customer(&pool_a, paddle_a.as_ref(), account_id, &email_a).await
        },
        async move {
            billing::ensure_paddle_customer(&pool_b, paddle_b.as_ref(), account_id, &email_b).await
        }
    );

    left.expect("left ensure");
    right.expect("right ensure");
    assert_eq!(paddle.create_customer_calls(), 1);

    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM accounts
        WHERE account_id = $1 AND paddle_customer_id IS NOT NULL
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1);
    cleanup_email(&pool, &email).await;
}
