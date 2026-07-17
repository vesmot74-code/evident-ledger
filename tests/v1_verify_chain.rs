//! Integration tests for `chain{}` on GET /v1/verify/{event_id} (Stage 5.3).

use evident_ledger::api::v1::chain_verification::verify_chain_prefix;
use evident_ledger::db::EventRow;
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

fn get_proof(client: &Client, api_key: &str, event_id: Uuid) -> Value {
    let resp = client
        .get(format!("{BASE}/v1/proof/{event_id}"))
        .header("X-API-KEY", api_key)
        .send()
        .expect("get proof");
    assert_eq!(resp.status(), 200, "get proof");
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

fn set_event_parent_event_id(event_id: Uuid, parent_event_id: Uuid) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query("UPDATE events SET parent_event_id = $1 WHERE event_id = $2")
            .bind(parent_event_id)
            .bind(event_id)
            .execute(&pool)
            .await
            .expect("update parent_event_id");
    });
}

fn set_event_file_hash(event_id: Uuid, file_hash: &str) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        sqlx::query("UPDATE events SET file_hash = $1 WHERE event_id = $2")
            .bind(file_hash)
            .bind(event_id)
            .execute(&pool)
            .await
            .expect("update file_hash");
    });
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

fn load_prefix_rows(chain_id: Uuid, target_sequence: i64) -> Vec<EventRow> {
    #[derive(sqlx::FromRow)]
    struct PrefixRow {
        event_id: Uuid,
        parent_event_id: Uuid,
        file_hash: String,
        created_at: chrono::DateTime<chrono::Utc>,
        sequence: i64,
    }

    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        let rows = sqlx::query_as::<_, PrefixRow>(
            r#"
            SELECT event_id, parent_event_id, file_hash, created_at, sequence
            FROM events
            WHERE chain_id = $1 AND sequence <= $2
            ORDER BY sequence ASC
            "#,
        )
        .bind(chain_id)
        .bind(target_sequence)
        .fetch_all(&pool)
        .await
        .expect("load prefix");
        rows.into_iter()
            .map(|row| EventRow {
                event_id: row.event_id,
                parent_event_id: row.parent_event_id,
                file_hash: row.file_hash,
                created_at: row.created_at,
                sequence: row.sequence,
            })
            .collect()
    })
}

fn proof_anchor_material(proof: &Value) -> (String, String, String) {
    (
        proof["merkle_root"].as_str().unwrap().to_string(),
        proof["signature"].as_str().unwrap().to_string(),
        proof["public_key"].as_str().unwrap().to_string(),
    )
}

fn assert_anchored_verify_with_chain(resp: reqwest::blocking::Response) -> Value {
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["proof_status"], "anchored");
    assert!(body.get("chain").is_some());
    body
}

#[test]
fn v1_verify_chain_valid_anchored_event() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-chain-valid"),
        &format!("verify-chain-valid-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let body = assert_anchored_verify_with_chain(get_verify(&client, &api_key, event_id));
    let chain = &body["chain"];
    assert_eq!(chain["valid"], true);
    assert_eq!(chain["merkle_valid"], true);
    assert_eq!(chain["signature_valid"], true);
    assert_eq!(chain["errors"].as_array().unwrap().len(), 0);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_chain_broken_parent_returns_structural_errors() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-chain-parent"),
        &format!("verify-chain-parent-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();
    let proof = get_proof(&client, &api_key, event_id);
    let (anchor_root, anchor_signature, public_key) = proof_anchor_material(&proof);

    set_event_parent_event_id(event_id, Uuid::new_v4());

    let prefix = load_prefix_rows(chain_id, created["sequence"].as_i64().unwrap());
    let chain = verify_chain_prefix(
        chain_id,
        event_id,
        &anchor_signature,
        &public_key,
        &prefix,
        &anchor_root,
    );
    assert!(!chain.valid);
    assert!(!chain.merkle_valid);
    assert!(chain.signature_valid);
    assert!(!chain.errors.is_empty());

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_chain_broken_merkle_is_independent_of_signature() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-chain-merkle"),
        &format!("verify-chain-merkle-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();
    let proof = get_proof(&client, &api_key, event_id);
    let (anchor_root, anchor_signature, public_key) = proof_anchor_material(&proof);

    set_event_file_hash(event_id, &valid_hash("verify-chain-merkle-tampered"));

    let prefix = load_prefix_rows(chain_id, created["sequence"].as_i64().unwrap());
    let chain = verify_chain_prefix(
        chain_id,
        event_id,
        &anchor_signature,
        &public_key,
        &prefix,
        &anchor_root,
    );
    assert!(!chain.valid);
    assert!(!chain.merkle_valid);
    assert!(chain.signature_valid);
    assert!(chain.errors.is_empty());

    cleanup_chain(chain_id);
}

// Intentional deviation from Stage 5.2 verify failure path: chain integrity is reported
// inside a 200 anchored response when proof material still passes gating (signature still
// verifies against the commit-time anchor root carried in resolved_root).
#[test]
fn v1_verify_chain_broken_signature_is_independent_of_merkle() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-chain-signature"),
        &format!("verify-chain-signature-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();
    let proof = get_proof(&client, &api_key, event_id);
    let (anchor_root, _anchor_signature, public_key) = proof_anchor_material(&proof);

    set_event_signature(event_id, &"aa".repeat(64));

    let prefix = load_prefix_rows(chain_id, created["sequence"].as_i64().unwrap());
    let chain = verify_chain_prefix(
        chain_id,
        event_id,
        &"aa".repeat(64),
        &public_key,
        &prefix,
        &anchor_root,
    );
    assert!(!chain.valid);
    assert!(chain.merkle_valid);
    assert!(!chain.signature_valid);
    assert!(chain.errors.is_empty());

    cleanup_chain(chain_id);
}
