//! Manual integration checks for the dev tariff switcher.
//! Run with: DEV_MODE=true evident-ledger (server) + cargo test dev_tariff_switcher -- --nocapture

use evident_ledger::client::EvidentClient;
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const CHAIN_ID: &str = "7aa1b5b0-94ff-4956-8a5e-114e54dae100";

fn evident_api_key() -> String {
    if let Ok(key) = std::env::var("EVIDENT_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return key;
        }
    }
    let path = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
        .join(".evident")
        .join("api_key");
    fs::read_to_string(path)
        .expect("EVIDENT_API_KEY or ~/.evident/api_key required")
        .trim()
        .to_string()
}

fn account_id_from_capabilities(client: &EvidentClient) -> Uuid {
    let caps = client.fetch_capabilities().expect("fetch capabilities");
    Uuid::parse_str(
        caps["account_id"]
            .as_str()
            .expect("account_id in capabilities"),
    )
    .expect("valid account_id uuid")
}

fn plan_name_from_capabilities(client: &EvidentClient) -> String {
    client.fetch_capabilities().expect("fetch capabilities")["plan_name"]
        .as_str()
        .expect("plan_name")
        .to_string()
}

fn tsa_mode_from_capabilities(client: &EvidentClient) -> String {
    client.fetch_capabilities().expect("fetch capabilities")["tsa_mode"]
        .as_str()
        .expect("tsa_mode")
        .to_string()
}

fn commit_status(file_body: &str) -> reqwest::StatusCode {
    let chain_uuid = Uuid::parse_str(CHAIN_ID).unwrap();
    let _client = EvidentClient::new("http://127.0.0.1:3000");
    let mut hasher = Sha256::new();
    hasher.update(file_body.as_bytes());
    let file_hash = format!("{:x}", hasher.finalize());
    let idempotency_key = Uuid::new_v4().to_string();

    let http = Client::new();
    let resp = http
        .post("http://127.0.0.1:3000/events")
        .header("X-API-KEY", evident_api_key())
        .json(&serde_json::json!({
            "chain_id": chain_uuid,
            "parent_event_id": null,
            "file_hash": file_hash,
            "idempotency_key": idempotency_key,
        }))
        .send()
        .expect("POST /events");
    resp.status()
}

#[test]
fn dev_tariff_switcher_end_to_end() {
    let client = EvidentClient::new("http://127.0.0.1:3000");
    let account_id = account_id_from_capabilities(&client);

    let caps = client.fetch_capabilities().expect("capabilities");
    assert!(
        caps["dev_tools_available"].as_bool().unwrap_or(false),
        "server must run with DEV_MODE=true or APP_ENV=development; dev_tools_available=false"
    );

    // Test 1 — Free → Vault (explicit baseline: set free first, then vault)
    let to_free_baseline = client
        .dev_change_plan(account_id, "free")
        .expect("baseline change-plan to free");
    eprintln!(
        "Test 1a (baseline → free): {:?}, capabilities plan={}",
        to_free_baseline,
        plan_name_from_capabilities(&client)
    );
    assert!(to_free_baseline.success);
    assert_eq!(to_free_baseline.new_plan, "free");
    assert_eq!(plan_name_from_capabilities(&client), "free");

    let to_vault = client
        .dev_change_plan(account_id, "vault")
        .expect("free→vault change-plan");
    eprintln!(
        "Test 1b (free→vault): {:?}, capabilities plan={}",
        to_vault,
        plan_name_from_capabilities(&client)
    );
    assert!(to_vault.success);
    assert_eq!(to_vault.old_plan, "free");
    assert_eq!(to_vault.new_plan, "vault");
    assert_eq!(plan_name_from_capabilities(&client), "vault");

    // Test 2 — Vault → Free, commit succeeds
    let to_free = client
        .dev_change_plan(account_id, "free")
        .expect("vault→free change-plan");
    let status_free = commit_status("tariff-switcher-test-free-commit");
    eprintln!(
        "Test 2: {:?}, tsa_mode={}, commit HTTP {}",
        to_free,
        tsa_mode_from_capabilities(&client),
        status_free
    );
    assert!(to_free.success);
    assert_eq!(to_free.old_plan, "vault");
    assert_eq!(to_free.new_plan, "free");
    assert_eq!(tsa_mode_from_capabilities(&client), "machine");
    assert_eq!(status_free, 200, "free plan commit should return 200");

    // Test 3 — Free → Vault, commit blocked with 503
    let back_vault = client
        .dev_change_plan(account_id, "vault")
        .expect("free→vault change-plan");
    let status_vault = commit_status("tariff-switcher-test-vault-commit");
    eprintln!(
        "Test 3: {:?}, tsa_mode={}, commit HTTP {}",
        back_vault,
        tsa_mode_from_capabilities(&client),
        status_vault
    );
    assert!(back_vault.success);
    assert_eq!(back_vault.new_plan, "vault");
    assert_eq!(tsa_mode_from_capabilities(&client), "qualified");
    assert_eq!(
        status_vault, 503,
        "vault plan commit should return 503 QualifiedTsaUnavailable"
    );
}

#[test]
fn dev_change_plan_forbidden_without_dev_mode() {
    if std::env::var("RUN_PROD_GATE_TEST")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        let client = EvidentClient::new("http://127.0.0.1:3000");
        let account_id = account_id_from_capabilities(&client);
        let err = client
            .dev_change_plan(account_id, "free")
            .expect_err("expected 403 without dev mode");
        let msg = err.to_string();
        eprintln!("Test 4: error message = {msg}");
        assert!(
            msg.contains("403") || msg.contains("Dev tools are not available"),
            "unexpected error: {msg}"
        );
    } else {
        eprintln!(
            "SKIP: set RUN_PROD_GATE_TEST=1 with server started without DEV_MODE to run Test 4"
        );
    }
}
