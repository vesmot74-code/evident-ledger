use base64::Engine;

use super::types::{TsaAttestation, TsaStatus};

fn validate_stub_token(token_b64: &str, bundle_hash: &str) -> bool {
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(token_b64.trim()) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    text.contains("\"stub\":true") && text.contains(bundle_hash)
}

/// Verify TSA attestation against bundle hash. Failures return `Failed`, never invalidate bundle core.
pub fn verify_tsa_attestation(attestation: &TsaAttestation, bundle_hash: &str) -> TsaStatus {
    if attestation.tsr_hash != bundle_hash {
        return TsaStatus::Failed;
    }

    if !attestation.signature_valid {
        return TsaStatus::Failed;
    }

    if attestation.raw_token_b64.is_empty() {
        return TsaStatus::Failed;
    }

    if validate_stub_token(&attestation.raw_token_b64, bundle_hash) {
        TsaStatus::Verified
    } else {
        TsaStatus::Failed
    }
}

pub fn tsa_status_for_bundle(tsa: Option<&TsaAttestation>, bundle_hash: &str) -> TsaStatus {
    match tsa {
        None => TsaStatus::NotProvided,
        Some(att) => verify_tsa_attestation(att, bundle_hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsa::create_stub_attestation;

    #[test]
    fn stub_attestation_verifies_for_correct_owner() {
        let hash = "a".repeat(64);
        let att = create_stub_attestation(&hash, "stub");
        assert_eq!(verify_tsa_attestation(&att, &hash), TsaStatus::Verified);
    }

    #[test]
    fn tampered_tsr_hash_fails() {
        let hash = "a".repeat(64);
        let mut att = create_stub_attestation(&hash, "stub");
        att.tsr_hash = "b".repeat(64);
        assert_eq!(verify_tsa_attestation(&att, &hash), TsaStatus::Failed);
    }

    #[test]
    fn invalid_signature_flag_fails() {
        let hash = "a".repeat(64);
        let mut att = create_stub_attestation(&hash, "stub");
        att.signature_valid = false;
        assert_eq!(verify_tsa_attestation(&att, &hash), TsaStatus::Failed);
    }

    #[test]
    fn missing_tsa_is_not_provided() {
        assert_eq!(tsa_status_for_bundle(None, "abc"), TsaStatus::NotProvided);
    }

    #[test]
    fn tampered_tsr_token_fails_verification() {
        let hash = "a".repeat(64);
        let mut att = create_stub_attestation(&hash, "freetsa");
        att.raw_token_b64 =
            base64::engine::general_purpose::STANDARD.encode(br#"{"stub":true,"sha256":"wrong"}"#);
        assert_eq!(verify_tsa_attestation(&att, &hash), TsaStatus::Failed);
    }
}
