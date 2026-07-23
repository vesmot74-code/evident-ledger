//! Stage 8.2c — subscription enforcement middleware tests.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use evident_ledger::api::v1;
use evident_ledger::auth::api_key;
use evident_ledger::state::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Barrier;
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn v1_app(state: AppState) -> axum::Router {
    v1::router(state)
}

fn legacy_write_app(state: AppState) -> axum::Router {
    use evident_ledger::api::{chains, events};

    axum::Router::new()
        .nest("/events", events::router(state.clone()))
        .nest("/chains", chains::router(state))
}

fn backup_app(state: AppState) -> axum::Router {
    use evident_ledger::api::backup;

    axum::Router::new().nest("/backup", backup::router(state))
}

async fn enable_machine_tsa_for_plan(pool: &sqlx::PgPool, plan_name: &str) {
    sqlx::query("UPDATE tariff_plans SET tsa_mode = 'machine' WHERE name = $1")
        .bind(plan_name)
        .execute(pool)
        .await
        .expect("tsa mode");
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
    chain_id: Uuid,
}

async fn create_test_account(
    pool: &sqlx::PgPool,
    plan_name: &str,
    subscription_status: &str,
) -> TestAccount {
    let account_id = Uuid::new_v4();
    let plan = plan_id(pool, plan_name).await;
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@sub.test"))
    .bind(plan)
    .bind(subscription_status)
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

    let chain_id = Uuid::new_v4();
    sqlx::query("INSERT INTO chains (chain_id, head_event_id, account_id) VALUES ($1, NULL, $2)")
        .bind(chain_id)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("chain");

    TestAccount {
        account_id,
        api_key: generated.full_key,
        chain_id,
    }
}

async fn set_usage_commits(pool: &sqlx::PgPool, account_id: Uuid, commits: i32) {
    sqlx::query(
        r#"
        INSERT INTO usage_monthly (account_id, period_start, server_commits)
        VALUES ($1, date_trunc('month', now())::date, $2)
        ON CONFLICT (account_id, period_start)
        DO UPDATE SET server_commits = EXCLUDED.server_commits
        "#,
    )
    .bind(account_id)
    .bind(commits)
    .execute(pool)
    .await
    .expect("usage");
}

async fn set_billing_fields(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    pending_plan: Option<&str>,
    period_end: Option<chrono::DateTime<Utc>>,
) {
    let pending_id = match pending_plan {
        Some(name) => Some(plan_id(pool, name).await),
        None => None,
    };
    sqlx::query(
        r#"
        UPDATE accounts
        SET pending_tariff_plan_id = $2, current_period_end = $3
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .bind(pending_id)
    .bind(period_end)
    .execute(pool)
    .await
    .expect("billing fields");
}

fn valid_file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn authed_request(method: &str, uri: &str, api_key: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    builder = builder.header("X-API-KEY", api_key);
    if let Some(json) = body {
        builder = builder.header("content-type", "application/json");
        builder.body(Body::from(json.to_string())).expect("request")
    } else {
        builder.body(Body::empty()).expect("request")
    }
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({ "raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, json)
}

async fn post_event(app: axum::Router, account: &TestAccount, label: &str) -> (StatusCode, Value) {
    let req = authed_request(
        "POST",
        "/events",
        &account.api_key,
        Some(json!({
            "chain_id": account.chain_id,
            "file_hash": valid_file_hash(label),
            "event_type": "submission",
        })),
    );
    let mut req = req;
    req.headers_mut()
        .insert("Idempotency-Key", format!("idem-{label}").parse().unwrap());
    call(app, req).await
}

/// Legacy `POST /events` uses `SubmitEventRequest` JSON (idempotency in body).
async fn post_legacy_event(
    app: axum::Router,
    account: &TestAccount,
    label: &str,
) -> (StatusCode, Value) {
    call(
        app,
        authed_request(
            "POST",
            "/events",
            &account.api_key,
            Some(json!({
                "chain_id": account.chain_id,
                "file_hash": valid_file_hash(label),
                "idempotency_key": format!("legacy-idem-{label}"),
                "parent_event_id": null,
            })),
        ),
    )
    .await
}

async fn post_legacy_chain(app: axum::Router, account: &TestAccount) -> (StatusCode, Value) {
    call(
        app,
        authed_request("POST", "/chains", &account.api_key, None),
    )
    .await
}

async fn account_plan_name(pool: &sqlx::PgPool, account_id: Uuid) -> String {
    sqlx::query_scalar(
        r#"
        SELECT tp.name
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("plan")
}

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query("DELETE FROM usage_monthly WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM idempotency_records WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM backups WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM events WHERE chain_id IN (SELECT chain_id FROM chains WHERE account_id = $1)",
    )
    .bind(account_id)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM chains WHERE account_id = $1")
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

async fn post_backup_create(
    app: axum::Router,
    account: &TestAccount,
) -> (StatusCode, Value) {
    call(
        app,
        authed_request(
            "POST",
            "/backup/create",
            &account.api_key,
            Some(json!({ "chain_id": account.chain_id })),
        ),
    )
    .await
}

#[tokio::test]
async fn free_plan_write_within_limit_passes() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free", "none").await;
    let app = v1_app(test_state(pool.clone()));
    let (status, _) = post_event(app, &account, "free-ok").await;
    assert!(status.is_success(), "expected success, got {status}");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn free_plan_write_over_limit_returns_usage_limit_exceeded() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free", "none").await;
    set_usage_commits(&pool, account.account_id, 100).await;
    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(app, &account, "free-limit").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "usage_limit_exceeded");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn paid_active_write_passes() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "legal").await;
    let account = create_test_account(&pool, "legal", "active").await;
    let app = v1_app(test_state(pool.clone()));
    let (status, _) = post_event(app, &account, "paid-active").await;
    assert!(status.is_success(), "expected success, got {status}");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn paid_past_due_write_returns_payment_required() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "past_due").await;
    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(app, &account, "paid-past-due").await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"]["code"], "payment_required");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn paid_past_due_read_passes() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "past_due").await;
    let app = v1_app(test_state(pool.clone()));
    let req = authed_request("GET", "/account/capabilities", &account.api_key, None);
    let (status, body) = call(app, req).await;
    assert_ne!(status, StatusCode::PAYMENT_REQUIRED);
    assert_ne!(body["error"]["code"], "payment_required");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn paid_canceled_before_period_end_write_passes() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "legal").await;
    let account = create_test_account(&pool, "legal", "canceled").await;
    set_billing_fields(
        &pool,
        account.account_id,
        None,
        Some(Utc::now() + Duration::days(7)),
    )
    .await;
    let app = v1_app(test_state(pool.clone()));
    let (status, _) = post_event(app, &account, "canceled-active").await;
    assert!(status.is_success(), "expected success, got {status}");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn lazy_evaluation_applies_pending_downgrade() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "active").await;
    set_billing_fields(
        &pool,
        account.account_id,
        Some("free"),
        Some(Utc::now() - Duration::hours(1)),
    )
    .await;

    let app = v1_app(test_state(pool.clone()));
    let req = authed_request("GET", "/account/capabilities", &account.api_key, None);
    let _ = call(app, req).await;

    assert_eq!(account_plan_name(&pool, account.account_id).await, "free");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn lazy_evaluation_canceled_after_period_end_moves_to_free_none() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "canceled").await;
    set_billing_fields(
        &pool,
        account.account_id,
        None,
        Some(Utc::now() - Duration::hours(1)),
    )
    .await;

    let app = v1_app(test_state(pool.clone()));
    let req = authed_request("GET", "/account/capabilities", &account.api_key, None);
    let _ = call(app, req).await;

    assert_eq!(account_plan_name(&pool, account.account_id).await, "free");
    let status: String =
        sqlx::query_scalar("SELECT subscription_status FROM accounts WHERE account_id = $1")
            .bind(account.account_id)
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "none");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn concurrent_lazy_evaluation_applies_transition_once() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "active").await;
    set_billing_fields(
        &pool,
        account.account_id,
        Some("free"),
        Some(Utc::now() - Duration::hours(1)),
    )
    .await;

    let app = Arc::new(v1_app(test_state(pool.clone())));
    let barrier = Arc::new(Barrier::new(2));

    let app_a = Arc::clone(&app);
    let app_b = Arc::clone(&app);
    let key_a = account.api_key.clone();
    let key_b = account.api_key.clone();
    let barrier_a = Arc::clone(&barrier);
    let barrier_b = Arc::clone(&barrier);

    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        let req = authed_request("GET", "/account/capabilities", &key_a, None);
        call((*app_a).clone(), req).await
    });
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        let req = authed_request("GET", "/account/capabilities", &key_b, None);
        call((*app_b).clone(), req).await
    });

    let (result_a, result_b) = tokio::join!(task_a, task_b);
    let _ = result_a.expect("task a");
    let _ = result_b.expect("task b");

    assert_eq!(account_plan_name(&pool, account.account_id).await, "free");
    let pending: Option<Uuid> =
        sqlx::query_scalar("SELECT pending_tariff_plan_id FROM accounts WHERE account_id = $1")
            .bind(account.account_id)
            .fetch_one(&pool)
            .await
            .expect("pending");
    assert!(pending.is_none());
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn identity_plan_null_limit_skips_usage_check() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "active").await;
    set_usage_commits(&pool, account.account_id, 1_000_000).await;
    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(app, &account, "identity-unlimited").await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_ne!(body["error"]["code"], "usage_limit_exceeded");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn free_plan_read_passes_when_usage_limit_exceeded() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free", "none").await;
    set_usage_commits(&pool, account.account_id, 100).await;
    let app = v1_app(test_state(pool.clone()));
    let req = authed_request("GET", "/account/capabilities", &account.api_key, None);
    let (status, body) = call(app, req).await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_ne!(body["error"]["code"], "usage_limit_exceeded");
    cleanup_account(&pool, account.account_id).await;
}

// --- Stage 11.3: legacy `/events` and `/chains` use the same subscription guard ---

#[tokio::test]
async fn legacy_events_paid_active_write_passes() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "legal").await;
    let account = create_test_account(&pool, "legal", "active").await;
    let app = legacy_write_app(test_state(pool.clone()));
    let (status, _) = post_legacy_event(app, &account, "legacy-paid-active").await;
    assert!(status.is_success(), "expected success, got {status}");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_events_past_due_matches_v1_payment_required() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "past_due").await;
    let state = test_state(pool.clone());

    let (v1_status, v1_body) = post_event(v1_app(state.clone()), &account, "v1-past-due").await;
    let (legacy_status, legacy_body) =
        post_legacy_event(legacy_write_app(state), &account, "legacy-past-due").await;

    assert_eq!(v1_status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(v1_body["error"]["code"], "payment_required");
    assert_eq!(legacy_status, v1_status);
    assert_eq!(legacy_body["error"]["code"], v1_body["error"]["code"]);
    assert_eq!(legacy_body["error"]["message"], v1_body["error"]["message"]);
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_events_free_account_write_allowed() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free", "none").await;
    let app = legacy_write_app(test_state(pool.clone()));
    let (status, _) = post_legacy_event(app, &account, "legacy-free-ok").await;
    assert!(status.is_success(), "expected success, got {status}");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_events_canceled_before_period_end_write_passes() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "legal").await;
    let account = create_test_account(&pool, "legal", "canceled").await;
    set_billing_fields(
        &pool,
        account.account_id,
        None,
        Some(Utc::now() + Duration::days(7)),
    )
    .await;
    let app = legacy_write_app(test_state(pool.clone()));
    let (status, _) = post_legacy_event(app, &account, "legacy-canceled-active").await;
    assert!(status.is_success(), "expected success, got {status}");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_chains_past_due_returns_payment_required() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "past_due").await;
    let app = legacy_write_app(test_state(pool.clone()));
    let (status, body) = post_legacy_chain(app, &account).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"]["code"], "payment_required");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_chains_active_write_passes() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "legal", "active").await;
    let app = legacy_write_app(test_state(pool.clone()));
    let (status, body) = post_legacy_chain(app, &account).await;
    assert!(status.is_success(), "expected success, got {status}");
    assert!(body.get("chain_id").is_some(), "body={body}");
    cleanup_account(&pool, account.account_id).await;
}

// --- Stage 11.5: `/backup` uses the same subscription guard ---

#[tokio::test]
async fn backup_create_active_subscription_passes() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "vault", "active").await;
    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("EVIDENT_BACKUP_DIR", tmp.path());
    }
    let app = backup_app(test_state(pool.clone()));
    let (status, body) = post_backup_create(app, &account).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["status"], "created");
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn backup_create_past_due_matches_v1_payment_required() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "vault", "past_due").await;
    let state = test_state(pool.clone());

    let (v1_status, v1_body) = post_event(v1_app(state.clone()), &account, "v1-vs-backup").await;
    let (backup_status, backup_body) = post_backup_create(backup_app(state), &account).await;

    assert_eq!(v1_status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(v1_body["error"]["code"], "payment_required");
    assert_eq!(backup_status, v1_status);
    assert_eq!(backup_body["error"]["code"], v1_body["error"]["code"]);
    assert_eq!(backup_body["error"]["message"], v1_body["error"]["message"]);

    let backup_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM backups WHERE account_id = $1")
            .bind(account.account_id)
            .fetch_one(&pool)
            .await
            .expect("backup count");
    assert_eq!(backup_rows, 0, "backup handler must not run on past_due");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn backup_create_free_account_keeps_entitlement_behavior() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free", "none").await;
    let app = backup_app(test_state(pool.clone()));
    let (status, body) = post_backup_create(app, &account).await;
    // Free has server_backup=false — entitlement rejection, not payment_required.
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "feature_not_available");
    assert_eq!(body["feature"], "server_backup");
    cleanup_account(&pool, account.account_id).await;
}
