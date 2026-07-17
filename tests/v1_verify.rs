//! Integration tests for GET /v1/verify/{event_id} (Stage 5.2 proof status gating).

use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const BASE: &str = "http://127.0.0.1:3000";

fn evident_api_key() -> String {
    if let Ok(key) = std::env::var("EVIDENT_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return key;
        }
    }
    fs::read_to_string(
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".evident/api_key"),
    )
    .expect("EVIDENT_API_KEY or ~/.evident/api_key required")
    .trim()
    .to_string()
}

fn account_id_for_api_key(api_key: &str) -> Uuid {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let key_hash = format!("{:x}", hasher.finalize());
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query_scalar::<_, Uuid>(
            "SELECT account_id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL",
        )
        .bind(key_hash)
        .fetch_one(&pool)
        .await
        .expect("account")
    })
}

fn ensure_machine_plan(account_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query(
            "UPDATE accounts SET tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = 'free') WHERE account_id = $1",
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("plan");
        sqlx::query(
            r#"
            UPDATE usage_monthly
            SET server_commits = 0, tsa_requests = 0
            WHERE account_id = $1 AND period_start = date_trunc('month', now())::date
            "#,
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("usage reset");
    });
}

fn valid_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn post_event(client: &Client, api_key: &str, chain_id: Uuid, file_hash: &str, key: &str) -> Value {
    let resp = client
        .post(format!("{BASE}/v1/events"))
        .header("X-API-KEY", api_key)
        .header("Idempotency-Key", key)
        .json(&json!({
            "chain_id": chain_id,
            "file_hash": file_hash,
            "event_type": "submission",
        }))
        .send()
        .expect("post");
    assert_eq!(resp.status(), 200, "post event");
    resp.json().expect("json")
}

fn get_verify(client: &Client, api_key: &str, event_id: Uuid) -> reqwest::blocking::Response {
    client
        .get(format!("{BASE}/v1/verify/{event_id}"))
        .header("X-API-KEY", api_key)
        .send()
        .expect("get verify")
}

fn cleanup_chain(chain_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        let _ =
            sqlx::query("DELETE FROM idempotency_records WHERE response_json->>'chain_id' = $1")
                .bind(chain_id.to_string())
                .execute(&pool)
                .await;
        let _ = sqlx::query("DELETE FROM tsa_tokens WHERE chain_id = $1")
            .bind(chain_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM events WHERE chain_id = $1")
            .bind(chain_id)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM chains WHERE chain_id = $1")
            .bind(chain_id)
            .execute(&pool)
            .await;
    });
}

fn foreign_account_id(caller_account_id: Uuid) -> Uuid {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db connect");
        if let Ok(existing) = sqlx::query_scalar::<_, Uuid>(
            "SELECT account_id FROM accounts WHERE account_id != $1 LIMIT 1",
        )
        .bind(caller_account_id)
        .fetch_optional(&pool)
        .await
        {
            if let Some(id) = existing {
                return id;
            }
        }
        let foreign_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, (SELECT plan_id FROM tariff_plans WHERE name = 'free'))",
        )
        .bind(foreign_id)
        .bind(format!("foreign-verify-{foreign_id}@test.local"))
        .execute(&pool)
        .await
        .expect("seed foreign account");
        foreign_id
    })
}

fn seed_foreign_event(owner_account_id: Uuid) -> (Uuid, Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let chain_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query("INSERT INTO chains (chain_id, head_event_id, account_id) VALUES ($1, $2, $3)")
            .bind(chain_id)
            .bind(event_id)
            .bind(owner_account_id)
            .execute(&pool)
            .await
            .expect("seed foreign chain");
        sqlx::query(
            r#"
            INSERT INTO events (
                event_id, chain_id, parent_event_id, file_hash,
                idempotency_key, signature, sequence
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(event_id)
        .bind(chain_id)
        .bind(Uuid::nil())
        .bind(valid_hash("foreign-verify-event"))
        .bind(format!("foreign-verify-{event_id}"))
        .bind("")
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("seed foreign event");
    });
    (chain_id, event_id)
}

fn seed_owned_event_empty_signature(owner_account_id: Uuid) -> (Uuid, Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let chain_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query("INSERT INTO chains (chain_id, head_event_id, account_id) VALUES ($1, $2, $3)")
            .bind(chain_id)
            .bind(event_id)
            .bind(owner_account_id)
            .execute(&pool)
            .await
            .expect("seed chain");
        sqlx::query(
            r#"
            INSERT INTO events (
                event_id, chain_id, parent_event_id, file_hash,
                idempotency_key, signature, sequence
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(event_id)
        .bind(chain_id)
        .bind(Uuid::nil())
        .bind(valid_hash("verify-pending-empty-sig"))
        .bind(format!("verify-pending-{event_id}"))
        .bind("")
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("seed event");
    });
    (chain_id, event_id)
}

fn set_event_signature(event_id: Uuid, signature: &str) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query("UPDATE events SET signature = $1 WHERE event_id = $2")
            .bind(signature)
            .bind(event_id)
            .execute(&pool)
            .await
            .expect("update signature");
    });
}

fn error_body_without_request_id(body: &Value) -> Value {
    let mut normalized = body.clone();
    if let Some(error) = normalized.get_mut("error").and_then(|v| v.as_object_mut()) {
        error.remove("request_id");
    }
    normalized
}

#[test]
fn v1_verify_missing_event_returns_not_found() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));

    let resp = get_verify(&client, &api_key, Uuid::new_v4());
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "not_found");
    assert!(body["error"]["request_id"].as_str().is_some());
}

#[test]
fn v1_verify_foreign_event_returns_not_found_same_shape_as_missing() {
    let client = Client::new();
    let api_key = evident_api_key();
    let caller_account = account_id_for_api_key(&api_key);
    ensure_machine_plan(caller_account);

    let missing = get_verify(&client, &api_key, Uuid::new_v4());
    assert_eq!(missing.status(), 404);
    let missing_body: Value = missing.json().expect("json");

    let foreign_owner = foreign_account_id(caller_account);
    let (chain_id, event_id) = seed_foreign_event(foreign_owner);

    let foreign = get_verify(&client, &api_key, event_id);
    assert_eq!(foreign.status(), 404);
    let foreign_body: Value = foreign.json().expect("json");

    assert_eq!(
        error_body_without_request_id(&missing_body),
        error_body_without_request_id(&foreign_body)
    );

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_pending_proof_returns_proof_not_ready() {
    let client = Client::new();
    let api_key = evident_api_key();
    let owner = account_id_for_api_key(&api_key);
    ensure_machine_plan(owner);

    // Empty persisted signature → ProofStatus::Pending via resolve_proof_state.
    let (chain_id, event_id) = seed_owned_event_empty_signature(owner);

    let resp = get_verify(&client, &api_key, event_id);
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "proof_not_ready");
    assert!(body["error"]["request_id"].as_str().is_some());

    cleanup_chain(chain_id);
}

// Intentional deviation from TZ letter: we corrupt persisted signature after
// post_event instead of seeding a boolean failure_signal flag. Failed status
// still flows through build_proof_snapshot_read → detect_failure_signal →
// resolve_proof_state (full stack, not a resolver bypass).
#[test]
fn v1_verify_failed_proof_returns_proof_generation_failed() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-failed-sig"),
        &format!("verify-failed-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();
    set_event_signature(event_id, &"aa".repeat(64));

    let resp = get_verify(&client, &api_key, event_id);
    assert_eq!(resp.status(), 422);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "proof_generation_failed");
    assert!(body["error"]["request_id"].as_str().is_some());

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_anchored_returns_minimal_body() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-anchored"),
        &format!("verify-anchored-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();
    let expected_chain_id = Uuid::parse_str(created["chain_id"].as_str().unwrap()).unwrap();
    let expected_sequence = created["sequence"].as_i64().unwrap();

    let resp = get_verify(&client, &api_key, event_id);
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["proof_status"], "anchored");
    assert_eq!(body["event_id"], event_id.to_string());
    assert_eq!(body["chain_id"], expected_chain_id.to_string());
    assert_eq!(body["sequence"], expected_sequence);
    assert!(body["request_id"].as_str().is_some());
    assert_eq!(body["chain"]["valid"], true);
    assert!(body.get("file").is_none());
    assert!(body.get("tsa").is_none());

    cleanup_chain(chain_id);
}

// Stage 5.2 gap: infra failure → 500 internal_error is not covered here — no mock
// infrastructure for snapshot builder / DB failures in integration tests.
