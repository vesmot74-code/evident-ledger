//! Identity key repository layer (Stage 9.1).

use sqlx::PgPool;
use uuid::Uuid;

use crate::models::identity_key::IdentityKey;
use crate::service::entitlements::{require_feature, Feature};

pub struct IdentityKeyRepository;

#[derive(Debug)]
pub enum IdentityKeyError {
    FingerprintAlreadyExists,
    KeyNotFound,
    EntitlementMissing,
    Database(sqlx::Error),
}

impl IdentityKeyRepository {
    /// Create a verified identity key (after successful proof-of-possession in Stage 9.2).
    pub async fn create(
        db: &PgPool,
        account_id: Uuid,
        public_key: &str,
        fingerprint: &str,
        label: Option<&str>,
    ) -> Result<IdentityKey, IdentityKeyError> {
        if Self::find_by_fingerprint(db, fingerprint)
            .await?
            .is_some()
        {
            return Err(IdentityKeyError::FingerprintAlreadyExists);
        }

        sqlx::query_as::<_, IdentityKey>(
            r#"
            INSERT INTO identity_keys (
                account_id, public_key, fingerprint, label, verified_at
            )
            VALUES ($1, $2, $3, $4, now())
            RETURNING
                id,
                account_id,
                public_key,
                fingerprint,
                label,
                created_at,
                verified_at,
                revoked_at
            "#,
        )
        .bind(account_id)
        .bind(public_key)
        .bind(fingerprint)
        .bind(label)
        .fetch_one(db)
        .await
        .map_err(|err| {
            if is_unique_violation(&err) {
                IdentityKeyError::FingerprintAlreadyExists
            } else {
                IdentityKeyError::Database(err)
            }
        })
    }

    pub async fn find_by_fingerprint(
        db: &PgPool,
        fingerprint: &str,
    ) -> Result<Option<IdentityKey>, IdentityKeyError> {
        sqlx::query_as::<_, IdentityKey>(
            r#"
            SELECT
                id,
                account_id,
                public_key,
                fingerprint,
                label,
                created_at,
                verified_at,
                revoked_at
            FROM identity_keys
            WHERE fingerprint = $1
            "#,
        )
        .bind(fingerprint)
        .fetch_optional(db)
        .await
        .map_err(IdentityKeyError::Database)
    }

    pub async fn find_by_id(db: &PgPool, id: Uuid) -> Result<Option<IdentityKey>, IdentityKeyError> {
        sqlx::query_as::<_, IdentityKey>(
            r#"
            SELECT
                id,
                account_id,
                public_key,
                fingerprint,
                label,
                created_at,
                verified_at,
                revoked_at
            FROM identity_keys
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(IdentityKeyError::Database)
    }

    pub async fn list_by_account(
        db: &PgPool,
        account_id: Uuid,
    ) -> Result<Vec<IdentityKey>, IdentityKeyError> {
        sqlx::query_as::<_, IdentityKey>(
            r#"
            SELECT
                id,
                account_id,
                public_key,
                fingerprint,
                label,
                created_at,
                verified_at,
                revoked_at
            FROM identity_keys
            WHERE account_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(db)
        .await
        .map_err(IdentityKeyError::Database)
    }

    pub async fn revoke(
        db: &PgPool,
        id: Uuid,
        account_id: Uuid,
    ) -> Result<IdentityKey, IdentityKeyError> {
        sqlx::query_as::<_, IdentityKey>(
            r#"
            UPDATE identity_keys
            SET revoked_at = now()
            WHERE id = $1
              AND account_id = $2
              AND revoked_at IS NULL
            RETURNING
                id,
                account_id,
                public_key,
                fingerprint,
                label,
                created_at,
                verified_at,
                revoked_at
            "#,
        )
        .bind(id)
        .bind(account_id)
        .fetch_optional(db)
        .await
        .map_err(IdentityKeyError::Database)?
        .ok_or(IdentityKeyError::KeyNotFound)
    }

    pub async fn check_entitlement(
        db: &PgPool,
        account_id: Uuid,
    ) -> Result<(), IdentityKeyError> {
        require_feature(db, account_id, Feature::Identity)
            .await
            .map_err(|err| match err {
                crate::service::entitlements::EntitlementError::Missing => {
                    IdentityKeyError::EntitlementMissing
                }
                crate::service::entitlements::EntitlementError::Database(e) => {
                    IdentityKeyError::Database(e)
                }
            })
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db) = err {
        return db.code().as_deref() == Some("23505");
    }
    false
}
