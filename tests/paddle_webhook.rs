//! Stage 8.2b — Paddle webhook processing tests.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Utc};
use evident_ledger::api::paddle_webhook;
use evident_ledger::paddle::sign_payload_for_test;
use evident_ledger::state::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use tower::util::ServiceExt;
use uuid::Uuid;

const WEBHOOK_SECRET: &str = common::TEST_PADDLE_WEBHOOK_SECRET;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

async fn setup_legal_price(pool: &sqlx::PgPool) {
    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'legal'")
        .bind("pri_legal_test")
        .execute(pool)
        .await
        .expect("legal price");
}

async fn create_account(pool: &sqlx::PgPool, customer_id: &str, subscription_status: &str) -> Uuid {
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, paddle_customer_id, subscription_status)
        VALUES ($1, $2, (SELECT plan_id FROM tariff_plans WHERE name = 'free'), $3, $4)
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@paddle.test"))
    .bind(customer_id)
    .bind(subscription_status)
    .execute(pool)
    .await
    .expect("insert account");
    account_id
}

fn event_payload(
    event_id: &str,
    event_type: &str,
    customer_id: &str,
    subscription_id: &str,
    period_end: &str,
) -> String {
    json!({
        "event_id": event_id,
        "event_type": event_type,
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": subscription_id,
            "customer_id": customer_id,
            "current_billing_period": { "ends_at": period_end },
            "items": [{ "price": { "id": "pri_legal_test" } }]
        }
    })
    .to_string()
}

fn past_due_payload(event_id: &str, customer_id: &str, subscription_id: &str) -> String {
    json!({
        "event_id": event_id,
        "event_type": "subscription.past_due",
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": subscription_id,
            "customer_id": customer_id
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

fn assert_period_end(actual: Option<DateTime<Utc>>, expected: &str) {
    let expected = DateTime::parse_from_rfc3339(expected)
        .expect("expected timestamp")
        .with_timezone(&Utc);
    assert_eq!(actual, Some(expected));
}

#[tokio::test]
async fn subscription_created_sets_active_and_plan() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload(
        &format!("evt_{}", Uuid::new_v4()),
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "processed");

    let (status, period_end) = account_row(&pool, account_id).await;
    assert_eq!(status, "active");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "legal");
    assert_period_end(period_end, "2026-08-18T10:00:00Z");
}

#[tokio::test]
async fn subscription_past_due_sets_past_due() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "active").await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = past_due_payload(
        &format!("evt_{}", Uuid::new_v4()),
        &customer_id,
        &subscription_id,
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "processed");
    assert_eq!(account_row(&pool, account_id).await.0, "past_due");
}

/// Renewal / recovery after `past_due` arrives as `subscription.updated` with a new
/// billing period (Paddle has no `subscription.payment_succeeded`).
#[tokio::test]
async fn subscription_updated_renewal_reactivates_and_extends_period() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "past_due").await;
    // Seed account on legal so updated same-plan branch runs (not upgrade/downgrade).
    sqlx::query(
        r#"
        UPDATE accounts
        SET tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = 'legal')
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("seed legal plan");

    let app = paddle_webhook::router(test_state(pool.clone()));
    let body = event_payload(
        &format!("evt_{}", Uuid::new_v4()),
        "subscription.updated",
        &customer_id,
        &subscription_id,
        "2026-09-18T10:00:00Z",
    );

    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "processed");
    let (status, period_end) = account_row(&pool, account_id).await;
    assert_eq!(status, "active");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "legal");
    assert_period_end(period_end, "2026-09-18T10:00:00Z");
}

#[tokio::test]
async fn unrecognized_event_type_is_ignored_with_200() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "active").await;
    let before = account_row(&pool, account_id).await;
    let event_id = format!("evt_{}", Uuid::new_v4());
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = json!({
        "event_id": event_id,
        "event_type": "customer.updated",
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": customer_id,
            "customer_id": customer_id
        }
    })
    .to_string();

    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ignored");
    assert_eq!(account_row(&pool, account_id).await, before);

    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("event count");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn subscription_canceled_preserves_period_end() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "active").await;
    sqlx::query("UPDATE accounts SET current_period_end = $2 WHERE account_id = $1")
        .bind(account_id)
        .bind(DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z").unwrap())
        .execute(&pool)
        .await
        .expect("seed period");

    let app = paddle_webhook::router(test_state(pool.clone()));
    let body = json!({
        "event_id": format!("evt_{}", Uuid::new_v4()),
        "event_type": "subscription.canceled",
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": subscription_id,
            "customer_id": customer_id,
            "current_billing_period": { "ends_at": "2026-08-01T00:00:00Z" }
        }
    })
    .to_string();

    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "processed");
    let (status, period_end) = account_row(&pool, account_id).await;
    assert_eq!(status, "canceled");
    assert_period_end(period_end, "2026-08-01T00:00:00Z");
}

#[tokio::test]
async fn duplicate_event_is_idempotent() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );

    let (status, _) = post_webhook(app.clone(), signed_request(&body)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, json) = post_webhook(app, signed_request(&body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "idempotent");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "legal");

    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("event count");
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn invalid_signature_rejected_without_db_changes() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let before = account_row(&pool, account_id).await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );
    let req = Request::builder()
        .method("POST")
        .uri("/webhook")
        .header("Paddle-Signature", "ts=1;h1=deadbeef")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("request");

    let (status, json) = post_webhook(app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["error"], "invalid_signature");
    assert_eq!(account_row(&pool, account_id).await, before);

    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("event count");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn unknown_customer_stored_as_waiting_for_account_link() {
    let pool = test_pool().await;
    let event_id = format!("evt_{}", Uuid::new_v4());
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        &format!("sub_{}", Uuid::new_v4()),
        "2026-08-18T10:00:00Z",
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "waiting_for_account_link");

    let webhook_status: String =
        sqlx::query_scalar("SELECT status FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("webhook status");
    assert_eq!(webhook_status, "waiting_for_account_link");

    let pending: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM paddle_pending_links
            WHERE paddle_customer_id = $1 AND resolved_at IS NULL
        )
        "#,
    )
    .bind(&customer_id)
    .fetch_one(&pool)
    .await
    .expect("pending link");
    assert!(pending);
}

#[tokio::test]
async fn conflicting_payload_hash_returns_conflict() {
    let pool = test_pool().await;
    setup_legal_price(&pool).await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body1 = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
    );
    let (status, _) = post_webhook(app.clone(), signed_request(&body1)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "legal");

    let body2 = event_payload(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-09-18T10:00:00Z",
    );
    let (status, json) = post_webhook(app, signed_request(&body2)).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["error"], "conflict");
    assert_period_end(
        account_row(&pool, account_id).await.1,
        "2026-08-18T10:00:00Z",
    );
}

// --- Stage 11.4: permanent vs temporary error classification ---

#[tokio::test]
async fn malformed_payload_is_permanent_bad_request() {
    let pool = test_pool().await;
    let app = paddle_webhook::router(test_state(pool));
    let body = "not-json";
    let (status, json) = post_webhook(app, signed_request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "invalid_payload");
}

#[tokio::test]
async fn missing_customer_id_is_permanent_bad_request() {
    let pool = test_pool().await;
    let app = paddle_webhook::router(test_state(pool));
    let body = json!({
        "event_id": format!("evt_{}", Uuid::new_v4()),
        "event_type": "subscription.created",
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": format!("sub_{}", Uuid::new_v4()),
            "items": [{ "price": { "id": "pri_legal_test" } }],
            "current_billing_period": { "ends_at": "2026-08-18T10:00:00Z" }
        }
    })
    .to_string();

    let (status, json) = post_webhook(app, signed_request(&body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "invalid_payload");
}

fn event_payload_with_price(
    event_id: &str,
    event_type: &str,
    customer_id: &str,
    subscription_id: &str,
    period_end: &str,
    price_id: &str,
) -> String {
    json!({
        "event_id": event_id,
        "event_type": event_type,
        "occurred_at": "2026-07-18T10:00:00Z",
        "data": {
            "id": subscription_id,
            "customer_id": customer_id,
            "current_billing_period": { "ends_at": period_end },
            "items": [{ "price": { "id": price_id } }]
        }
    })
    .to_string()
}

/// Unknown price mapping is temporary (config may sync); Paddle should retry (5xx).
#[tokio::test]
async fn plan_not_found_is_temporary_internal_error() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let missing_price = format!("pri_missing_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload_with_price(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
        &missing_price,
    );
    let (status, json) = post_webhook(app, signed_request(&body)).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(json["error"], "temporary_failure");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "free");

    let webhook_status: String =
        sqlx::query_scalar("SELECT status FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("webhook status");
    assert_eq!(webhook_status, "failed");
}

/// After a temporary failure (`failed`), a later delivery must re-enter processing.
/// Uses `vault` price mapping so parallel tests that touch `legal` do not race.
#[tokio::test]
async fn failed_webhook_retries_successfully_after_config_fix() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let price_id = format!("pri_retry_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let app = paddle_webhook::router(test_state(pool.clone()));

    let body = event_payload_with_price(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
        &price_id,
    );

    let (status, _) = post_webhook(app.clone(), signed_request(&body)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'vault'")
        .bind(&price_id)
        .execute(&pool)
        .await
        .expect("map price");

    let (status, json) = post_webhook(app, signed_request(&body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "processed");
    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "vault");

    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("event count");
    assert_eq!(rows, 1);

    let webhook_status: String =
        sqlx::query_scalar("SELECT status FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("webhook status");
    assert_eq!(webhook_status, "processed");
}

/// Concurrent retries of a `failed` event must not double-apply (conditional mark_processing).
/// Uses `identity` price mapping to avoid racing other tests on `legal`/`vault`.
#[tokio::test]
async fn concurrent_failed_retries_do_not_double_apply() {
    let pool = test_pool().await;
    let customer_id = format!("ctm_{}", Uuid::new_v4());
    let subscription_id = format!("sub_{}", Uuid::new_v4());
    let event_id = format!("evt_{}", Uuid::new_v4());
    let price_id = format!("pri_race_{}", Uuid::new_v4());
    let account_id = create_account(&pool, &customer_id, "none").await;
    let app = Arc::new(paddle_webhook::router(test_state(pool.clone())));

    let body = event_payload_with_price(
        &event_id,
        "subscription.created",
        &customer_id,
        &subscription_id,
        "2026-08-18T10:00:00Z",
        &price_id,
    );

    // First delivery fails (no price mapping) → status `failed`.
    let (status, _) = post_webhook((*app).clone(), signed_request(&body)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'identity'")
        .bind(&price_id)
        .execute(&pool)
        .await
        .expect("map price");

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let app_a = Arc::clone(&app);
    let app_b = Arc::clone(&app);
    let body_a = body.clone();
    let body_b = body.clone();
    let barrier_a = Arc::clone(&barrier);
    let barrier_b = Arc::clone(&barrier);

    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        post_webhook((*app_a).clone(), signed_request(&body_a)).await
    });
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        post_webhook((*app_b).clone(), signed_request(&body_b)).await
    });

    let (ra, rb) = tokio::join!(task_a, task_b);
    let (status_a, json_a) = ra.expect("task a");
    let (status_b, json_b) = rb.expect("task b");

    // One wins (processed); the other is idempotent or temporary — never a second apply.
    let statuses = [status_a, status_b];
    assert!(
        statuses.iter().any(|s| *s == StatusCode::OK),
        "expected at least one OK, got {status_a:?}/{status_b:?} bodies={json_a}/{json_b}"
    );
    for (status, json) in [(status_a, &json_a), (status_b, &json_b)] {
        if status == StatusCode::OK {
            assert!(
                json["status"] == "processed" || json["status"] == "idempotent",
                "unexpected ok body: {json}"
            );
        } else {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    assert_eq!(account_tariff_plan_name(&pool, account_id).await, "identity");
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM paddle_webhook_events WHERE paddle_event_id = $1")
            .bind(&event_id)
            .fetch_one(&pool)
            .await
            .expect("event count");
    assert_eq!(rows, 1);
}
