//! Shared proof status resolution for `GET /v1/proof` and future `GET /v1/verify`.
//!
//! Single source of truth for TSA load, `ProofContext` assembly, and
//! `derive_proof_status` — both endpoints must use [`resolve_proof_state`].

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::tsa::{verify_tsa_attestation, TsaAttestation, TsaStatus, TsaTrustLevel};

use super::errors::ApiError;
use super::event_access::Event;
use super::proof_material::ProofSnapshot;
use super::proof_status::{derive_proof_status, ProofContext, ProofStatus};

pub struct ResolvedProofState {
    pub status: ProofStatus,
    pub tsa: Option<Value>,
    pub context: ProofContext,
}

/// Loads TSA state, merges failure signals, and derives API `proof_status`.
///
/// TSA semantics (unchanged from Stage 4 §3 PR2):
/// - absent row → `TsaStatus::NotProvided`
/// - stub valid → `Verified`
/// - stub corrupt → `Failed`
/// - external RFC3161 row → not converted to stub attestation (PR3 scope)
pub async fn resolve_proof_state(
    pool: &PgPool,
    chain_id: Uuid,
    _event: &Event,
    snapshot: &ProofSnapshot,
) -> Result<ResolvedProofState, ApiError> {
    // Latency impact not measured in Stage 4; if proof read paths become a
    // bottleneck, consider caching TSA validation keyed by (chain_id, merkle_root).
    let tsa_row = load_tsa_row_for_root(pool, chain_id, &snapshot.merkle_root)
        .await
        .map_err(|_| ApiError::Internal)?;
    let stub_attestation = tsa_row.as_ref().and_then(tsa_attestation_from_stub_row);
    let validation_status = stub_attestation
        .as_ref()
        .map(|att| tsa_validation_status(att, &snapshot.merkle_root))
        .unwrap_or(TsaStatus::NotProvided);
    let context = proof_context_with_tsa(
        snapshot.context.clone(),
        stub_attestation.is_some(),
        validation_status,
    );
    let status = derive_proof_status(&context);
    let tsa = tsa_row.map(|t| {
        json!({
            "timestamp": t.tsa_timestamp,
            "serial": t.tsa_serial,
            "token_bytes": t.tsa_token.len() as i64,
        })
    });

    Ok(ResolvedProofState {
        status,
        tsa,
        context,
    })
}

/// Runtime failure condition 4 (Stage 4 §3 PR2). Separate from conditions 1+2.
pub(crate) fn tsa_validation_failure_signal(
    tsa_row_present: bool,
    validation_status: TsaStatus,
) -> bool {
    tsa_row_present && validation_status == TsaStatus::Failed
}

fn tsa_validation_status(att: &TsaAttestation, bundle_hash: &str) -> TsaStatus {
    verify_tsa_attestation(att, bundle_hash)
}

/// Evident stub tokens are JSON objects from `create_stub_attestation` only.
/// RFC3161 DER (FreeTSA) rows are not validated for failure_signal in PR2 (PR3).
fn is_evident_stub_json_token(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.contains("\"stub\":true"))
}

fn stub_sha256_from_token_bytes(token: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(token).ok()?;
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    payload.get("sha256")?.as_str().map(str::to_string)
}

fn tsa_attestation_from_stub_row(row: &TsaRow) -> Option<TsaAttestation> {
    if !is_evident_stub_json_token(&row.tsa_token) {
        return None;
    }
    let tsr_hash = stub_sha256_from_token_bytes(&row.tsa_token)?;
    Some(TsaAttestation {
        provider: "stub".to_string(),
        timestamp: row.tsa_timestamp,
        tsr_hash,
        // signature_valid=true here does NOT mean an Ed25519/cryptographic signature
        // was verified — stub tokens have no such signature. It signals only that
        // this material is eligible for the stub verification path
        // (validate_stub_token), which checks binding via JSON content instead.
        // External RFC3161 tokens are not validated in this PR (see PR3).
        signature_valid: true,
        raw_token_b64: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &row.tsa_token,
        ),
        trust_level: TsaTrustLevel::Stub,
    })
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
}

async fn load_tsa_row_for_root(
    pool: &PgPool,
    chain_id: Uuid,
    merkle_root: &str,
) -> Result<Option<TsaRow>, sqlx::Error> {
    sqlx::query_as::<_, TsaRow>(
        r#"
        SELECT tsa_timestamp, tsa_serial, tsa_token
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
    fn tsa_validation_status_valid_stub_is_verified() {
        use crate::tsa::create_stub_attestation;

        let hash = "bb".repeat(64);
        let att = create_stub_attestation(&hash, "stub");
        assert_eq!(tsa_validation_status(&att, &hash), TsaStatus::Verified);
    }

    #[test]
    fn tsa_validation_status_invalid_stub_is_failed() {
        use crate::tsa::create_stub_attestation;

        let hash = "cc".repeat(64);
        let mut att = create_stub_attestation(&hash, "stub");
        att.tsr_hash = "dd".repeat(64);
        assert_eq!(tsa_validation_status(&att, &hash), TsaStatus::Failed);
    }

    #[test]
    fn tsa_attestation_from_stub_row_uses_hash_from_token_not_lookup_key() {
        use crate::tsa::create_stub_attestation;

        let merkle_root = "ee".repeat(64);
        let att = create_stub_attestation(&merkle_root, "stub");
        let token_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            att.raw_token_b64.trim(),
        )
        .unwrap();
        let row = TsaRow {
            tsa_timestamp: att.timestamp,
            tsa_serial: "stub-serial".to_string(),
            tsa_token: token_bytes,
        };

        let parsed = tsa_attestation_from_stub_row(&row).expect("stub row");
        assert_eq!(parsed.tsr_hash, merkle_root);

        let wrong_merkle = "ff".repeat(64);
        assert_eq!(
            tsa_validation_status(&parsed, &wrong_merkle),
            TsaStatus::Failed
        );
    }

    #[test]
    fn non_stub_tsa_row_does_not_produce_stub_attestation() {
        let row = TsaRow {
            tsa_timestamp: 1,
            tsa_serial: "external".to_string(),
            tsa_token: vec![0x30, 0x03, 0x01, 0x01],
        };
        assert!(tsa_attestation_from_stub_row(&row).is_none());
    }

    #[test]
    fn proof_context_with_tsa_valid_signature_and_no_row_is_not_failure() {
        let base = proof_context_from_parts(true, true, true);
        let merged = proof_context_with_tsa(base, false, TsaStatus::NotProvided);
        assert!(!merged.failure_signal);
        assert_eq!(derive_proof_status(&merged), ProofStatus::Anchored);
    }
}
