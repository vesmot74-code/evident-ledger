//! Integration tests for POST /v1/events idempotency.
//! Requires DATABASE_URL and EVIDENT_API_KEY (or ~/.evident/api_key).

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
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
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
        .expect("switch account to free plan for integration test");
    });
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

fn event_count_for_chain(chain_id: Uuid) -> i64 {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("db connect");
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE chain_id = $1")
            .bind(chain_id)
            .fetch_one(&pool)
            .await
            .expect("count events")
    })
}

fn cleanup_chain(chain_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
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
fn v1_idempotency_replay_and_conflict() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    let idempotency_key = format!("test-key-{}", Uuid::new_v4());

    let hash_a = format!("{:x}", Sha256::digest(b"payload-a"));
    let hash_b = format!("{:x}", Sha256::digest(b"payload-b"));

    cleanup_chain(chain_id);

    let first = post_v1_event(
        &client,
        &api_key,
        chain_id,
        &hash_a,
        Some(&idempotency_key),
        "submission",
    );
    assert_eq!(first.status(), 200, "first request should create event");
    let first_json: Value = first.json().expect("json");
    let event_id = first_json["event_id"]
        .as_str()
        .expect("event_id")
        .to_string();
    let count_after_first = event_count_for_chain(chain_id);
    assert_eq!(
        count_after_first, 1,
        "first request should create one event"
    );

    let second = post_v1_event(
        &client,
        &api_key,
        chain_id,
        &hash_a,
        Some(&idempotency_key),
        "submission",
    );
    assert_eq!(second.status(), 200, "replay should return 200");
    let second_json: Value = second.json().expect("json");
    assert_eq!(second_json["event_id"].as_str(), Some(event_id.as_str()));
    assert_eq!(
        event_count_for_chain(chain_id),
        count_after_first,
        "replay must not create a new event"
    );

    let conflict = post_v1_event(
        &client,
        &api_key,
        chain_id,
        &hash_b,
        Some(&idempotency_key),
        "submission",
    );
    assert_eq!(conflict.status(), 409, "hash mismatch should conflict");
    let conflict_json: Value = conflict.json().expect("json");
    assert_eq!(conflict_json["error"]["code"], "conflict");
    assert_eq!(
        event_count_for_chain(chain_id),
        count_after_first,
        "conflict must not create a new event"
    );

    cleanup_chain(chain_id);
}
