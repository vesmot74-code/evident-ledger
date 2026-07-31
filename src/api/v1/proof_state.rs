//! Shared proof status resolution for `GET /v1/proof` and `GET /v1/verify`.
//!
//! Single source of truth for TSA load, read-path verification (with cache),
//! `ProofContext` assembly, and `derive_proof_status`.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::tsa::{
    resolve_and_cache_tsa_verification, CachedTsaVerification, TsaStatus, TsaVerificationStatus,
};

use super::errors::ApiError;
use super::event_access::Event;
use super::proof_material::ProofSnapshot;
use super::proof_status::{derive_proof_status, ProofContext, ProofStatus};

pub struct ResolvedProofState {
    pub status: ProofStatus,
    pub tsa: Option<Value>,
    pub context: ProofContext,
    /// Commit-time merkle root for prefix verification (Stage 5.3).
    pub resolved_root: String,
}

/// Loads TSA state, runs/caches cryptographic verification, merges failure signals,
/// and derives API `proof_status`.
///
/// TSA verification statuses (API `tsa.verification_status`):
/// - `verified` — fresh crypto check
/// - `verified_cached` — prior check reused (`token_sha256` match)
/// - `failed` — invalid token / stub outside dev
/// - `unavailable` — imprint OK but OpenSSL trust bundle unavailable
///
/// Optional additive `tsa.verification_reason` distinguishes trust-material
/// misconfiguration from crypto failure without changing status vocabulary.
pub async fn resolve_proof_state(
    pool: &PgPool,
    chain_id: Uuid,
    _event: &Event,
    snapshot: &ProofSnapshot,
) -> Result<ResolvedProofState, ApiError> {
    let tsa_row = load_tsa_row_for_root(pool, chain_id, &snapshot.merkle_root)
        .await
        .map_err(|_| ApiError::Internal)?;

    let (validation_status, verification_status, tsa_json) = match tsa_row {
        None => (TsaStatus::NotProvided, None, None),
        Some(row) => {
            let cache = CachedTsaVerification {
                verification_status: row.verification_status.clone(),
                token_sha256: row.token_sha256.clone(),
            };
            let outcome = resolve_and_cache_tsa_verification(
                pool,
                chain_id,
                &snapshot.merkle_root,
                &row.tsa_token,
                &cache,
            )
            .await
            .map_err(|_| ApiError::Internal)?;

            let tsa_status = if outcome.is_failure() {
                TsaStatus::Failed
            } else {
                // Unavailable does not fail the proof envelope; token is present.
                TsaStatus::Verified
            };

            let mut json = json!({
                "timestamp": row.tsa_timestamp,
                "serial": row.tsa_serial,
                "token_bytes": row.tsa_token.len() as i64,
                "verification_status": outcome.as_str(),
            });
            if let Some(reason) = outcome.reason {
                json.as_object_mut()
                    .expect("tsa json object")
                    .insert(
                        "verification_reason".to_string(),
                        json!(reason.as_str()),
                    );
            }
            (tsa_status, Some(outcome.status), Some(json))
        }
    };

    let context = proof_context_with_tsa(
        snapshot.context.clone(),
        verification_status.is_some(),
        validation_status,
    );
    let status = derive_proof_status(&context);

    Ok(ResolvedProofState {
        status,
        tsa: tsa_json,
        context,
        resolved_root: snapshot.merkle_root.clone(),
    })
}

/// Runtime failure condition 4 (Stage 4 §3 PR2). Separate from conditions 1+2.
pub(crate) fn tsa_validation_failure_signal(
    tsa_row_present: bool,
    validation_status: TsaStatus,
) -> bool {
    tsa_row_present && validation_status == TsaStatus::Failed
}

fn proof_context_with_tsa(
    base: ProofContext,
    tsa_row_present: bool,
    validation_status: TsaStatus,
) -> ProofContext {
    ProofContext {
        failure_signal: base.failure_signal
            || tsa_validation_failure_signal(tsa_row_present, validation_status),
        ..base
    }
}

#[derive(Debug, sqlx::FromRow)]
struct TsaRow {
    tsa_timestamp: i64,
    tsa_serial: String,
    tsa_token: Vec<u8>,
    verification_status: Option<String>,
    token_sha256: Option<String>,
}

async fn load_tsa_row_for_root(
    pool: &PgPool,
    chain_id: Uuid,
    merkle_root: &str,
) -> Result<Option<TsaRow>, sqlx::Error> {
    sqlx::query_as::<_, TsaRow>(
        r#"
        SELECT tsa_timestamp, tsa_serial, tsa_token,
               verification_status, token_sha256
        FROM tsa_tokens
        WHERE chain_id = $1 AND merkle_root = $2
        "#,
    )
    .bind(chain_id)
    .bind(merkle_root)
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::v1::proof_material::proof_context_from_parts;
    use crate::tsa::{create_stub_attestation, verify_token_fresh};

    #[test]
    fn tsa_validation_failure_signal_absent_row_is_not_failure() {
        assert!(!tsa_validation_failure_signal(false, TsaStatus::Failed));
    }

    #[test]
    fn tsa_validation_failure_signal_valid_status_is_not_failure() {
        assert!(!tsa_validation_failure_signal(true, TsaStatus::Verified));
    }

    #[test]
    fn tsa_validation_failure_signal_failed_status_is_failure() {
        assert!(tsa_validation_failure_signal(true, TsaStatus::Failed));
    }

    #[test]
    fn proof_context_with_tsa_valid_signature_and_no_row_is_not_failure() {
        let base = proof_context_from_parts(true, true, true);
        let merged = proof_context_with_tsa(base, false, TsaStatus::NotProvided);
        assert!(!merged.failure_signal);
        assert_eq!(derive_proof_status(&merged), ProofStatus::Anchored);
    }

    #[test]
    fn stub_verification_status_maps_to_api_strings() {
        assert_eq!(TsaVerificationStatus::Verified.as_str(), "verified");
        assert_eq!(
            TsaVerificationStatus::VerifiedCached.as_str(),
            "verified_cached"
        );
        assert_eq!(TsaVerificationStatus::Failed.as_str(), "failed");
        assert_eq!(TsaVerificationStatus::Unavailable.as_str(), "unavailable");
    }

    #[test]
    fn stub_token_fresh_verify_respects_dev_gate() {
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let root = "ee".repeat(32);
        let att = create_stub_attestation(&root, "stub");
        let token = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            att.raw_token_b64.trim(),
        )
        .expect("stub token bytes");

        std::env::remove_var("DEV_MODE");
        std::env::remove_var("APP_ENV");
        assert_eq!(
            verify_token_fresh(&token, &root).status,
            TsaVerificationStatus::Failed
        );

        std::env::set_var("DEV_MODE", "true");
        assert_eq!(
            verify_token_fresh(&token, &root).status,
            TsaVerificationStatus::Verified
        );
        std::env::remove_var("DEV_MODE");
    }
}
