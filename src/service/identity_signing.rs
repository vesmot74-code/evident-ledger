//! User identity signature validation for event submission (Stage 9.3).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sqlx::PgPool;
use uuid::Uuid;

pub struct IdentitySigningService;

#[derive(Debug)]
pub enum IdentitySigningError {
    KeyNotFound,
    KeyRevoked,
    KeyNotVerified,
    InvalidSignature,
    InvalidEventHash,
    Database(sqlx::Error),
}

#[derive(Debug, sqlx::FromRow)]
struct SigningKeyRow {
    id: Uuid,
    public_key: String,
    fingerprint: String,
    verified_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl IdentitySigningService {
    /// Validate identity signature for an event.
    ///
    /// Performs only key lookup and cryptographic verification.
    /// Entitlement check is performed by the API layer via `Feature::Identity`.
    pub async fn validate_and_prepare(
        db: &PgPool,
        account_id: Uuid,
        identity_key_id: Uuid,
        signature_hex: &str,
        canonical_event_hash_hex: &str,
    ) -> Result<(Uuid, String, String), IdentitySigningError> {
        let key = sqlx::query_as::<_, SigningKeyRow>(
            r#"
            SELECT id, public_key, fingerprint, verified_at, revoked_at
            FROM identity_keys
            WHERE id = $1 AND account_id = $2
            "#,
        )
        .bind(identity_key_id)
        .bind(account_id)
        .fetch_optional(db)
        .await
        .map_err(IdentitySigningError::Database)?
        .ok_or(IdentitySigningError::KeyNotFound)?;

        if key.revoked_at.is_some() {
            return Err(IdentitySigningError::KeyRevoked);
        }
        if key.verified_at.is_none() {
            return Err(IdentitySigningError::KeyNotVerified);
        }

        let raw_hash = hex::decode(canonical_event_hash_hex)
            .map_err(|_| IdentitySigningError::InvalidEventHash)?;
        if raw_hash.len() != 32 {
            return Err(IdentitySigningError::InvalidEventHash);
        }

        if !verify_ed25519_signature(&key.public_key, &raw_hash, signature_hex) {
            return Err(IdentitySigningError::InvalidSignature);
        }

        Ok((key.id, signature_hex.to_string(), key.fingerprint))
    }
}

fn verify_ed25519_signature(public_key_hex: &str, message: &[u8], signature_hex: &str) -> bool {
    let Ok(pk_bytes) = hex::decode(public_key_hex) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(pk_array): Result<[u8; 32], _> = pk_bytes.try_into() else {
        return false;
    };
    let Ok(sig_array): Result<[u8; 64], _> = sig_bytes.try_into() else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_array) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);
    verifying_key.verify(message, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_ed25519_accepts_valid_signature_on_raw_bytes() {
        use ed25519_dalek::{Signer, SigningKey};
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let message = [0xAB; 32];
        let signature_hex = hex::encode(signing_key.sign(&message).to_bytes());

        assert!(verify_ed25519_signature(
            &public_key_hex,
            &message,
            &signature_hex
        ));
    }

    #[test]
    fn verify_ed25519_rejects_invalid_signature() {
        use ed25519_dalek::{Signer, SigningKey};
        use rand::rngs::OsRng;

        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let message = [0xAB; 32];
        let other = SigningKey::generate(&mut OsRng);
        let wrong_sig = hex::encode(other.sign(&message).to_bytes());

        assert!(!verify_ed25519_signature(
            &public_key_hex,
            &message,
            &wrong_sig
        ));
    }
}
