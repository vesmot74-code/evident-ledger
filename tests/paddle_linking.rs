//! Stage 8.2d — Paddle account linking hardening tests.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Utc};
use evident_ledger::api::paddle_webhook;
use evident_ledger::paddle::{
    link_paddle_customer_to_account, sign_payload_for_test, LinkCustomerError,
};
use evident_ledger::state::AppState;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use std::sync::Arc;
use tower::util::ServiceExt;
use uuid::Uuid;

const WEBHOOK_SECRET: &str = common::TEST_PADDLE_WEBHOOK_SECRET;

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
    AppState {
        db: pool,
        signer: Arc::new(
            evident_ledger::signing::ServerSigner::load_or_create("signing_key.bin"),
        ),
        config: {
            common::setup_test_env();
            evident_ledger::config::AppConfig::from_env()
        },
    }
}

async fn setup_legal_price(pool: &sqlx::PgPool) {
    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'legal'")
        .bind("pri_legal_test")
        .execute(pool)
        .await
        .expect("legal price");
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

fn event_payload(
    event_id: &str,
    event_type: &str,
    customer_id: &str,
    customer_email: Option<&str>,
    subscription_id: &str,
    period_end: &str,
) -> String {
    let customer = match customer_email {
        Some(email) => json!({ "id": customer_id, "email": email }),
        None => json!({ "id": customer_id }),
    };
    json!({
        "event_id": event_id,
        "event_type": event_type,
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": subscription_id,
            "customer_id": customer_id,
            "customer": customer,
            "current_billing_period": { "ends_at": period_end },
            "items": [{ "price": { "id": "pri_legal_test" } }]
        }
    })
    .to_string()
}

fn signed_request(body: &str) -> Request<Body> {
    let signature = sign_payload_for_test(WEBHOOK_SECRET, body, 1_700_000_000);
    Request::builder()
        .method("POST")
        .uri("/webhook")
        .header("Paddle-Signature", signature)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn post_webhook(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
    (status, json)
}

async fn account_row(pool: &sqlx::PgPool, account_id: Uuid) -> (String, Option<DateTime<Utc>>) {
    let row = sqlx::query(
        r#"
        SELECT subscription_status, current_period_end
        FROM accounts WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("account");
    (
        row.get("subscription_status"),
        row.get("current_period_end"),
    )
}

async fn account_tariff_plan_name(pool: &sqlx::PgPool, account_id: Uuid) -> String {
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
    .expect("plan name")
}

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query("DELETE FROM paddle_pending_links WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM paddle_webhook_events WHERE account_id = $1",
    )
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

#[tokio::test]
async fn proactive_linking_before_webhook_activates_subscription() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let email = format!("link-proactive-{}@example.com", Uuid::new_v4());
    let account_id = create_unlinked_account(&pool, &email).await;

    link_paddle_customer_to_account(&pool, account_id, &customer_id)
        .await
        .expect("proactive link");

    let app = paddle_webhook::router(test_state(pool.clone()));
    let body = event_payload(
        &format!("evt_{}", Uuid::new_v4()),
        "subscription.created",
        &customer_id,
        Some(&email),
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "processed");
    assert_eq!(account_row(&pool, account_id).await.0, "active");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "legal");
    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn webhook_before_linking_with_unverified_email_waits_for_account_link() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let email = format!("link-wait-{}@example.com", Uuid::new_v4());
    let account_id = create_unlinked_account(&pool, &email).await;

    let app = paddle_webhook::router(test_state(pool.clone()));
    let body = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        Some(&email),
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "waiting_for_account_link");

    let webhook_status: String = sqlx::query_scalar(
        "SELECT status FROM paddle_webhook_events WHERE paddle_event_id = $1",
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .expect("webhook status");
    assert_eq!(webhook_status, "waiting_for_account_link");

    assert_eq!(account_row(&pool, account_id).await.0, "none");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "free");

    let verified: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT email_verified_at FROM accounts WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("verified");
    assert!(verified.is_none());

    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn duplicate_waiting_webhook_is_idempotent() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        Some("unknown@example.com"),
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );

    let (status, json) = post_webhook(app.clone(), signed_request(&body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "waiting_for_account_link");

    let (status, json) = post_webhook(app, signed_request(&body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "idempotent");

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM paddle_webhook_events WHERE paddle_event_id = $1",
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .expect("event count");
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn linking_rejects_customer_already_bound_to_other_account() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let email_a = format!("link-owner-{}@example.com", Uuid::new_v4());
    let email_b = format!("link-other-{}@example.com", Uuid::new_v4());
    let account_a = create_unlinked_account(&pool, &email_a).await;
    let account_b = create_unlinked_account(&pool, &email_b).await;

    link_paddle_customer_to_account(&pool, account_a, &customer_id)
        .await
        .expect("first link");

    let err = link_paddle_customer_to_account(&pool, account_b, &customer_id)
        .await
        .expect_err("second link must fail");
    assert_eq!(
        err,
        LinkCustomerError::CustomerAlreadyLinkedToOtherAccount
    );

    cleanup_account(&pool, account_a).await;
    cleanup_account(&pool, account_b).await;
}

#[tokio::test]
async fn webhook_with_unknown_email_waits_for_account_link() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        Some("nobody@nowhere.example"),
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "waiting_for_account_link");

    let stored_email: String = sqlx::query_scalar(
        r#"
        SELECT paddle_email
        FROM paddle_pending_links
        WHERE paddle_customer_id = $1 AND resolved_at IS NULL
        "#,
    )
    .bind(&customer_id)
    .fetch_one(&pool)
    .await
    .expect("pending email");
    assert_eq!(stored_email, "nobody@nowhere.example");
}
