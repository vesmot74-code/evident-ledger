//! Public proof identity materialization (Stage 6.1).
//!
//! Implements first-materialized-wins canonical selection per `VERIFY_MODEL.md`.
//! `public_id` is assigned once at materialization — not derived from internal ids.

use chrono::{DateTime, Utc};
use rand::rngs::OsRng;
use rand::RngCore;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PublicProof {
    pub id: Uuid,
    pub public_id: String,
    pub proof_id: Uuid,
    pub file_hash: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
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

/// Generates `pv_` + base58(128-bit CSPRNG). Not derived from internal identifiers.
pub fn generate_public_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("pv_{}", bs58::encode(bytes).into_string())
}

/// Records an internal proof as Anchored and materializes canonical public proof if eligible.
///
/// Integration point §4.1 — call after internal proof transitions to `Anchored`.
pub async fn on_proof_anchored(
    pool: &PgPool,
    proof_id: Uuid,
    file_hash: &str,
) -> Result<(), PublicProofError> {
    sqlx::query(
        r#"
        INSERT INTO public_proof_registry (id, file_hash, status)
        VALUES ($1, $2, 'Anchored')
        ON CONFLICT (id) DO UPDATE
        SET status = EXCLUDED.status, file_hash = EXCLUDED.file_hash
        WHERE public_proof_registry.status <> 'Anchored'
        "#,
    )
    .bind(proof_id)
    .bind(file_hash)
    .execute(pool)
    .await?;
    sync_canonical_public_proof(pool, file_hash).await?;
    Ok(())
}

/// Disables the active canonical public proof and re-runs materialization for future candidates.
///
/// Integration point §4.2 — administrative disable hook (caller out of scope for 6.1).
pub async fn disable_canonical_public_proof(
    pool: &PgPool,
    file_hash: &str,
) -> Result<(), PublicProofError> {
    sqlx::query(
        r#"
        UPDATE public_proofs
        SET enabled = false
        WHERE file_hash = $1 AND enabled = true
        "#,
    )
    .bind(file_hash)
    .execute(pool)
    .await?;
    sync_canonical_public_proof(pool, file_hash).await?;
    Ok(())
}

/// Materializes canonical public proof for `file_hash` (first-materialized-wins, idempotent).
pub async fn sync_canonical_public_proof(
    pool: &PgPool,
    file_hash: &str,
) -> Result<(), PublicProofError> {
    let mut tx = pool.begin().await?;
    sync_canonical_public_proof_in_tx(&mut tx, file_hash).await?;
    tx.commit().await?;
    Ok(())
}

async fn sync_canonical_public_proof_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    file_hash: &str,
) -> Result<(), PublicProofError> {
    let active = sqlx::query_as::<_, PublicProof>(
        r#"
        SELECT id, public_id, proof_id, file_hash, enabled, created_at
        FROM public_proofs
        WHERE file_hash = $1 AND enabled = true
        FOR UPDATE
        "#,
    )
    .bind(file_hash)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(row) = active {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM public_proof_registry WHERE id = $1")
                .bind(row.proof_id)
                .fetch_optional(&mut **tx)
                .await?;

        match status.as_deref() {
            Some("Anchored") => {}
            Some(other) => {
                eprintln!(
                    "public_proof anomaly: active public proof {} references proof {} with non-Anchored status {other}",
                    row.public_id, row.proof_id
                );
            }
            None => {
                eprintln!(
                    "public_proof anomaly: active public proof {} references missing proof {}",
                    row.public_id, row.proof_id
                );
            }
        }
        return Ok(());
    }

    let candidate = sqlx::query_as::<_, PublicProofRecord>(
        r#"
        SELECT id, created_at
        FROM public_proof_registry
        WHERE file_hash = $1 AND status = 'Anchored'
        ORDER BY created_at ASC
        LIMIT 1
        "#,
    )
    .bind(file_hash)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(candidate) = candidate else {
        return Ok(());
    };

    let sticky_disabled: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM public_proofs
            WHERE proof_id = $1 AND enabled = false
        )
        "#,
    )
    .bind(candidate.id)
    .fetch_one(&mut **tx)
    .await?;

    if sticky_disabled {
        return Ok(());
    }

    let public_id = generate_public_id();
    let row_id = Uuid::new_v4();
    let insert = sqlx::query(
        r#"
        INSERT INTO public_proofs (id, public_id, proof_id, file_hash, enabled)
        VALUES ($1, $2, $3, $4, true)
        "#,
    )
    .bind(row_id)
    .bind(&public_id)
    .bind(candidate.id)
    .bind(file_hash)
    .execute(&mut **tx)
    .await;

    if let Err(err) = insert {
        if is_unique_violation(&err) {
            return Ok(());
        }
        return Err(PublicProofError::Database(err));
    }

    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct PublicProofRecord {
    id: Uuid,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
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
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for public_proof tests");
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("test db connection failed")
    }

    async fn insert_proof(
        pool: &PgPool,
        proof_id: Uuid,
        file_hash: &str,
        created_at: DateTime<Utc>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO public_proof_registry (id, file_hash, status, created_at)
            VALUES ($1, $2, 'Anchored', $3)
            "#,
        )
        .bind(proof_id)
        .bind(file_hash)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert proof");
    }

    async fn cleanup(pool: &PgPool, file_hash: &str) {
        let _ = sqlx::query("DELETE FROM public_proofs WHERE file_hash = $1")
            .bind(file_hash)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM public_proof_registry WHERE file_hash = $1")
            .bind(file_hash)
            .execute(pool)
            .await;
    }

    async fn active_public_proof(pool: &PgPool, file_hash: &str) -> Option<PublicProof> {
        sqlx::query_as::<_, PublicProof>(
            r#"
            SELECT id, public_id, proof_id, file_hash, enabled, created_at
            FROM public_proofs
            WHERE file_hash = $1 AND enabled = true
            "#,
        )
        .bind(file_hash)
        .fetch_optional(pool)
        .await
        .expect("load active public proof")
    }

    async fn public_proof_count(pool: &PgPool, file_hash: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM public_proofs WHERE file_hash = $1")
            .bind(file_hash)
            .fetch_one(pool)
            .await
            .expect("count public proofs")
    }

    #[test]
    fn public_id_has_prefix_and_is_not_empty_payload() {
        let id = generate_public_id();
        assert!(id.starts_with("pv_"));
        assert!(id.len() > 3);
        let payload = &id[3..];
        assert!(!payload.is_empty());
        assert!(payload.chars().all(|c| !"0OIl".contains(c)));
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
    async fn sync_is_idempotent_for_stable_state() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-idempotent");
        cleanup(&pool, &file_hash).await;

        let proof_id = Uuid::new_v4();
        insert_proof(&pool, proof_id, &file_hash, Utc::now()).await;

        sync_canonical_public_proof(&pool, &file_hash)
            .await
            .expect("first sync");
        sync_canonical_public_proof(&pool, &file_hash)
            .await
            .expect("second sync");

        assert_eq!(public_proof_count(&pool, &file_hash).await, 1);
        let active = active_public_proof(&pool, &file_hash)
            .await
            .expect("active row");
        assert_eq!(active.proof_id, proof_id);

        cleanup(&pool, &file_hash).await;
    }

    #[tokio::test]
    async fn first_materialized_wins_over_earlier_created_at() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-first-materialized");
        cleanup(&pool, &file_hash).await;

        let proof_a = Uuid::new_v4();
        let proof_b = Uuid::new_v4();
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let t1 = t0 + Duration::hours(1);

        insert_proof(&pool, proof_b, &file_hash, t1).await;
        sync_canonical_public_proof(&pool, &file_hash)
            .await
            .expect("materialize B first");

        insert_proof(&pool, proof_a, &file_hash, t0).await;
        sync_canonical_public_proof(&pool, &file_hash)
            .await
            .expect("sync after A anchored");

        let active = active_public_proof(&pool, &file_hash)
            .await
            .expect("active row");
        assert_eq!(active.proof_id, proof_b);
        assert_eq!(public_proof_count(&pool, &file_hash).await, 1);

        cleanup(&pool, &file_hash).await;
    }

    #[tokio::test]
    async fn disabled_public_proof_is_not_auto_reactivated() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-sticky-disable");
        cleanup(&pool, &file_hash).await;

        let proof_id = Uuid::new_v4();
        insert_proof(&pool, proof_id, &file_hash, Utc::now()).await;

        sqlx::query(
            r#"
            INSERT INTO public_proofs (id, public_id, proof_id, file_hash, enabled)
            VALUES ($1, $2, $3, $4, false)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(generate_public_id())
        .bind(proof_id)
        .bind(&file_hash)
        .execute(&pool)
        .await
        .expect("seed disabled row");

        sync_canonical_public_proof(&pool, &file_hash)
            .await
            .expect("sync");

        assert!(active_public_proof(&pool, &file_hash).await.is_none());
        assert_eq!(public_proof_count(&pool, &file_hash).await, 1);

        cleanup(&pool, &file_hash).await;
    }

    #[tokio::test]
    async fn parallel_sync_does_not_create_duplicate_active_rows() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-parallel");
        cleanup(&pool, &file_hash).await;

        let proof_id = Uuid::new_v4();
        insert_proof(&pool, proof_id, &file_hash, Utc::now()).await;

        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let hash_a = file_hash.clone();
        let hash_b = file_hash.clone();

        let (r1, r2) = tokio::join!(
            sync_canonical_public_proof(&pool_a, &hash_a),
            sync_canonical_public_proof(&pool_b, &hash_b),
        );
        r1.expect("parallel sync 1");
        r2.expect("parallel sync 2");

        assert_eq!(public_proof_count(&pool, &file_hash).await, 1);
        let active = active_public_proof(&pool, &file_hash)
            .await
            .expect("one active row");
        assert_eq!(active.proof_id, proof_id);

        cleanup(&pool, &file_hash).await;
    }

    #[tokio::test]
    async fn on_proof_anchored_materializes_once() {
        let pool = test_pool().await;
        let file_hash = test_file_hash("public-proof-on-anchored");
        cleanup(&pool, &file_hash).await;

        let proof_id = Uuid::new_v4();
        on_proof_anchored(&pool, proof_id, &file_hash)
            .await
            .expect("on anchored");

        let active = active_public_proof(&pool, &file_hash)
            .await
            .expect("materialized");
        assert_eq!(active.proof_id, proof_id);
        assert!(active.public_id.starts_with("pv_"));

        cleanup(&pool, &file_hash).await;
    }
}
