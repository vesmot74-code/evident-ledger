//! Integration tests for POST /v1/events validation and response schema (Stage 2 §B).
//! Requires server on :3000, DATABASE_URL, EVIDENT_API_KEY (or ~/.evident/api_key).

mod common;
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
    let database_url = common::live_server_database_url();
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let key_hash = format!("{:x}", hasher.finalize());

    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("db connect");
        sqlx::query_scalar::<_, Uuid>(
            "SELECT account_id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL",
        )
        .bind(key_hash)
        .fetch_one(&pool)
        .await
        .expect("api key account")
    })
}

fn ensure_machine_plan(account_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = common::live_server_database_url();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("db connect");
        sqlx::query(
            r#"
            UPDATE accounts
            SET tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = 'free')
            WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("switch account to free plan");
    });
}

fn valid_file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn post_v1_event(
    client: &Client,
    api_key: &str,
    chain_id: Uuid,
    file_hash: &str,
    idempotency_key: Option<&str>,
    event_type: &str,
) -> reqwest::blocking::Response {
    let mut builder = client
        .post(format!("{BASE}/v1/events"))
        .header("X-API-KEY", api_key)
        .json(&json!({
            "chain_id": chain_id,
            "file_hash": file_hash,
            "event_type": event_type,
        }));

    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key);
    }

    builder.send().expect("POST /v1/events")
}

fn seed_foreign_chain(chain_id: Uuid, owner_account_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = common::live_server_database_url();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("db connect");
        sqlx::query(
            "INSERT INTO chains (chain_id, head_event_id, account_id) VALUES ($1, NULL, $2)",
        )
        .bind(chain_id)
        .bind(owner_account_id)
        .execute(&pool)
        .await
        .expect("seed foreign chain");
    });
}

fn foreign_account_id(caller_account_id: Uuid) -> Uuid {
    dotenvy::dotenv().ok();
    let database_url = common::live_server_database_url();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("db connect");
        sqlx::query_scalar::<_, Uuid>(
            "SELECT account_id FROM accounts WHERE account_id != $1 LIMIT 1",
        )
        .bind(caller_account_id)
        .fetch_one(&pool)
        .await
        .expect("foreign account")
    })
}

fn cleanup_chain(chain_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = common::live_server_database_url();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("db connect");
        let _ =
            sqlx::query("DELETE FROM idempotency_records WHERE response_json->>'chain_id' = $1")
                .bind(chain_id.to_string())
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

#[test]
fn v1_happy_path_response_schema_and_derived_proof_status() {
    let client = Client::new();
    let api_key = evident_api_key();
    let account_id = account_id_for_api_key(&api_key);
    ensure_machine_plan(account_id);

    let chain_id = Uuid::new_v4();
    let idempotency_key = format!("validation-happy-{}", Uuid::new_v4());
    let file_hash = valid_file_hash("happy-path");

    cleanup_chain(chain_id);

    let resp = post_v1_event(
        &client,
        &api_key,
        chain_id,
        &file_hash,
        Some(&idempotency_key),
        "submission",
    );
    assert_eq!(resp.status(), 200);

    let body: Value = resp.json().expect("json");
    assert!(body["event_id"].as_str().is_some());
    assert_eq!(
        body["chain_id"].as_str(),
        Some(chain_id.to_string().as_str())
    );
    assert!(body["sequence"].as_i64().is_some());
    assert_eq!(body["proof_status"].as_str(), Some("anchored"));
    assert!(body["trust_level"].as_str().is_some());
    assert!(body["request_id"].as_str().is_some());

    cleanup_chain(chain_id);
}

#[test]
fn v1_missing_idempotency_key_returns_400() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));

    let resp = post_v1_event(
        &client,
        &api_key,
        Uuid::new_v4(),
        &valid_file_hash("no-key"),
        None,
        "submission",
    );
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[test]
fn v1_invalid_file_hash_returns_400() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));

    let resp = post_v1_event(
        &client,
        &api_key,
        Uuid::new_v4(),
        "not-a-valid-hash",
        Some("validation-key-invalid-hash"),
        "submission",
    );
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[test]
fn v1_invalid_event_type_returns_400() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));

    let resp = post_v1_event(
        &client,
        &api_key,
        Uuid::new_v4(),
        &valid_file_hash("bad-event-type"),
        Some(&format!("validation-bad-type-{}", Uuid::new_v4())),
        "commit",
    );
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[test]
fn v1_foreign_chain_returns_404() {
    let client = Client::new();
    let api_key = evident_api_key();
    let caller_account = account_id_for_api_key(&api_key);
    ensure_machine_plan(caller_account);

    let foreign_owner = foreign_account_id(caller_account);
    let chain_id = Uuid::new_v4();
    seed_foreign_chain(chain_id, foreign_owner);

    let resp = post_v1_event(
        &client,
        &api_key,
        chain_id,
        &valid_file_hash("foreign-chain"),
        Some(&format!("validation-foreign-{}", Uuid::new_v4())),
        "submission",
    );
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "not_found");

    cleanup_chain(chain_id);
}
