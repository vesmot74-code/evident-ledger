//! Regression: signature persistence parity between legacy POST /events and POST /v1/events.
//!
//! Guards the Stage 12 critical gap where CLI (/events) returned a signature in JSON
//! but left `events.signature` empty (bb43af7 only wired persist on the v1 path).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use evident_ledger::api::{chains, events, v1};
use evident_ledger::auth::api_key;
use evident_ledger::state::AppState;
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

/// Combined router so tests exercise real HTTP surfaces for both write and proof read.
fn app(state: AppState) -> axum::Router {
    axum::Router::new()
        .nest("/events", events::router(state.clone()))
        .nest("/chains", chains::router(state.clone()))
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
    .bind(format!("{account_id}@sig-persist.test"))
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
        account_id,
        api_key: generated.full_key,
        chain_id,
    }
}

fn file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

async fn call(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let mut svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let parsed = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({ "_raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, parsed)
}

fn authed_json(method: &str, uri: &str, api_key: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("X-API-KEY", api_key)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn authed_get(uri: &str, api_key: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("X-API-KEY", api_key)
        .body(Body::empty())
        .expect("request")
}

async fn db_signature(pool: &sqlx::PgPool, event_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT signature FROM events WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("signature")
}

async fn materialization_count(pool: &sqlx::PgPool, event_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM public_proof_materialization WHERE internal_proof_id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("materialization count")
}

async fn registry_public_id(pool: &sqlx::PgPool, file_hash: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"
        SELECT public_proof_id
        FROM public_proof_registry
        WHERE file_hash = $1 AND enabled = true
        LIMIT 1
        "#,
    )
    .bind(file_hash)
    .fetch_optional(pool)
    .await
    .expect("registry")
}

#[tokio::test]
async fn legacy_events_persists_response_signature_exactly() {
    let pool = test_pool().await;
    let account = create_test_account(&pool).await;
    let state = test_state(pool.clone());
    let hash = file_hash(&format!("legacy-{}", Uuid::new_v4()));

    let (status, body) = call(
        app(state),
        authed_json(
            "POST",
            "/events",
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
    let event_id = Uuid::parse_str(body["event_id"].as_str().expect("event_id")).unwrap();
    let response_sig = body["proof"]["signature"]
        .as_str()
        .expect("response signature")
        .to_string();
    assert_eq!(response_sig.len(), 128, "ed25519 signature hex length");

    let db_sig = db_signature(&pool, event_id).await;
    assert_eq!(
        db_sig, response_sig,
        "DB signature must equal legacy /events response signature byte-for-byte"
    );
}

#[tokio::test]
async fn v1_events_persists_response_signature_exactly() {
    let pool = test_pool().await;
    let account = create_test_account(&pool).await;
    let state = test_state(pool.clone());
    let hash = file_hash(&format!("v1-{}", Uuid::new_v4()));

    let (status, body) = call(
        app(state.clone()),
        Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("X-API-KEY", &account.api_key)
            .header("Idempotency-Key", Uuid::new_v4().to_string())
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "chain_id": account.chain_id,
                    "file_hash": hash,
                    "event_type": "submission",
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let event_id = Uuid::parse_str(body["event_id"].as_str().expect("event_id")).unwrap();

    // v1 response does not embed proof.signature — load via GET /v1/proof
    let (proof_status, proof_body) = call(
        app(state),
        authed_get(&format!("/v1/proof/{event_id}"), &account.api_key),
    )
    .await;
    assert_eq!(proof_status, StatusCode::OK, "body={proof_body}");
    let response_sig = proof_body["signature"]
        .as_str()
        .expect("proof signature")
        .to_string();

    let db_sig = db_signature(&pool, event_id).await;
    assert_eq!(
        db_sig, response_sig,
        "DB signature must equal /v1/proof signature byte-for-byte"
    );
    assert!(!db_sig.is_empty());
}

#[tokio::test]
async fn legacy_events_anchors_and_materializes_public_proof() {
    let pool = test_pool().await;
    let account = create_test_account(&pool).await;
    let state = test_state(pool.clone());
    let hash = file_hash(&format!("mat-{}", Uuid::new_v4()));

    let (status, body) = call(
        app(state.clone()),
        authed_json(
            "POST",
            "/events",
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
    let event_id = Uuid::parse_str(body["event_id"].as_str().expect("event_id")).unwrap();

    let db_sig = db_signature(&pool, event_id).await;
    assert!(!db_sig.is_empty(), "signature must be persisted");

    let (proof_status, proof_body) = call(
        app(state),
        authed_get(&format!("/v1/proof/{event_id}"), &account.api_key),
    )
    .await;
    assert_eq!(proof_status, StatusCode::OK, "body={proof_body}");
    assert_eq!(
        proof_body["proof_status"].as_str(),
        Some("anchored"),
        "persisted signature must yield anchored status"
    );

    assert_eq!(
        materialization_count(&pool, event_id).await,
        1,
        "public_proof_materialization row expected after legacy anchored commit"
    );
    let public_id = registry_public_id(&pool, &hash).await;
    assert!(
        public_id.as_ref().is_some_and(|id| !id.is_empty()),
        "public_proof_registry must expose a public_proof_id for the file_hash"
    );

    // silence unused warning if account fields expand later
    let _ = account.account_id;
}
