use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use base64::Engine;
use notary_crypto::sha256_hex;

use crate::types::TsaAttestation;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

/// Stub attestation for tests and offline mode (JSON token accepted by notary-tsa validator).
pub fn create_stub_attestation(bundle_hash: &str, provider: &str) -> TsaAttestation {
    let token_json = format!(
        r#"{{"stub":true,"sha256":"{bundle_hash}","provider":"{provider}"}}"#
    );
    let raw_token_b64 = base64::engine::general_purpose::STANDARD.encode(token_json.as_bytes());
    TsaAttestation {
        provider: provider.to_string(),
        timestamp: unix_now(),
        tsr_hash: bundle_hash.to_string(),
        signature_valid: true,
        raw_token_b64,
    }
}

/// Simulate external TSA submission binding token to bundle hash.
pub fn submit_bundle_hash_stub(bundle_hash: &str, provider: &str) -> Result<TsaAttestation> {
    Ok(create_stub_attestation(bundle_hash, provider))
}

/// Hash of raw TSR bytes for audit metadata (external layer only).
pub fn tsr_content_hash(raw_token_b64: &str) -> String {
    sha256_hex(raw_token_b64.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::verify_tsa_attestation;
    use crate::types::TsaStatus;

    #[test]
    fn bundle_plus_tsa_success() {
        let hash = "c".repeat(64);
        let att = submit_bundle_hash_stub(&hash, "freetsa").unwrap();
        assert_eq!(verify_tsa_attestation(&att, &hash), TsaStatus::Verified);
    }
}
