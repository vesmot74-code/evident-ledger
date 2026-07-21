//! Stage 9.6 — identity key revocation API tests.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use evident_ledger::api::v1;
use evident_ledger::auth::api_key;
use evident_ledger::merkle::MerkleTree;
use evident_ledger::models::identity_key::IdentityKey;
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

fn v1_app(state: AppState) -> axum::Router {
    v1::router(state)
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
    IdentityKeyRepository::create(
        pool,
        account_id,
        &public_key_hex,
        &fingerprint,
        Some("revoke-test"),
    )
    .await
    .expect("identity key")
    .id
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

fn revoke_request(api_key: Option<&str>, key_id: Uuid) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/identity/keys/{key_id}/revoke"));
    if let Some(api_key) = api_key {
        builder = builder.header("X-API-KEY", api_key);
    }
    builder.body(Body::empty()).expect("request")
}

async fn post_revoke(
    app: axum::Router,
    api_key: Option<&str>,
    key_id: Uuid,
) -> (StatusCode, Value) {
    call(app, revoke_request(api_key, key_id)).await
}

async fn fetch_key(pool: &sqlx::PgPool, key_id: Uuid) -> Option<IdentityKey> {
    IdentityKeyRepository::find_by_id(pool, key_id)
        .await
        .expect("fetch key")
}

async fn audit_count(pool: &sqlx::PgPool, key_id: Uuid, action: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_key_audit_events WHERE key_id = $1 AND action = $2",
    )
    .bind(key_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("audit count")
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
    let _ = sqlx::query("DELETE FROM chains WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
}

fn valid_file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn sign_event_hash(signing_key: &SigningKey, canonical_hash_hex: &str) -> String {
    let raw = hex::decode(canonical_hash_hex).expect("hash hex");
    hex::encode(signing_key.sign(&raw).to_bytes())
}

async fn post_event_with_identity(
    app: axum::Router,
    account: &TestAccount,
    label: &str,
    key_id: Uuid,
    signature: &str,
) -> (StatusCode, Value) {
    let event_id = Uuid::new_v4();
    let body = json!({
        "chain_id": account.chain_id,
        "file_hash": valid_file_hash(label),
        "event_type": "submission",
        "event_id": event_id,
        "identity_signature": {
            "key_id": key_id,
            "signature": signature,
        }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/events")
        .header("X-API-KEY", &account.api_key)
        .header("content-type", "application/json")
        .header("Idempotency-Key", format!("idem-{}", Uuid::new_v4()))
        .body(Body::from(body.to_string()))
        .expect("request");
    call(app, req).await
}

#[tokio::test]
async fn revoke_own_active_key_returns_ok_and_audit_entry() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "revoke-ok").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_revoke(app, Some(&account.api_key), key_id).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["key_id"], json!(key_id));
    assert_eq!(body["status"], "revoked");
    assert!(body["revoked_at"].is_string());

    let key = fetch_key(&pool, key_id).await.expect("key");
    assert!(key.revoked_at.is_some());
    assert_eq!(audit_count(&pool, key_id, "revoked").await, 1);

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn revoke_foreign_key_returns_not_found() {
    let pool = test_pool().await;
    let owner = create_test_account(&pool, "revoke-owner").await;
    let other = create_test_account(&pool, "revoke-other").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, owner.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_revoke(app, Some(&other.api_key), key_id).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "identity_key_not_found");

    cleanup_account(&pool, owner.account_id).await;
    cleanup_account(&pool, other.account_id).await;
}

#[tokio::test]
async fn revoke_already_revoked_key_returns_conflict() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "revoke-twice").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let (first_status, _) = post_revoke(app.clone(), Some(&account.api_key), key_id).await;
    assert_eq!(first_status, StatusCode::OK);

    let (status, body) = post_revoke(app, Some(&account.api_key), key_id).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "identity_key_already_revoked");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn revoke_missing_key_returns_not_found() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "revoke-missing").await;
    let missing_key_id = Uuid::new_v4();

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_revoke(app, Some(&account.api_key), missing_key_id).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "identity_key_not_found");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn revoke_without_api_key_returns_unauthorized() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "revoke-unauth").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_revoke(app, None, key_id).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn revoke_rolls_back_when_audit_insert_fails() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "revoke-tx").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    {
        let mut tx = pool.begin().await.expect("begin");
        let updated = sqlx::query(
            "UPDATE identity_keys SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(key_id)
        .execute(&mut *tx)
        .await
        .expect("update");
        assert_eq!(updated.rows_affected(), 1);

        let audit_err = sqlx::query(
            r#"
            INSERT INTO identity_key_audit_events (key_id, actor_type, actor_id, action)
            VALUES ($1, 'account', $2, 'not_a_valid_action')
            "#,
        )
        .bind(key_id)
        .bind(account.account_id)
        .execute(&mut *tx)
        .await;
        assert!(audit_err.is_err());
    }

    let key = fetch_key(&pool, key_id).await.expect("key");
    assert!(key.revoked_at.is_none());
    assert_eq!(audit_count(&pool, key_id, "revoked").await, 0);

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn signing_with_revoked_key_after_api_revoke_is_rejected() {
    let pool = test_pool().await;
    sqlx::query("UPDATE tariff_plans SET tsa_mode = 'machine' WHERE name = 'identity'")
        .execute(&pool)
        .await
        .expect("tsa mode");

    let account = create_test_account(&pool, "revoke-sign").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let (revoke_status, _) = post_revoke(app.clone(), Some(&account.api_key), key_id).await;
    assert_eq!(revoke_status, StatusCode::OK);

    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash("post-revoke-sign");
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(&signing_key, &canonical_hash);

    let (status, body) =
        post_event_with_identity(app, &account, "post-revoke-sign", key_id, &signature).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "identity_key_revoked");

    cleanup_account(&pool, account.account_id).await;
}
