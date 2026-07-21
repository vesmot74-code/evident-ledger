//! Stage 9.1 — identity key storage tests.

mod common;
use evident_ledger::models::identity_key::IdentityKey;
use evident_ledger::service::identity_keys::{IdentityKeyError, IdentityKeyRepository};
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

async fn create_account(pool: &sqlx::PgPool, plan_name: &str) -> Uuid {
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, (SELECT plan_id FROM tariff_plans WHERE name = $3), 'none')
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@identity.test"))
    .bind(plan_name)
    .execute(pool)
    .await
    .expect("insert account");
    account_id
}

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query("DELETE FROM identity_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM api_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn create_identity_key_sets_verified_at() {
    let pool = test_pool().await;
    let account_id = create_account(&pool, "identity").await;
    let fingerprint = format!("fp_create_{}", Uuid::new_v4());

    let key = IdentityKeyRepository::create(
        &pool,
        account_id,
        "deadbeef01",
        &fingerprint,
        Some("primary"),
    )
    .await
    .expect("create key");

    assert!(key.verified_at <= chrono::Utc::now());
    assert!(key.revoked_at.is_none());
    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn create_duplicate_fingerprint_returns_error() {
    let pool = test_pool().await;
    let owner = create_account(&pool, "identity").await;
    let other = create_account(&pool, "identity").await;
    let fingerprint = format!("fp_dup_{}", Uuid::new_v4());

    IdentityKeyRepository::create(&pool, owner, "pubkey_a", &fingerprint, None)
        .await
        .expect("first create");

    let err = IdentityKeyRepository::create(&pool, other, "pubkey_b", &fingerprint, None)
        .await
        .expect_err("duplicate fingerprint");
    assert!(matches!(err, IdentityKeyError::FingerprintAlreadyExists));

    cleanup_account(&pool, owner).await;
    cleanup_account(&pool, other).await;
}

#[tokio::test]
async fn list_by_account_returns_keys() {
    let pool = test_pool().await;
    let account_id = create_account(&pool, "identity").await;
    let fp1 = format!("fp_list_{}", Uuid::new_v4());
    let fp2 = format!("fp_list_{}", Uuid::new_v4());

    IdentityKeyRepository::create(&pool, account_id, "pk1", &fp1, None)
        .await
        .expect("key1");
    IdentityKeyRepository::create(&pool, account_id, "pk2", &fp2, None)
        .await
        .expect("key2");

    let keys = IdentityKeyRepository::list_by_account(&pool, account_id)
        .await
        .expect("list");
    assert_eq!(keys.len(), 2);
    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn find_by_id_returns_key() {
    let pool = test_pool().await;
    let account_id = create_account(&pool, "identity").await;
    let fingerprint = format!("fp_find_{}", Uuid::new_v4());

    let created = IdentityKeyRepository::create(&pool, account_id, "pk-find", &fingerprint, None)
        .await
        .expect("create");

    let found = IdentityKeyRepository::find_by_id(&pool, created.id)
        .await
        .expect("find")
        .expect("some key");
    assert_eq!(found.id, created.id);
    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn find_by_id_missing_returns_none() {
    let pool = test_pool().await;
    let missing = IdentityKeyRepository::find_by_id(&pool, Uuid::new_v4())
        .await
        .expect("find");
    assert!(missing.is_none());
}

#[tokio::test]
async fn revoke_owner_sets_revoked_at() {
    let pool = test_pool().await;
    let account_id = create_account(&pool, "identity").await;
    let fingerprint = format!("fp_revoke_{}", Uuid::new_v4());

    let created = IdentityKeyRepository::create(&pool, account_id, "pk-revoke", &fingerprint, None)
        .await
        .expect("create");

    let revoked = IdentityKeyRepository::revoke(&pool, created.id, account_id)
        .await
        .expect("revoke");
    assert!(revoked.revoked_at.is_some());
    cleanup_account(&pool, account_id).await;
}

#[tokio::test]
async fn revoke_foreign_key_returns_not_found() {
    let pool = test_pool().await;
    let owner = create_account(&pool, "identity").await;
    let other = create_account(&pool, "identity").await;
    let fingerprint = format!("fp_foreign_{}", Uuid::new_v4());

    let created = IdentityKeyRepository::create(&pool, owner, "pk-foreign", &fingerprint, None)
        .await
        .expect("create");

    let err = IdentityKeyRepository::revoke(&pool, created.id, other)
        .await
        .expect_err("foreign revoke");
    assert!(matches!(err, IdentityKeyError::KeyNotFound));

    cleanup_account(&pool, owner).await;
    cleanup_account(&pool, other).await;
}

#[test]
fn active_identity_key_can_sign() {
    let key = IdentityKey {
        id: Uuid::new_v4(),
        account_id: Uuid::new_v4(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        label: None,
        created_at: chrono::Utc::now(),
        verified_at: chrono::Utc::now(),
        revoked_at: None,
    };
    assert!(key.is_active());
    assert!(key.can_sign());
}

#[test]
fn revoked_identity_key_cannot_sign() {
    let key = IdentityKey {
        id: Uuid::new_v4(),
        account_id: Uuid::new_v4(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        label: None,
        created_at: chrono::Utc::now(),
        verified_at: chrono::Utc::now(),
        revoked_at: Some(chrono::Utc::now()),
    };
    assert!(!key.is_active());
    assert!(!key.can_sign());
}

#[tokio::test]
async fn check_entitlement_uses_identity_feature() {
    let pool = test_pool().await;
    let identity_account = create_account(&pool, "identity").await;
    let free_account = create_account(&pool, "free").await;

    IdentityKeyRepository::check_entitlement(&pool, identity_account)
        .await
        .expect("identity plan entitled");
    let err = IdentityKeyRepository::check_entitlement(&pool, free_account)
        .await
        .expect_err("free plan not entitled");
    assert!(matches!(err, IdentityKeyError::EntitlementMissing));

    cleanup_account(&pool, identity_account).await;
    cleanup_account(&pool, free_account).await;
}
