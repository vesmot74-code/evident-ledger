//! Identity challenge repository (Stage 9.2).

use rand::rngs::OsRng;
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::identity_challenge::IdentityChallenge;

pub struct IdentityChallengeRepository;

#[derive(Debug)]
pub enum IdentityChallengeError {
    ChallengeNotFound,
    ChallengeExpired,
    ChallengeAlreadyUsed,
    Database(sqlx::Error),
}

impl IdentityChallengeRepository {
    /// Create a new challenge for an account (32 random bytes, hex-encoded).
    pub async fn create(
        db: &PgPool,
        account_id: Uuid,
    ) -> Result<IdentityChallenge, IdentityChallengeError> {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let challenge = hex::encode(bytes);

        sqlx::query_as::<_, IdentityChallenge>(
            r#"
            INSERT INTO identity_challenges (account_id, challenge)
            VALUES ($1, $2)
            RETURNING
                id,
                account_id,
                challenge,
                created_at,
                expires_at,
                used_at
            "#,
        )
        .bind(account_id)
        .bind(&challenge)
        .fetch_one(db)
        .await
        .map_err(IdentityChallengeError::Database)
    }

    /// Find a challenge by ID and account (no used/expired filtering).
    pub async fn find_by_id_and_account(
        db: &PgPool,
        challenge_id: Uuid,
        account_id: Uuid,
    ) -> Result<Option<IdentityChallenge>, IdentityChallengeError> {
        sqlx::query_as::<_, IdentityChallenge>(
            r#"
            SELECT
                id,
                account_id,
                challenge,
                created_at,
                expires_at,
                used_at
            FROM identity_challenges
            WHERE id = $1 AND account_id = $2
            "#,
        )
        .bind(challenge_id)
        .bind(account_id)
        .fetch_optional(db)
        .await
        .map_err(IdentityChallengeError::Database)
    }

    /// Validate challenge state (not used, not expired).
    pub fn validate(challenge: &IdentityChallenge) -> Result<(), IdentityChallengeError> {
        if challenge.is_used() {
            return Err(IdentityChallengeError::ChallengeAlreadyUsed);
        }
        if challenge.is_expired() {
            return Err(IdentityChallengeError::ChallengeExpired);
        }
        Ok(())
    }

    /// Mark a challenge as used (fails if already consumed).
    pub async fn mark_used(
        db: &PgPool,
        challenge_id: Uuid,
    ) -> Result<(), IdentityChallengeError> {
        let updated = sqlx::query_scalar::<_, Uuid>(
            r#"
            UPDATE identity_challenges
            SET used_at = now()
            WHERE id = $1 AND used_at IS NULL
            RETURNING id
            "#,
        )
        .bind(challenge_id)
        .fetch_optional(db)
        .await
        .map_err(IdentityChallengeError::Database)?;

        if updated.is_none() {
            return Err(IdentityChallengeError::ChallengeAlreadyUsed);
        }
        Ok(())
    }
}
