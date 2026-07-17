//! Integration tests for public proof materialization wired to POST /v1/events (Stage 6.1.1).

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

fn cleanup_chain(chain_id: Uuid, file_hash: &str) {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        let _ = sqlx::query("DELETE FROM public_proof_materialization WHERE file_hash = $1")
            .bind(file_hash)
            .execute(&pool)
            .await;
        let _ = sqlx::query("DELETE FROM public_proof_registry WHERE file_hash = $1")
            .bind(file_hash)
            .execute(&pool)
            .await;
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

#[test]
fn v1_submit_anchored_event_materializes_public_proof() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    let file_hash = valid_hash("wire-public-proof-submit");
    cleanup_chain(chain_id, &file_hash);

    let body = post_event(
        &client,
        &api_key,
        chain_id,
        &file_hash,
        &format!("wire-public-proof-{}", Uuid::new_v4()),
    );
    assert_eq!(body["proof_status"], "anchored");
    let event_id = Uuid::parse_str(body["event_id"].as_str().unwrap()).unwrap();

    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");

        let materialization_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM public_proof_materialization WHERE internal_proof_id = $1 AND file_hash = $2",
        )
        .bind(event_id)
        .bind(&file_hash)
        .fetch_one(&pool)
        .await
        .expect("materialization count");
        assert_eq!(materialization_count, 1);

        let registry_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM public_proof_registry WHERE file_hash = $1 AND enabled = true",
        )
        .bind(&file_hash)
        .fetch_one(&pool)
        .await
        .expect("registry count");
        assert_eq!(registry_count, 1);

        let public_proof_id: String = sqlx::query_scalar(
            "SELECT public_proof_id FROM public_proof_registry WHERE file_hash = $1",
        )
        .bind(&file_hash)
        .fetch_one(&pool)
        .await
        .expect("public_proof_id");
        assert!(public_proof_id.starts_with("pv_"));
    });

    cleanup_chain(chain_id, &file_hash);
}

#[test]
fn v1_submit_idempotent_replay_does_not_duplicate_public_proof() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    let file_hash = valid_hash("wire-public-proof-idempotent");
    cleanup_chain(chain_id, &file_hash);

    let idem_key = format!("wire-idem-{}", Uuid::new_v4());
    let first = post_event(&client, &api_key, chain_id, &file_hash, &idem_key);
    assert_eq!(first["proof_status"], "anchored");
    let event_id = Uuid::parse_str(first["event_id"].as_str().unwrap()).unwrap();

    let second = post_event(&client, &api_key, chain_id, &file_hash, &idem_key);
    assert_eq!(second["event_id"], first["event_id"]);

    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&database_url).await.expect("db");
        let registry_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM public_proof_registry WHERE file_hash = $1",
        )
        .bind(&file_hash)
        .fetch_one(&pool)
        .await
        .expect("registry count");
        assert_eq!(registry_count, 1);
    });

    cleanup_chain(chain_id, &file_hash);
}
