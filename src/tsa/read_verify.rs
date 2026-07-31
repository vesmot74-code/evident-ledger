//! Read-path TSA verification: independent crypto check + DB result cache.
//!
//! Source of truth is cryptographic verification (imprint + OpenSSL CA check).
//! Cached `verification_status` is a reproducible prior outcome, not trust in the row alone.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::trust_config::{
    check_tsa_configuration, freetsa_trust_path_options_from_env,
    verification_reason_for_config_error, TsaConfigError,
};
use super::types::{TsaVerificationOutcome, TsaVerificationReason, TsaVerificationStatus};
use super::verify::verify_tsa_attestation;
use super::{TsaAttestation, TsaTrustLevel};

/// SHA-256 hex of raw TSA token bytes (cache invalidation key).
pub fn token_sha256_hex(token: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token);
    hex::encode(hasher.finalize())
}

/// Stub tokens are allowed only in explicit development mode.
pub fn stubs_allowed_in_current_env() -> bool {
    std::env::var("DEV_MODE")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
        || std::env::var("APP_ENV")
            .map(|v| v.eq_ignore_ascii_case("development"))
            .unwrap_or(false)
}

pub fn is_evident_stub_json_token(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.contains("\"stub\":true"))
}

#[derive(Debug, Clone)]
pub struct CachedTsaVerification {
    pub verification_status: Option<String>,
    pub token_sha256: Option<String>,
}

/// Decide whether a cached row can be reused without re-running OpenSSL / imprint checks.
pub fn cached_status_if_fresh(
    cache: &CachedTsaVerification,
    current_token_sha: &str,
) -> Option<TsaVerificationStatus> {
    let status = cache.verification_status.as_deref()?;
    let cached_sha = cache.token_sha256.as_deref()?;
    if cached_sha != current_token_sha {
        return None;
    }
    match status {
        "verified" => Some(TsaVerificationStatus::VerifiedCached),
        // Do not trust a prior failure/unavailable without re-check — token or CA may have changed.
        // Except we still re-verify on sha mismatch above; for same sha + failed, re-running is OK
        // but optional. Spec: only verified+sha → verified_cached.
        _ => None,
    }
}

/// Fresh verification of token bytes against `merkle_root` (hex SHA-256).
pub fn verify_token_fresh(token: &[u8], merkle_root_hex: &str) -> TsaVerificationOutcome {
    if is_evident_stub_json_token(token) {
        return verify_stub_token(token, merkle_root_hex);
    }
    verify_rfc3161_token(token, merkle_root_hex)
}

fn verify_stub_token(token: &[u8], merkle_root_hex: &str) -> TsaVerificationOutcome {
    if !stubs_allowed_in_current_env() {
        return TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed);
    }

    let Some(tsr_hash) = stub_sha256_from_token_bytes(token) else {
        return TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed);
    };

    let att = TsaAttestation {
        provider: "stub".to_string(),
        timestamp: 0,
        tsr_hash,
        signature_valid: true,
        raw_token_b64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, token),
        trust_level: TsaTrustLevel::Stub,
    };

    match verify_tsa_attestation(&att, merkle_root_hex) {
        super::types::TsaStatus::Verified => TsaVerificationOutcome::verified(),
        _ => TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed),
    }
}

fn stub_sha256_from_token_bytes(token: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(token).ok()?;
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    payload.get("sha256")?.as_str().map(str::to_string)
}

fn reason_from_config_error(err: &TsaConfigError) -> TsaVerificationReason {
    match verification_reason_for_config_error(err) {
        "trust_material_invalid" => TsaVerificationReason::TrustMaterialInvalid,
        _ => TsaVerificationReason::TrustMaterialMissing,
    }
}

fn verify_rfc3161_token(token: &[u8], merkle_root_hex: &str) -> TsaVerificationOutcome {
    #[cfg(test)]
    if let Some(status) = test_hooks::take_der_override(token, merkle_root_hex) {
        return match status {
            TsaVerificationStatus::Verified => TsaVerificationOutcome::verified(),
            TsaVerificationStatus::VerifiedCached => TsaVerificationOutcome::verified_cached(),
            TsaVerificationStatus::Failed => {
                TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed)
            }
            TsaVerificationStatus::Unavailable => {
                TsaVerificationOutcome::unavailable(TsaVerificationReason::TrustMaterialMissing)
            }
        };
    }

    let Ok(hash) = hex::decode(merkle_root_hex) else {
        return TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed);
    };
    if hash.len() != 32 {
        return TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed);
    }

    // (1) Structural + message-imprint — independent of DB.
    if notary_tsa::parse_and_validate_tsr(token, &hash).is_err() {
        return TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed);
    }

    // (2) Trust material must be configured before OpenSSL CA chain check.
    // Missing/invalid local trust files → `unavailable` + reason (not a TSA network outage).
    let (ca, untrusted) = freetsa_trust_path_options_from_env();
    if let Err(err) = check_tsa_configuration(ca.as_deref(), untrusted.as_deref()) {
        return TsaVerificationOutcome::unavailable(reason_from_config_error(&err));
    }
    let ca = ca.expect("checked");
    let untrusted = untrusted.expect("checked");

    match notary_tsa::verify_tsr_bytes(token, &hash, &ca, &untrusted) {
        Ok(()) => TsaVerificationOutcome::verified(),
        Err(_) => TsaVerificationOutcome::failed(TsaVerificationReason::VerificationFailed),
    }
}

/// Load cache fields, reuse or freshly verify, then race-safe persist.
pub async fn resolve_and_cache_tsa_verification(
    pool: &PgPool,
    chain_id: Uuid,
    merkle_root: &str,
    token: &[u8],
    cache: &CachedTsaVerification,
) -> Result<TsaVerificationOutcome, sqlx::Error> {
    let current_sha = token_sha256_hex(token);

    if let Some(cached) = cached_status_if_fresh(cache, &current_sha) {
        return Ok(TsaVerificationOutcome {
            status: cached,
            reason: None,
        });
    }

    let outcome = verify_token_fresh(token, merkle_root);
    persist_verification_cache(pool, chain_id, merkle_root, &current_sha, outcome.status).await?;
    Ok(outcome)
}

/// Persist verification outcome. Race-safe: only writes when no status yet or token hash changed.
async fn persist_verification_cache(
    pool: &PgPool,
    chain_id: Uuid,
    merkle_root: &str,
    token_sha: &str,
    outcome: TsaVerificationStatus,
) -> Result<(), sqlx::Error> {
    let Some(status_value) = outcome.cache_value() else {
        return Ok(());
    };

    // Claim / refresh: first writer for NULL status, or any writer when token bytes changed.
    let result = sqlx::query(
        r#"
        UPDATE tsa_tokens
        SET verification_status = $1,
            verified_at = now(),
            token_sha256 = $2
        WHERE chain_id = $3
          AND merkle_root = $4
          AND (
                verification_status IS NULL
             OR token_sha256 IS DISTINCT FROM $2
          )
        "#,
    )
    .bind(status_value)
    .bind(token_sha)
    .bind(chain_id)
    .bind(merkle_root)
    .execute(pool)
    .await?;

    let _ = result; // 0 rows ⇒ another worker already cached a result for this token
    Ok(())
}

#[cfg(test)]
pub mod test_hooks {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    thread_local! {
        static DER_OVERRIDE: RefCell<Option<Box<dyn FnMut(&[u8], &str) -> TsaVerificationStatus>>> =
            RefCell::new(None);
    }

    static OPENSSL_CALLS: AtomicUsize = AtomicUsize::new(0);

    pub fn reset_openssl_calls() {
        OPENSSL_CALLS.store(0, Ordering::SeqCst);
    }

    pub fn openssl_calls() -> usize {
        OPENSSL_CALLS.load(Ordering::SeqCst)
    }

    pub fn set_der_override(f: impl FnMut(&[u8], &str) -> TsaVerificationStatus + 'static) {
        DER_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(f));
        });
    }

    pub fn clear_der_override() {
        DER_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }

    pub(super) fn take_der_override(
        token: &[u8],
        merkle_root: &str,
    ) -> Option<TsaVerificationStatus> {
        DER_OVERRIDE.with(|slot| {
            let mut guard = slot.borrow_mut();
            let f = guard.as_mut()?;
            OPENSSL_CALLS.fetch_add(1, Ordering::SeqCst);
            Some(f(token, merkle_root))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsa::create_stub_attestation;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn stub_token_bytes(merkle_root: &str) -> Vec<u8> {
        let att = create_stub_attestation(merkle_root, "stub");
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            att.raw_token_b64.trim(),
        )
        .unwrap()
    }

    #[test]
    fn stub_token_in_dev_verifies() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DEV_MODE", "true");
        std::env::remove_var("APP_ENV");
        let root = "aa".repeat(32);
        let token = stub_token_bytes(&root);
        assert_eq!(
            verify_token_fresh(&token, &root).status,
            TsaVerificationStatus::Verified
        );
        std::env::remove_var("DEV_MODE");
    }

    #[test]
    fn stub_token_outside_dev_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DEV_MODE");
        std::env::remove_var("APP_ENV");
        let root = "bb".repeat(32);
        let token = stub_token_bytes(&root);
        let outcome = verify_token_fresh(&token, &root);
        assert_eq!(outcome.status, TsaVerificationStatus::Failed);
        assert_eq!(
            outcome.reason,
            Some(TsaVerificationReason::VerificationFailed)
        );
    }

    #[test]
    fn malformed_der_token_fails() {
        test_hooks::clear_der_override();
        let root = "cc".repeat(32);
        let token = vec![0x30, 0x03, 0x01, 0x01, 0xff];
        assert_eq!(
            verify_token_fresh(&token, &root).status,
            TsaVerificationStatus::Failed
        );
    }

    #[test]
    fn valid_der_token_via_hook_is_verified() {
        test_hooks::reset_openssl_calls();
        test_hooks::set_der_override(|_token, _root| TsaVerificationStatus::Verified);
        let root = "dd".repeat(32);
        let token = vec![0x30, 0x82, 0x01, 0x00]; // non-stub bytes; hook short-circuits
        assert_eq!(
            verify_token_fresh(&token, &root).status,
            TsaVerificationStatus::Verified
        );
        assert_eq!(test_hooks::openssl_calls(), 1);
        test_hooks::clear_der_override();
    }

    #[test]
    fn cached_verified_same_sha_is_verified_cached() {
        let sha = "abc123";
        let cache = CachedTsaVerification {
            verification_status: Some("verified".into()),
            token_sha256: Some(sha.into()),
        };
        assert_eq!(
            cached_status_if_fresh(&cache, sha),
            Some(TsaVerificationStatus::VerifiedCached)
        );
    }

    #[test]
    fn cached_status_ignored_when_token_sha_changes() {
        let cache = CachedTsaVerification {
            verification_status: Some("verified".into()),
            token_sha256: Some("old".into()),
        };
        assert_eq!(cached_status_if_fresh(&cache, "new"), None);
    }
}
