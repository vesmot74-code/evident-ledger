//! Integration tests for GET /public/verify (Stage 6.3 / 6.4).

mod common;
use evident_ledger::api::public_verify::verify_by_hash;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn canonical_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

async fn test_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let database_url = common::test_database_url();
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("db")
}

async fn cleanup(pool: &sqlx::PgPool, file_hash: &str) {
    let _ = sqlx::query("DELETE FROM public_proof_materialization WHERE file_hash = $1")
        .bind(file_hash)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM public_proof_registry WHERE file_hash = $1")
        .bind(file_hash)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn public_verify_rejects_invalid_hash_without_database_lookup() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://127.0.0.1:1/unreachable")
        .expect("lazy pool");

    let response = verify_by_hash(&pool, Some("not-a-valid-hash".into()), None)
        .await
        .expect("invalid hash must fail before db access");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let parsed: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(parsed["error"]["code"], "invalid_request");
    assert_eq!(parsed["error"]["message"], "Invalid request");
    assert!(parsed["error"]["request_id"].is_string());
}

#[tokio::test]
async fn public_verify_hash_case_normalization_via_on_proof_anchored() {
    let pool = test_pool().await;
    let label = "public-verify-case-normalization";
    let canonical = canonical_hash(label);
    let mixed_case = canonical
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i % 2 == 0 {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect::<String>();
    cleanup(&pool, &canonical).await;

    evident_ledger::public_proof::on_proof_anchored(&pool, Uuid::new_v4(), &mixed_case, "basic")
        .await
        .expect("anchor");

    for hash in [canonical.clone(), canonical.to_uppercase(), mixed_case] {
        let response = verify_by_hash(&pool, Some(hash), None)
            .await
            .expect("verify");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let parsed: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(parsed["exists"], true);
    }

    cleanup(&pool, &canonical).await;
}

#[tokio::test]
async fn public_verify_not_found_returns_exists_false() {
    let pool = test_pool().await;
    let missing = canonical_hash("public-verify-missing");
    cleanup(&pool, &missing).await;

    let response = verify_by_hash(&pool, Some(missing), None)
        .await
        .expect("verify");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let parsed: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(parsed["exists"], false);
    assert!(parsed["public_proof_id"].is_null());
}
