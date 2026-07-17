//! Integration tests for `file{}` on GET /v1/verify/{event_id} (Stage 5.4).

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

fn get_verify(
    client: &Client,
    api_key: &str,
    event_id: Uuid,
    file_hash: Option<&str>,
) -> reqwest::blocking::Response {
    let url = match file_hash {
        Some(hash) => format!("{BASE}/v1/verify/{event_id}?file_hash={hash}"),
        None => format!("{BASE}/v1/verify/{event_id}"),
    };
    client
        .get(url)
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
        .bind(format!("foreign-verify-file-{foreign_id}@test.local"))
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
        .bind(valid_hash("foreign-verify-file-event"))
        .bind(format!("foreign-verify-file-{event_id}"))
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
        .bind(valid_hash("verify-file-pending-empty-sig"))
        .bind(format!("verify-file-pending-{event_id}"))
        .bind("")
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("seed event");
    });
    (chain_id, event_id)
}

/// Zero-disclosure: stored hash and derived status enum must not appear in responses.
fn assert_file_response_contract(body: &Value) {
    assert!(body.get("expected_hash").is_none());
    assert!(body.get("stored_hash").is_none());
    assert!(body.get("stored_file_hash").is_none());
    if let Some(file) = body.get("file").and_then(|v| v.as_object()) {
        assert!(file.get("status").is_none());
        assert!(file.get("expected_hash").is_none());
        assert!(file.get("stored_hash").is_none());
        assert!(file.get("stored_file_hash").is_none());
    }
    if let Some(error) = body.get("error").and_then(|v| v.as_object()) {
        assert!(error.get("expected_hash").is_none());
        assert!(error.get("stored_hash").is_none());
    }
}

#[test]
fn v1_verify_file_not_provided() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-file-not-provided"),
        &format!("verify-file-none-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let resp = get_verify(&client, &api_key, event_id, None);
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["file"]["provided"], false);
    assert!(body["file"]["provided_hash"].is_null());
    assert!(body["file"]["is_valid_file_hash"].is_null());
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_matching_hash_returns_valid() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let stored = valid_hash("verify-file-valid-match");
    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &stored,
        &format!("verify-file-valid-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let resp = get_verify(&client, &api_key, event_id, Some(&stored));
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["file"]["provided"], true);
    assert_eq!(body["file"]["provided_hash"], stored);
    assert_eq!(body["file"]["is_valid_file_hash"], true);
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_mismatching_hash_returns_tampered() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let stored = valid_hash("verify-file-stored");
    let provided = valid_hash("verify-file-different");
    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &stored,
        &format!("verify-file-tampered-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let resp = get_verify(&client, &api_key, event_id, Some(&provided));
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["file"]["provided"], true);
    assert_eq!(body["file"]["provided_hash"], provided);
    assert_eq!(body["file"]["is_valid_file_hash"], false);
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_invalid_hex_returns_bad_request() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-file-invalid-hex"),
        &format!("verify-file-badhex-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let resp = get_verify(
        &client,
        &api_key,
        event_id,
        Some("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
    );
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(
        body["error"]["message"],
        "file_hash must be a valid SHA-256 hex string (64 chars, 0-9a-f)"
    );
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_wrong_length_returns_bad_request() {
    let client = Client::new();
    let api_key = evident_api_key();
    ensure_machine_plan(account_id_for_api_key(&api_key));
    let chain_id = Uuid::new_v4();
    cleanup_chain(chain_id);

    let created = post_event(
        &client,
        &api_key,
        chain_id,
        &valid_hash("verify-file-wrong-len"),
        &format!("verify-file-len-{}", Uuid::new_v4()),
    );
    let event_id = Uuid::parse_str(created["event_id"].as_str().unwrap()).unwrap();

    let short = "a".repeat(63);
    let resp = get_verify(&client, &api_key, event_id, Some(&short));
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_pending_with_valid_hash_returns_proof_not_ready() {
    let client = Client::new();
    let api_key = evident_api_key();
    let owner = account_id_for_api_key(&api_key);
    ensure_machine_plan(owner);

    let (chain_id, event_id) = seed_owned_event_empty_signature(owner);
    let provided = valid_hash("verify-file-pending-empty-sig");

    let resp = get_verify(&client, &api_key, event_id, Some(&provided));
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "proof_not_ready");
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_pending_with_invalid_hash_returns_bad_request() {
    let client = Client::new();
    let api_key = evident_api_key();
    let owner = account_id_for_api_key(&api_key);
    ensure_machine_plan(owner);

    let (chain_id, event_id) = seed_owned_event_empty_signature(owner);

    let resp = get_verify(&client, &api_key, event_id, Some("not-a-valid-hash"));
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(
        body["error"]["message"],
        "file_hash must be a valid SHA-256 hex string (64 chars, 0-9a-f)"
    );
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}

#[test]
fn v1_verify_file_foreign_event_invalid_hash_returns_not_found() {
    let client = Client::new();
    let api_key = evident_api_key();
    let caller_account = account_id_for_api_key(&api_key);
    ensure_machine_plan(caller_account);

    let foreign_owner = foreign_account_id(caller_account);
    let (chain_id, event_id) = seed_foreign_event(foreign_owner);

    let resp = get_verify(
        &client,
        &api_key,
        event_id,
        Some("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
    );
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().expect("json");
    assert_eq!(body["error"]["code"], "not_found");
    assert_file_response_contract(&body);

    cleanup_chain(chain_id);
}
