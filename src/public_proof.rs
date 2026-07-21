//! Public proof identity materialization (Stage 6.1 / 6.4).
//!
//! Materializes a public-safe projection into `public_proof_registry`.
//! Internal proof identifiers live only in `public_proof_materialization`,
//! which public endpoints never query.

use chrono::{DateTime, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub const PUBLIC_PROOF_STATUS_REGISTERED: &str = "REGISTERED";
pub const PUBLIC_INTEGRITY_VALID: &str = "VALID";

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PublicRegistryEntry {
    pub public_proof_id: String,
    pub file_hash: String,
    pub proof_status: String,
    pub registered_at: DateTime<Utc>,
    pub tsa_class: String,
    pub integrity_state: String,
    pub enabled: bool,
}

#[derive(Debug)]
pub enum PublicProofError {
    Database(sqlx::Error),
}

impl std::fmt::Display for PublicProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PublicProofError {}

impl From<sqlx::Error> for PublicProofError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

fn normalize_stored_file_hash(file_hash: &str) -> String {
    file_hash.trim().to_ascii_lowercase()
}

/// Maps account tariff plan to public TSA class (no provider disclosure).
pub fn tsa_class_from_plan(plan_name: &str) -> &'static str {
    match plan_name {
        "legal" => "legal",
        "identity" => "identity",
        "vault" => "vault",
        _ => "basic",
    }
}

/// Generates `pv_` + base58(128-bit CSPRNG). Not derived from internal identifiers.
pub fn generate_public_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("pv_{}", bs58::encode(bytes).into_string())
}

const BASE58_ALPHABET: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Validates `public_proof_id` against [`generate_public_id`] output (Stage 6.3/6.4 format).
pub fn validate_public_proof_id(public_proof_id: &str) -> bool {
    if !public_proof_id.starts_with("pv_") {
        return false;
    }
    let suffix = &public_proof_id[3..];
    if suffix.is_empty() {
        return false;
    }
    if !suffix.chars().all(|c| BASE58_ALPHABET.contains(c)) {
        return false;
    }
    match bs58::decode(suffix).into_vec() {
        Ok(bytes) => bytes.len() == 16,
        Err(_) => false,
    }
}

/// Records an internal proof as Anchored and materializes canonical public proof if eligible.
pub async fn on_proof_anchored(
    pool: &PgPool,
    proof_id: Uuid,
    file_hash: &str,
    tsa_class: &str,
) -> Result<(), PublicProofError> {
    let file_hash = normalize_stored_file_hash(file_hash);
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO public_proof_materialization (internal_proof_id, file_hash)
        VALUES ($1, $2)
        ON CONFLICT (internal_proof_id) DO NOTHING
        "#,
    )
    .bind(proof_id)
    .bind(&file_hash)
    .execute(&mut *tx)
    .await?;

    sync_canonical_public_proof_in_tx(&mut tx, &file_hash, tsa_class).await?;
    tx.commit().await?;
    Ok(())
}

/// Disables the active canonical public proof for `file_hash`.
pub async fn disable_canonical_public_proof(
    pool: &PgPool,
    file_hash: &str,
) -> Result<(), PublicProofError> {
    let file_hash = normalize_stored_file_hash(file_hash);
    let mut tx = pool.begin().await?;

    let public_proof_id: Option<String> = sqlx::query_scalar(
        r#"
        SELECT public_proof_id
        FROM public_proof_registry
        WHERE file_hash = $1 AND enabled = true
        "#,
    )
    .bind(&file_hash)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(public_proof_id) = public_proof_id {
        sqlx::query(
            r#"
            UPDATE public_proof_registry
            SET enabled = false
            WHERE public_proof_id = $1
            "#,
        )
        .bind(&public_proof_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE public_proof_materialization
            SET sticky_disabled = true
            WHERE public_proof_id = $1
            "#,
        )
        .bind(&public_proof_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn sync_canonical_public_proof(
    pool: &PgPool,
    file_hash: &str,
    tsa_class: &str,
) -> Result<(), PublicProofError> {
    let file_hash = normalize_stored_file_hash(file_hash);
    let mut tx = pool.begin().await?;
    sync_canonical_public_proof_in_tx(&mut tx, &file_hash, tsa_class).await?;
    tx.commit().await?;
    Ok(())
}

async fn sync_canonical_public_proof_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    file_hash: &str,
    tsa_class: &str,
) -> Result<(), PublicProofError> {
    let active: Option<String> = sqlx::query_scalar(
        r#"
        SELECT public_proof_id
        FROM public_proof_registry
        WHERE file_hash = $1 AND enabled = true
        FOR UPDATE
        "#,
    )
    .bind(file_hash)
    .fetch_optional(&mut **tx)
    .await?;

    if active.is_some() {
        return Ok(());
    }

    let candidate: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT internal_proof_id
        FROM public_proof_materialization
        WHERE file_hash = $1 AND sticky_disabled = false
        ORDER BY materialized_at ASC
        LIMIT 1
        "#,
    )
    .bind(file_hash)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(candidate) = candidate else {
        return Ok(());
    };

    let public_proof_id = generate_public_id();
    let insert = sqlx::query(
        r#"
        INSERT INTO public_proof_registry (
            public_proof_id, file_hash, proof_status, registered_at,
            tsa_class, integrity_state, enabled
        )
        VALUES ($1, $2, $3, now(), $4, $5, true)
        "#,
    )
    .bind(&public_proof_id)
    .bind(file_hash)
    .bind(PUBLIC_PROOF_STATUS_REGISTERED)
    .bind(tsa_class)
    .bind(PUBLIC_INTEGRITY_VALID)
    .execute(&mut **tx)
    .await;

    if let Err(err) = insert {
        if is_unique_violation(&err) {
            return Ok(());
        }
        return Err(PublicProofError::Database(err));
    }

    sqlx::query(
        r#"
        UPDATE public_proof_materialization
        SET public_proof_id = $1
        WHERE internal_proof_id = $2
        "#,
    )
    .bind(&public_proof_id)
    .bind(candidate)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23505")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use sqlx::postgres::PgPoolOptions;

    fn test_file_hash(label: &str) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(label.as_bytes()))
    }

    async fn test_pool() -> PgPool {
        dotenvy::dotenv().ok();
        let database_url = crate::db::require_test_database_url();
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("test db connection failed")
    }

    async fn cleanup(pool: &PgPool, file_hash: &str) {
        let _ = sqlx::query("DELETE FROM public_proof_materialization WHERE file_hash = $1")
            .bind(file_hash)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM public_proof_registry WHERE file_hash = $1")
            .bind(file_hash)
            .execute(pool)
            .await;
    }

    async fn active_registry(pool: &PgPool, file_hash: &str) -> Option<PublicRegistryEntry> {
        sqlx::query_as::<_, PublicRegistryEntry>(
            r#"
            SELECT public_proof_id, file_hash, proof_status, registered_at,
                   tsa_class, integrity_state, enabled
            FROM public_proof_registry
            WHERE file_hash = $1 AND enabled = true
            "#,
        )
        .bind(file_hash)
        .fetch_optional(pool)
        .await
        .expect("load active registry row")
    }

    async fn registry_count(pool: &PgPool, file_hash: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM public_proof_registry WHERE file_hash = $1")
            .bind(file_hash)
            .fetch_one(pool)
            .await
            .expect("count registry rows")
    }

    #[test]
    fn public_id_has_prefix_and_is_not_empty_payload() {
        let id = generate_public_id();
        assert!(id.starts_with("pv_"));
        assert!(id.len() > 3);
        assert!(validate_public_proof_id(&id));
    }

    #[test]
    fn public_ids_are_unique_across_many_generations() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            let id = generate_public_id();
            assert!(seen.insert(id));
        }
    }

    #[tokio::test]
    async fn on_proof_anchored_materializes_once() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-on-anchored");
        cleanup(&pool, &file_hash).await;

        let proof_id = Uuid::new_v4();
        on_proof_anchored(&pool, proof_id, &file_hash, "basic")
            .await
            .expect("on anchored");

        let active = active_registry(&pool, &file_hash)
            .await
            .expect("materialized");
        assert!(active.public_proof_id.starts_with("pv_"));
        assert_eq!(active.proof_status, PUBLIC_PROOF_STATUS_REGISTERED);
        assert_eq!(active.integrity_state, PUBLIC_INTEGRITY_VALID);

        cleanup(&pool, &file_hash).await;
    }

    #[tokio::test]
    async fn on_proof_anchored_normalizes_mixed_case_file_hash() {
        let pool = test_pool().await;
        let canonical = test_file_hash("public-proof-mixed-case-write");
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

        on_proof_anchored(&pool, Uuid::new_v4(), &mixed_case, "legal")
            .await
            .expect("on anchored");

        let active = active_registry(&pool, &canonical)
            .await
            .expect("materialized");
        assert_eq!(active.file_hash, canonical);
        assert_eq!(active.tsa_class, "legal");

        cleanup(&pool, &canonical).await;
    }

    #[tokio::test]
    async fn first_materialized_wins_over_later_internal_proof() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-first-materialized");
        cleanup(&pool, &file_hash).await;

        let proof_b = Uuid::new_v4();
        let proof_a = Uuid::new_v4();

        on_proof_anchored(&pool, proof_b, &file_hash, "basic")
            .await
            .expect("materialize B first");
        let first_id = active_registry(&pool, &file_hash)
            .await
            .expect("active")
            .public_proof_id
            .clone();

        on_proof_anchored(&pool, proof_a, &file_hash, "basic")
            .await
            .expect("anchor A later");

        let active = active_registry(&pool, &file_hash)
            .await
            .expect("still first");
        assert_eq!(active.public_proof_id, first_id);
        assert_eq!(registry_count(&pool, &file_hash).await, 1);

        cleanup(&pool, &file_hash).await;
    }

    #[tokio::test]
    async fn disabled_public_proof_is_not_auto_reactivated() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-sticky-disable");
        cleanup(&pool, &file_hash).await;

        let proof_id = Uuid::new_v4();
        on_proof_anchored(&pool, proof_id, &file_hash, "basic")
            .await
            .expect("materialize");
        disable_canonical_public_proof(&pool, &file_hash)
            .await
            .expect("disable");

        on_proof_anchored(&pool, proof_id, &file_hash, "basic")
            .await
            .expect("re-anchor same proof");

        assert!(active_registry(&pool, &file_hash).await.is_none());
        assert_eq!(registry_count(&pool, &file_hash).await, 1);

        cleanup(&pool, &file_hash).await;
    }
}
