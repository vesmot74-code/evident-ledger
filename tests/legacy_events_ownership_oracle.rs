//! SEC-003: legacy POST /events ownership *error* disclosure.
//!
//! Closes distinguishable `ChainAccessDenied` (403 + ownership wording) vs
//! `ChainNotFound` responses on the legacy HTTP surface only.
//!
//! Auto-claim of unseen `chain_id` (`INSERT … ON CONFLICT DO NOTHING`) is
//! existing product behavior and is out of scope for SEC-003.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use evident_ledger::api::events::{self, map_legacy_events_error};
use evident_ledger::auth::api_key;
use evident_ledger::service::ledger::LedgerError;
use evident_ledger::state::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::util::ServiceExt;
use uuid::Uuid;

fn app(state: AppState) -> axum::Router {
    axum::Router::new().nest("/events", events::router(state))
}

struct TestAccount {
    api_key: String,
    chain_id: Uuid,
}

async fn plan_id(pool: &sqlx::PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("plan")
}

async fn create_test_account(pool: &sqlx::PgPool) -> TestAccount {
    let account_id = Uuid::new_v4();
    let plan = plan_id(pool, "free").await;
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, $3, 'none')
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@sec003-oracle.test"))
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

    let chain_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO chains (chain_id, head_event_id, account_id)
        VALUES ($1, NULL, $2)
        "#,
    )
    .bind(chain_id)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("chain");

    TestAccount {
        api_key: generated.full_key,
        chain_id,
    }
}

fn file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    (status, body)
}

async fn response_parts(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    (status, body)
}

fn post_events(api_key: &str, chain_id: Uuid, hash: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/events")
        .header("X-API-KEY", api_key)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "chain_id": chain_id,
                "file_hash": hash,
                "idempotency_key": Uuid::new_v4().to_string(),
            })
            .to_string(),
        ))
        .expect("request")
}

/// ChainNotFound is not normally reachable through a fresh chain_id on
/// POST /events because the endpoint auto-claims unseen chains.
/// This test verifies the defensive error mapping only:
/// if ChainNotFound is returned, it must be indistinguishable from
/// ChainAccessDenied.
#[tokio::test]
async fn chain_not_found_and_access_denied_have_same_legacy_error_response() {
    let (chain_not_found_status, chain_not_found_body) =
        response_parts(map_legacy_events_error(LedgerError::ChainNotFound)).await;
    let (chain_access_denied_status, chain_access_denied_body) =
        response_parts(map_legacy_events_error(LedgerError::ChainAccessDenied)).await;

    assert_eq!(chain_not_found_status, StatusCode::NOT_FOUND);
    assert_eq!(chain_access_denied_status, StatusCode::NOT_FOUND);
    assert_eq!(chain_not_found_status, chain_access_denied_status);
    assert_eq!(chain_not_found_body, chain_access_denied_body);
    assert_eq!(chain_not_found_body, json!({ "error": "not_found" }));
}

#[tokio::test]
async fn foreign_chain_http_returns_generic_not_found() {
    let pool = common::test_pool().await;
    let owner = create_test_account(&pool).await;
    let attacker = create_test_account(&pool).await;
    let state = common::test_app_state(pool);

    let (status, body) = call(
        app(state),
        post_events(
            &attacker.api_key,
            owner.chain_id,
            &file_hash(&format!("oracle-{}", Uuid::new_v4())),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "not_found" }));
    let serialized = body.to_string();
    assert!(!serialized.contains("different account"));
    assert!(!serialized.contains("Chain belongs"));
    assert!(!serialized.to_lowercase().contains("forbidden"));
}

#[tokio::test]
async fn shared_ledger_error_into_response_still_returns_403_for_access_denied() {
    // Shared LedgerError default path remains unchanged (not used by legacy /events handler).
    let legacy_default = LedgerError::ChainAccessDenied.into_response();
    assert_eq!(legacy_default.status(), StatusCode::FORBIDDEN);
}
