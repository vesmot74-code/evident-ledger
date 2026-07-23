//! P1 regression: legacy POST /events must reject identity fields.
//!
//! Identity commits remain supported only via POST /v1/events (PoP + feature gate).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use evident_ledger::api::{events, v1};
use evident_ledger::auth::api_key;
use evident_ledger::merkle::MerkleTree;
use evident_ledger::service::identity_keys::IdentityKeyRepository;
use evident_ledger::state::AppState;
use rand::rngs::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn app(state: AppState) -> axum::Router {
    axum::Router::new()
        .nest("/events", events::router(state.clone()))
        .nest("/v1", v1::router(state))
}

struct TestAccount {
    account_id: Uuid,
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

async fn enable_machine_tsa_for_plan(pool: &sqlx::PgPool, plan_name: &str) {
    sqlx::query("UPDATE tariff_plans SET tsa_mode = 'machine' WHERE name = $1")
        .bind(plan_name)
        .execute(pool)
        .await
        .expect("tsa mode");
}

async fn create_test_account(pool: &sqlx::PgPool, plan_name: &str) -> TestAccount {
    let account_id = Uuid::new_v4();
    let plan = plan_id(pool, plan_name).await;
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, $3, 'active')
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@legacy-identity-reject.test"))
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

async fn create_identity_key(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    signing_key: &SigningKey,
) -> Uuid {
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let fingerprint = IdentityKeyRepository::fingerprint_from_public_key_hex(&public_key_hex)
        .expect("fingerprint");
    let key = IdentityKeyRepository::create(
        pool,
        account_id,
        &public_key_hex,
        &fingerprint,
        Some("test-key"),
    )
    .await
    .expect("identity key");
    key.id
}

fn file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn sign_event_hash(signing_key: &SigningKey, canonical_hash_hex: &str) -> String {
    let raw = hex::decode(canonical_hash_hex).expect("hash hex");
    hex::encode(signing_key.sign(&raw).to_bytes())
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let parsed = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "_raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, parsed)
}

fn legacy_post(api_key: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/events")
        .header("X-API-KEY", api_key)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn v1_post(api_key: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("X-API-KEY", api_key)
        .header("Idempotency-Key", Uuid::new_v4().to_string())
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
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
    let _ = sqlx::query(
        "DELETE FROM events WHERE chain_id IN (SELECT chain_id FROM chains WHERE account_id = $1)",
    )
    .bind(account_id)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM identity_keys WHERE account_id = $1")
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

#[tokio::test]
async fn legacy_events_with_identity_fields_is_rejected() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free").await;
    let state = test_state(pool.clone());
    let hash = file_hash(&format!("reject-{}", Uuid::new_v4()));

    let cases = [
        json!({
            "chain_id": account.chain_id,
            "file_hash": hash,
            "idempotency_key": Uuid::new_v4().to_string(),
            "identity_key_id": Uuid::new_v4(),
        }),
        json!({
            "chain_id": account.chain_id,
            "file_hash": hash,
            "idempotency_key": Uuid::new_v4().to_string(),
            "identity_signature": "ab",
        }),
        json!({
            "chain_id": account.chain_id,
            "file_hash": hash,
            "idempotency_key": Uuid::new_v4().to_string(),
            "identity_fingerprint": "cd",
        }),
        json!({
            "chain_id": account.chain_id,
            "file_hash": hash,
            "idempotency_key": Uuid::new_v4().to_string(),
            "identity_key_id": Uuid::new_v4(),
            "identity_signature": "ab",
            "identity_fingerprint": "cd",
        }),
    ];

    for body in cases {
        let (status, resp) = call(app(state.clone()), legacy_post(&account.api_key, body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={resp}");
        assert_eq!(
            resp["error"].as_str(),
            Some("Identity fields are not supported on POST /events; use POST /v1/events"),
        );
    }

    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM events WHERE chain_id = $1",
    )
    .bind(account.chain_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(event_count, 0, "rejected requests must not insert events");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_events_without_identity_fields_unchanged() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "free").await;
    let state = test_state(pool.clone());
    let hash = file_hash(&format!("ok-{}", Uuid::new_v4()));

    let (status, body) = call(
        app(state),
        legacy_post(
            &account.api_key,
            json!({
                "chain_id": account.chain_id,
                "file_hash": hash,
                "idempotency_key": Uuid::new_v4().to_string(),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body["event_id"].as_str().is_some());
    assert_eq!(
        body["proof"]["signature"].as_str().map(|s| s.len()),
        Some(128)
    );

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn v1_events_with_valid_identity_fields_unchanged() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let event_id = Uuid::new_v4();
    let file_hash = file_hash("v1-identity-ok");
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(&signing_key, &canonical_hash);

    let (status, body) = call(
        app(test_state(pool.clone())),
        v1_post(
            &account.api_key,
            json!({
                "chain_id": account.chain_id,
                "file_hash": file_hash,
                "event_type": "submission",
                "event_id": event_id,
                "identity_signature": {
                    "key_id": key_id,
                    "signature": signature,
                },
            }),
        ),
    )
    .await;

    assert!(status.is_success(), "expected success, got {status} {body}");
    assert_eq!(body["event_id"], event_id.to_string());

    let stored_key_id: Option<Uuid> =
        sqlx::query_scalar("SELECT identity_key_id FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event row");
    assert_eq!(stored_key_id, Some(key_id));

    cleanup_account(&pool, account.account_id).await;
}
