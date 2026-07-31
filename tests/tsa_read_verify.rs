//! TSA read-path verification + cache (RFC3161 harden).

mod common;

use base64::Engine;
use evident_ledger::tsa::{
    create_stub_attestation, resolve_and_cache_tsa_verification, token_sha256_hex,
    verify_token_fresh, CachedTsaVerification, TsaVerificationStatus,
};
use std::sync::Mutex;
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn seed_chain_event_and_token(
    pool: &sqlx::PgPool,
    chain_id: Uuid,
    event_id: Uuid,
    merkle_root: &str,
    token: &[u8],
) {
    let account_id = Uuid::new_v4();
    let free_plan_id: Uuid =
        sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = 'free'")
            .fetch_one(pool)
            .await
            .expect("free plan");

    sqlx::query("INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)")
        .bind(account_id)
        .bind(format!("tsa-read-{account_id}@test.local"))
        .bind(free_plan_id)
        .execute(pool)
        .await
        .expect("account");

    sqlx::query("INSERT INTO chains (chain_id, account_id) VALUES ($1, $2)")
        .bind(chain_id)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("chain");

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
    .bind("aa".repeat(32))
    .bind(format!("idem-{event_id}"))
    .bind("")
    .bind(1_i64)
    .execute(pool)
    .await
    .expect("event");

    sqlx::query(
        r#"
        INSERT INTO tsa_tokens (chain_id, event_id, merkle_root, tsa_token, tsa_timestamp, tsa_serial)
        VALUES ($1, $2, $3, $4, 1, 'test')
        ON CONFLICT (chain_id, merkle_root) DO UPDATE
        SET tsa_token = EXCLUDED.tsa_token,
            verification_status = NULL,
            verified_at = NULL,
            token_sha256 = NULL
        "#,
    )
    .bind(chain_id)
    .bind(event_id)
    .bind(merkle_root)
    .bind(token)
    .execute(pool)
    .await
    .expect("tsa token");
}

fn stub_bytes(merkle_root: &str) -> Vec<u8> {
    let att = create_stub_attestation(merkle_root, "stub");
    base64::engine::general_purpose::STANDARD
        .decode(att.raw_token_b64.trim())
        .expect("stub bytes")
}

#[tokio::test]
async fn stub_in_dev_verifies_then_cached_on_repeat() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe {
        std::env::set_var("DEV_MODE", "true");
        std::env::remove_var("APP_ENV");
    }
    let pool = common::test_pool().await;
    let chain_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let merkle_root = "11".repeat(32);
    let token = stub_bytes(&merkle_root);
    seed_chain_event_and_token(&pool, chain_id, event_id, &merkle_root, &token).await;

    let empty = CachedTsaVerification {
        verification_status: None,
        token_sha256: None,
    };
    let first = resolve_and_cache_tsa_verification(&pool, chain_id, &merkle_root, &token, &empty)
        .await
        .expect("first verify");
    assert_eq!(first.status, TsaVerificationStatus::Verified);

    let (status, sha): (Option<String>, Option<String>) = sqlx::query_as(
        r#"
        SELECT verification_status, token_sha256
        FROM tsa_tokens
        WHERE chain_id = $1 AND merkle_root = $2
        "#,
    )
    .bind(chain_id)
    .bind(&merkle_root)
    .fetch_one(&pool)
    .await
    .expect("cache row");
    assert_eq!(status.as_deref(), Some("verified"));
    assert_eq!(sha.as_deref(), Some(token_sha256_hex(&token).as_str()));

    let cache = CachedTsaVerification {
        verification_status: status,
        token_sha256: sha,
    };
    let second = resolve_and_cache_tsa_verification(&pool, chain_id, &merkle_root, &token, &cache)
        .await
        .expect("second verify");
    assert_eq!(second.status, TsaVerificationStatus::VerifiedCached);

    unsafe {
        std::env::remove_var("DEV_MODE");
    }
}

#[tokio::test]
async fn stub_outside_dev_fails() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe {
        std::env::remove_var("DEV_MODE");
        std::env::remove_var("APP_ENV");
    }
    let merkle_root = "22".repeat(32);
    let token = stub_bytes(&merkle_root);
    assert_eq!(
        verify_token_fresh(&token, &merkle_root).status,
        TsaVerificationStatus::Failed
    );
}

#[tokio::test]
async fn malformed_der_fails() {
    let merkle_root = "33".repeat(32);
    let token = vec![0x30, 0x01, 0xff];
    assert_eq!(
        verify_token_fresh(&token, &merkle_root).status,
        TsaVerificationStatus::Failed
    );
}

#[tokio::test]
async fn changed_token_sha_forces_reverify_not_stale_cache() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe {
        std::env::set_var("DEV_MODE", "true");
        std::env::remove_var("APP_ENV");
    }
    let pool = common::test_pool().await;
    let chain_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let merkle_root = "44".repeat(32);
    let token = stub_bytes(&merkle_root);
    seed_chain_event_and_token(&pool, chain_id, event_id, &merkle_root, &token).await;

    let empty = CachedTsaVerification {
        verification_status: None,
        token_sha256: None,
    };
    let _ = resolve_and_cache_tsa_verification(&pool, chain_id, &merkle_root, &token, &empty)
        .await
        .expect("seed cache");

    let stale = CachedTsaVerification {
        verification_status: Some("verified".into()),
        token_sha256: Some("deadbeef".into()),
    };
    let outcome = resolve_and_cache_tsa_verification(&pool, chain_id, &merkle_root, &token, &stale)
        .await
        .expect("reverify");
    assert_eq!(outcome.status, TsaVerificationStatus::Verified);

    unsafe {
        std::env::remove_var("DEV_MODE");
    }
}
