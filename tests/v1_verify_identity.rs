//! Stage 9.4 — identity verification on GET /v1/verify/{event_id}.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use evident_ledger::api::v1;
use evident_ledger::auth::api_key;
use evident_ledger::merkle::MerkleTree;
use evident_ledger::service::identity_keys::IdentityKeyRepository;
use evident_ledger::state::AppState;
use rand::rngs::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("db");
    sqlx::migrate!().run(&pool).await.expect("migrate");
    pool
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn v1_app(state: AppState) -> axum::Router {
    v1::router(state)
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
    .bind(format!("{account_id}@verify-identity.test"))
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

async fn create_identity_key(pool: &sqlx::PgPool, account_id: Uuid, signing_key: &SigningKey) -> Uuid {
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let fingerprint = IdentityKeyRepository::fingerprint_from_public_key_hex(&public_key_hex)
        .expect("fingerprint");
    IdentityKeyRepository::create(
        pool,
        account_id,
        &public_key_hex,
        &fingerprint,
        Some("verify-key"),
    )
    .await
    .expect("identity key")
    .id
}

fn valid_file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn sign_event_hash(signing_key: &SigningKey, canonical_hash_hex: &str) -> String {
    let raw = hex::decode(canonical_hash_hex).expect("hash hex");
    hex::encode(signing_key.sign(&raw).to_bytes())
}

fn authed_request(method: &str, uri: &str, api_key: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    builder = builder.header("X-API-KEY", api_key);
    if let Some(json) = body {
        builder = builder
            .header("content-type", "application/json")
            .header("Idempotency-Key", format!("idem-{}", Uuid::new_v4()));
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

async fn post_event(
    app: axum::Router,
    account: &TestAccount,
    label: &str,
    event_id: Option<Uuid>,
    identity_signature: Option<Value>,
) -> (StatusCode, Value) {
    let mut body = json!({
        "chain_id": account.chain_id,
        "file_hash": valid_file_hash(label),
        "event_type": "submission",
    });
    if let Some(event_id) = event_id {
        body["event_id"] = json!(event_id);
    }
    if let Some(sig) = identity_signature {
        body["identity_signature"] = sig;
    }
    call(
        app,
        authed_request("POST", "/events", &account.api_key, Some(body)),
    )
    .await
}

async fn get_verify(app: axum::Router, api_key: &str, event_id: Uuid) -> (StatusCode, Value) {
    call(
        app,
        authed_request("GET", &format!("/verify/{event_id}"), api_key, None),
    )
    .await
}

async fn submit_signed_event(
    pool: &sqlx::PgPool,
    account: &TestAccount,
    label: &str,
    signing_key: &SigningKey,
    key_id: Uuid,
) -> Uuid {
    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash(label);
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(signing_key, &canonical_hash);
    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        account,
        label,
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": signature,
        })),
    )
    .await;
    assert!(status.is_success(), "submit failed: {status} {body}");
    event_id
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
async fn verify_without_identity_signature_returns_null() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let app = v1_app(test_state(pool.clone()));

    let (post_status, post_body) = post_event(app.clone(), &account, "no-identity", None, None).await;
    assert!(post_status.is_success(), "{post_status} {post_body}");
    let event_id = Uuid::parse_str(post_body["event_id"].as_str().unwrap()).unwrap();

    let (status, body) = get_verify(app, &account.api_key, event_id).await;
    assert!(status.is_success(), "{status} {body}");
    assert!(body["identity_signature"].is_null());
    assert!(body["chain"].is_object());
    assert!(body["file"].is_object());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn verify_with_valid_identity_signature_returns_valid_true() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id = submit_signed_event(&pool, &account, "valid-identity", &signing_key, key_id).await;

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = get_verify(app, &account.api_key, event_id).await;

    assert!(status.is_success(), "{status} {body}");
    assert_eq!(body["identity_signature"]["present"], true);
    assert_eq!(body["identity_signature"]["valid"], true);
    assert!(body["identity_signature"]["reason"].is_null());
    assert_eq!(body["identity_signature"]["key_id"], key_id.to_string());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn verify_with_tampered_signature_returns_signature_mismatch() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id =
        submit_signed_event(&pool, &account, "tampered-identity", &signing_key, key_id).await;

    sqlx::query("UPDATE events SET identity_signature = $1 WHERE event_id = $2")
        .bind("00".repeat(64))
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("tamper signature");

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = get_verify(app, &account.api_key, event_id).await;

    assert!(status.is_success(), "{status} {body}");
    assert_eq!(body["identity_signature"]["present"], true);
    assert_eq!(body["identity_signature"]["valid"], false);
    assert_eq!(body["identity_signature"]["reason"], "signature_mismatch");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn verify_historical_signature_stays_valid_after_key_revoked() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id =
        submit_signed_event(&pool, &account, "revoked-historical", &signing_key, key_id).await;

    IdentityKeyRepository::revoke(&pool, key_id, account.account_id)
        .await
        .expect("revoke key");

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = get_verify(app, &account.api_key, event_id).await;

    assert!(status.is_success(), "{status} {body}");
    assert_eq!(body["identity_signature"]["valid"], true);
    assert!(body["identity_signature"]["reason"].is_null());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn verify_historical_signature_stays_valid_for_unverified_key() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id =
        submit_signed_event(&pool, &account, "unverified-historical", &signing_key, key_id).await;

    sqlx::query("ALTER TABLE identity_keys ALTER COLUMN verified_at DROP NOT NULL")
        .execute(&pool)
        .await
        .expect("drop not null");
    sqlx::query("UPDATE identity_keys SET verified_at = NULL WHERE id = $1")
        .bind(key_id)
        .execute(&pool)
        .await
        .expect("clear verified_at");

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = get_verify(app, &account.api_key, event_id).await;
    assert!(status.is_success(), "{status} {body}");
    assert_eq!(body["identity_signature"]["valid"], true);
    assert!(body["identity_signature"]["reason"].is_null());

    let _ = sqlx::query(
        "ALTER TABLE identity_keys ALTER COLUMN verified_at SET NOT NULL",
    )
    .execute(&pool)
    .await;
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn verify_missing_identity_key_returns_key_not_found() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id =
        submit_signed_event(&pool, &account, "missing-key", &signing_key, key_id).await;

    let row = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, i64, Option<String>, Option<String>)>(
        r#"
        SELECT event_id, chain_id, parent_event_id, file_hash, sequence,
               identity_signature, identity_fingerprint
        FROM events
        WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("event row");

    let orphan_key_id = Uuid::new_v4();
    let identity_event = evident_ledger::models::event::Event {
        event_id: row.0,
        chain_id: row.1,
        parent_event_id: row.2,
        file_hash: row.3,
        sequence: row.4,
        identity_key_id: Some(orphan_key_id),
        identity_signature: row.5,
        identity_fingerprint: row.6,
    };
    let canonical_hash = MerkleTree::build_leaf(
        identity_event.sequence,
        &identity_event.event_id,
        &identity_event.parent_event_id,
        &identity_event.file_hash,
    );

    let result = evident_ledger::service::identity_verification::IdentityVerificationService::verify(
        &pool,
        &identity_event,
        &canonical_hash,
    )
    .await
    .expect("verify");

    assert!(result.present);
    assert!(!result.valid);
    assert_eq!(result.reason.as_deref(), Some("key_not_found"));
    assert_eq!(result.key_id, Some(orphan_key_id));

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn verify_existing_chain_and_file_contracts_unchanged() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let app = v1_app(test_state(pool.clone()));

    let file_hash = valid_file_hash("contract-check");
    let (post_status, post_body) = post_event(app.clone(), &account, "contract-check", None, None).await;
    assert!(post_status.is_success());
    let event_id = Uuid::parse_str(post_body["event_id"].as_str().unwrap()).unwrap();

    let (status, body) = get_verify(app.clone(), &account.api_key, event_id).await;
    assert!(status.is_success(), "{status} {body}");

    assert!(body.get("chain").and_then(|v| v.get("valid")).is_some());
    assert!(body.get("chain").and_then(|v| v.get("merkle_valid")).is_some());
    assert!(body.get("chain").and_then(|v| v.get("signature_valid")).is_some());
    assert!(body.get("file").and_then(|v| v.get("provided")).is_some());
    assert!(body.get("file").and_then(|v| v.get("is_valid_file_hash")).is_some());
    assert_eq!(body["proof_status"], "anchored");
    assert!(body["identity_signature"].is_null());

    let (status_with_hash, body_with_hash) = call(
        app,
        Request::builder()
            .method("GET")
            .uri(format!("/verify/{event_id}?file_hash={file_hash}"))
            .header("X-API-KEY", &account.api_key)
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert!(status_with_hash.is_success());
    assert_eq!(body_with_hash["file"]["is_valid_file_hash"], true);

    cleanup_account(&pool, account.account_id).await;
}
