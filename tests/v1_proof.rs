//! Integration tests for GET /v1/proof/{event_id} (Stage 2 §C).

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
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".evident/api_key"),
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
    });
}

fn valid_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn post_event(
    client: &Client,
    api_key: &str,
    chain_id: Uuid,
    file_hash: &str,
    key: &str,
) -> Value {
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

fn get_proof(client: &Client, api_key: &str, event_id: Uuid) -> reqwest::blocking::Response {
    client
        .get(format!("{BASE}/v1/proof/{event_id}"))
        .header("X-API-KEY", api_key)
        .send()
        .expect("get proof")
}

fn cleanup_chain(chain_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        let _ = sqlx::query("DELETE FROM idempotency_records WHERE response_json->>'chain_id' = $1")
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
        .bind(format!("foreign-proof-{foreign_id}@test.local"))
        .execute(&pool)
        .await
        .expect("seed foreign account");
        foreign_id
    })
}

/// Seeds a chain + event owned by `owner_account_id`; returns `(chain_id, event_id)`.
fn seed_foreign_event(owner_account_id: Uuid) -> (Uuid, Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let chain_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query(
            "INSERT INTO chains (chain_id, head_event_id, account_id) VALUES ($1, $2, $3)",
        )
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
        .bind(valid_hash("foreign-proof-event"))
        .bind(format!("foreign-proof-{event_id}"))
        .bind("")
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("seed foreign event");
    });
    (chain_id, event_id)
}

#[test]
fn v1_proof_historical_snapshot_stable_after_chain_grows() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let first = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("proof-event-1"),
        &format!("proof-key-1-{}", Uuid::new_v4()),
    );
    let event_id_1 = Uuid::parse_str(first["event_id"].as_str().unwrap()).unwrap();

    let proof_after_first = get_proof(&client, &api_key, event_id_1);
    assert_eq!(proof_after_first.status(), 200);
    let snap1: Value = proof_after_first.json().expect("json");
    assert_eq!(snap1["proof_status"], "anchored");
    let root1 = snap1["merkle_root"].as_str().unwrap().to_string();
    let sig1 = snap1["signature"].as_str().unwrap().to_string();

    let _second = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("proof-event-2"),
        &format!("proof-key-2-{}", Uuid::new_v4()),
    );

    let proof_event_1_again = get_proof(&client, &api_key, event_id_1);
    assert_eq!(proof_event_1_again.status(), 200);
    let snap1_later: Value = proof_event_1_again.json().expect("json");
    assert_eq!(snap1_later["merkle_root"].as_str(), Some(root1.as_str()));
    assert_eq!(snap1_later["signature"].as_str(), Some(sig1.as_str()));

    cleanup_chain(chain_id);
}

fn signature_from_db(event_id: Uuid) -> String {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query_scalar("SELECT signature FROM events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("signature row")
    })
}

#[test]
fn v1_get_proof_reads_persisted_signature_not_resign() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("persisted-sig"),
        &format!("proof-persist-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let persisted = signature_from_db(event_id);
    assert!(
        !persisted.is_empty(),
        "POST /v1/events must persist signature before commit"
    );

    let proof = get_proof(&client, &api_key, event_id);
    assert_eq!(proof.status(), 200);
    let body: Value = proof.json().expect("json");
    assert_eq!(body["signature"].as_str(), Some(persisted.as_str()));

    cleanup_chain(chain_id);
}

#[test]
fn v1_proof_returns_200_not_404_for_owned_event() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("proof-exists"),
        &format!("proof-key-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let resp = get_proof(&client, &api_key, event_id);
    assert_eq!(resp.status(), 200, "existing event must not 404");

    cleanup_chain(chain_id);
}

#[test]
fn v1_proof_foreign_event_returns_404() {
    let client = Client::new();
    let api_key = evident_api_key();
    let caller_account = account_id_for_api_key(&api_key);
    ensure_machine_plan(caller_account);

    let foreign_owner = foreign_account_id(caller_account);
    let (chain_id, event_id) = seed_foreign_event(foreign_owner);

    let resp = get_proof(&client, &api_key, event_id);
    assert_eq!(resp.status(), 404, "foreign event must not leak proof material");
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "not_found");

    cleanup_chain(chain_id);
}
