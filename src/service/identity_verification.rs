//! Identity signature verification for historical events (Stage 9.4).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::event::Event;

pub struct IdentityVerificationResult {
    pub present: bool,
    pub valid: bool,
    pub reason: Option<String>,
    pub fingerprint: Option<String>,
    pub key_id: Option<Uuid>,
}

#[derive(Debug)]
pub enum IdentityVerificationError {
    Database(sqlx::Error),
    InvalidEventHash,
}

pub struct IdentityVerificationService;

#[derive(Debug, sqlx::FromRow)]
struct VerificationKeyRow {
    public_key: String,
}

impl IdentityVerificationService {
    /// Verify identity signature for an event.
    ///
    /// Returns identity verification result without modifying core verification.
    /// Does **not** check `revoked_at` or `verified_at` — historical signatures remain
    /// cryptographically valid regardless of current key state.
    pub async fn verify(
        db: &PgPool,
        event: &Event,
        canonical_event_hash_hex: &str,
    ) -> Result<IdentityVerificationResult, IdentityVerificationError> {
        let Some(signature_hex) = event.identity_signature.as_deref() else {
            return Ok(IdentityVerificationResult {
                present: false,
                valid: false,
                reason: None,
                fingerprint: None,
                key_id: None,
            });
        };

        let key_id = event.identity_key_id;
        let fingerprint = event.identity_fingerprint.clone();

        let Some(key_id) = key_id else {
            return Ok(IdentityVerificationResult {
                present: true,
                valid: false,
                reason: Some("key_not_found".to_string()),
                fingerprint,
                key_id: None,
            });
        };

        let key = sqlx::query_as::<_, VerificationKeyRow>(
            r#"
            SELECT public_key
            FROM identity_keys
            WHERE id = $1
            "#,
        )
        .bind(key_id)
        .fetch_optional(db)
        .await
        .map_err(IdentityVerificationError::Database)?;

        let Some(key) = key else {
            return Ok(IdentityVerificationResult {
                present: true,
                valid: false,
                reason: Some("key_not_found".to_string()),
                fingerprint,
                key_id: Some(key_id),
            });
        };

        let raw_hash = hex::decode(canonical_event_hash_hex)
            .map_err(|_| IdentityVerificationError::InvalidEventHash)?;
        if raw_hash.len() != 32 {
            return Err(IdentityVerificationError::InvalidEventHash);
        }

        let valid = verify_ed25519_signature(&key.public_key, &raw_hash, signature_hex);
        Ok(IdentityVerificationResult {
            present: true,
            valid,
            reason: if valid {
                None
            } else {
                Some("signature_mismatch".to_string())
            },
            fingerprint,
            key_id: Some(key_id),
        })
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
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    #[test]
    fn verify_ed25519_accepts_valid_signature_on_raw_bytes() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let message = [0xCD; 32];
        let signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert!(verify_ed25519_signature(
            &public_key_hex,
            &message,
            &signature_hex
        ));
    }

    #[tokio::test]
    async fn absent_identity_signature_returns_not_present() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/nonexistent")
            .expect("lazy pool");
        let event = Event {
            event_id: Uuid::new_v4(),
            chain_id: Uuid::new_v4(),
            parent_event_id: Uuid::nil(),
            file_hash: "aa".repeat(32),
            sequence: 1,
            identity_key_id: None,
            identity_signature: None,
            identity_fingerprint: None,
        };

        let result = IdentityVerificationService::verify(&pool, &event, &"bb".repeat(64))
            .await
            .expect("verify");

        assert!(!result.present);
        assert!(!result.valid);
        assert!(result.reason.is_none());
    }
}
