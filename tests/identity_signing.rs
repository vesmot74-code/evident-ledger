//! Stage 9.3 — optional user identity signatures on events.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use evident_ledger::api::v1;
use evident_ledger::auth::api_key;
use evident_ledger::merkle::MerkleTree;
use evident_ledger::service::identity_keys::IdentityKeyRepository;
use evident_ledger::signing::verify_root;
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
    .bind(format!("{account_id}@identity-sign.test"))
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

fn valid_file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn sign_event_hash(signing_key: &SigningKey, canonical_hash_hex: &str) -> String {
    let raw = hex::decode(canonical_hash_hex).expect("hash hex");
    hex::encode(signing_key.sign(&raw).to_bytes())
}

fn authed_request(method: &str, uri: &str, api_key: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("X-API-KEY", api_key)
        .header("content-type", "application/json")
        .header("Idempotency-Key", format!("idem-{}", Uuid::new_v4()))
        .body(Body::from(body.to_string()))
        .expect("request")
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
        authed_request("POST", "/events", &account.api_key, body),
    )
    .await
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
async fn event_without_identity_signature_succeeds() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let app = v1_app(test_state(pool.clone()));

    let (status, body) = post_event(app, &account, "no-identity", None, None).await;

    assert!(status.is_success(), "expected success, got {status} {body}");
    let event_id = Uuid::parse_str(body["event_id"].as_str().unwrap()).unwrap();
    let identity_key_id: Option<Uuid> =
        sqlx::query_scalar("SELECT identity_key_id FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event row");
    let identity_signature: Option<String> =
        sqlx::query_scalar("SELECT identity_signature FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event row");
    let identity_fingerprint: Option<String> =
        sqlx::query_scalar("SELECT identity_fingerprint FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event row");
    assert!(identity_key_id.is_none());
    assert!(identity_signature.is_none());
    assert!(identity_fingerprint.is_none());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn event_with_valid_identity_signature_persists_fields() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash("with-identity");
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(&signing_key, &canonical_hash);

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        &account,
        "with-identity",
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": signature,
        })),
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
    let stored_signature: Option<String> =
        sqlx::query_scalar("SELECT identity_signature FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event row");
    let stored_fingerprint: Option<String> =
        sqlx::query_scalar("SELECT identity_fingerprint FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("event row");
    assert_eq!(stored_key_id, Some(key_id));
    assert_eq!(stored_signature.as_deref(), Some(signature.as_str()));
    assert!(stored_fingerprint.is_some());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn event_with_invalid_identity_signature_returns_unauthorized() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id = Uuid::new_v4();

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        &account,
        "bad-signature",
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": "00".repeat(64),
        })),
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "invalid_identity_signature");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn event_with_revoked_key_returns_forbidden() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    IdentityKeyRepository::revoke(&pool, key_id, account.account_id)
        .await
        .expect("revoke");

    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash("revoked-key");
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(&signing_key, &canonical_hash);

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        &account,
        "revoked-key",
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": signature,
        })),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "identity_key_revoked");

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn event_with_unverified_key_returns_forbidden() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let fingerprint = IdentityKeyRepository::fingerprint_from_public_key_hex(&public_key_hex)
        .expect("fingerprint");
    let key_id = Uuid::new_v4();

    sqlx::query("ALTER TABLE identity_keys ALTER COLUMN verified_at DROP NOT NULL")
        .execute(&pool)
        .await
        .expect("drop not null");

    sqlx::query(
        r#"
        INSERT INTO identity_keys (id, account_id, public_key, fingerprint, label, verified_at)
        VALUES ($1, $2, $3, $4, 'unverified', NULL)
        "#,
    )
    .bind(key_id)
    .bind(account.account_id)
    .bind(&public_key_hex)
    .bind(&fingerprint)
    .execute(&pool)
    .await
    .expect("insert unverified key");

    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash("unverified-key");
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(&signing_key, &canonical_hash);

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        &account,
        "unverified-key",
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": signature,
        })),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "identity_key_not_verified");

    let _ = sqlx::query("DELETE FROM identity_keys WHERE id = $1")
        .bind(key_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE identity_keys ALTER COLUMN verified_at SET NOT NULL")
        .execute(&pool)
        .await;

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn event_with_foreign_key_returns_not_found() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let owner = create_test_account(&pool, "identity").await;
    let other = create_test_account(&pool, "identity").await;
    let owner_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, owner.account_id, &owner_key).await;

    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash("foreign-key");
    let canonical_hash = MerkleTree::build_leaf(1, &event_id, &Uuid::nil(), &file_hash);
    let signature = sign_event_hash(&owner_key, &canonical_hash);

    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        &other,
        "foreign-key",
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": signature,
        })),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "identity_key_not_found");

    cleanup_account(&pool, owner.account_id).await;
    cleanup_account(&pool, other.account_id).await;
}

#[tokio::test]
async fn event_with_identity_signature_without_entitlement_returns_forbidden() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "free").await;
    let account = create_test_account(&pool, "free").await;
    let signing_key = SigningKey::generate(&mut OsRng);

    sqlx::query("ALTER TABLE identity_keys ALTER COLUMN verified_at DROP NOT NULL")
        .execute(&pool)
        .await
        .ok();
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let fingerprint = IdentityKeyRepository::fingerprint_from_public_key_hex(&public_key_hex)
        .expect("fingerprint");
    let key_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO identity_keys (id, account_id, public_key, fingerprint, label, verified_at)
        VALUES ($1, $2, $3, $4, 'free-plan-key', now())
        "#,
    )
    .bind(key_id)
    .bind(account.account_id)
    .bind(&public_key_hex)
    .bind(&fingerprint)
    .execute(&pool)
    .await
    .expect("insert key on free plan");

    let event_id = Uuid::new_v4();
    let app = v1_app(test_state(pool.clone()));
    let (status, body) = post_event(
        app,
        &account,
        "no-entitlement",
        Some(event_id),
        Some(json!({
            "key_id": key_id,
            "signature": "00".repeat(64),
        })),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "entitlement_missing");

    let _ = sqlx::query("DELETE FROM identity_keys WHERE id = $1")
        .bind(key_id)
        .execute(&pool)
        .await;
    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn legacy_event_without_identity_remains_server_verifiable() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity").await;
    let app = v1_app(test_state(pool.clone()));

    let (status, body) = post_event(app, &account, "legacy-verify", None, None).await;
    assert!(status.is_success(), "expected success, got {status} {body}");

    let event_id = Uuid::parse_str(body["event_id"].as_str().unwrap()).unwrap();

    #[derive(sqlx::FromRow)]
    struct LegacyEventRow {
        signature: String,
        file_hash: String,
        sequence: i64,
        parent_event_id: Uuid,
        chain_id: Uuid,
    }

    let row = sqlx::query_as::<_, LegacyEventRow>(
        r#"
        SELECT e.signature, e.file_hash, e.sequence, e.parent_event_id, c.chain_id
        FROM events e
        INNER JOIN chains c ON c.chain_id = e.chain_id
        WHERE e.event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("event");

    assert!(!row.signature.is_empty());

    let events = sqlx::query_as!(
        evident_ledger::db::EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        row.chain_id
    )
    .fetch_all(&pool)
    .await
    .expect("events");

    let root = evident_ledger::merkle::MerkleTree::recompute_root_from_events(&events);
    let public_key = common::test_app_state(pool.clone()).signer.public_key_hex();

    assert!(verify_root(
        &row.chain_id.to_string(),
        &root,
        &event_id.to_string(),
        &row.signature,
        &public_key,
    ));

    cleanup_account(&pool, account.account_id).await;
}
