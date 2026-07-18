//! Input validation for public verification endpoints (Stage 6.6).

use crate::api::v1::file_verification::normalize_query_file_hash;

pub use crate::public_proof::validate_public_proof_id;

/// Validates and normalizes a required `file_hash` query parameter.
pub fn validate_public_file_hash(raw: Option<String>) -> Result<String, ()> {
    normalize_query_file_hash(raw).and_then(|opt| opt.ok_or(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_proof::generate_public_id;

    #[test]
    fn rejects_short_and_long_hashes() {
        assert!(validate_public_file_hash(Some("abc".into())).is_err());
        assert!(validate_public_file_hash(Some("a".repeat(63))).is_err());
        assert!(validate_public_file_hash(Some("a".repeat(65))).is_err());
    }

    #[test]
    fn accepts_uppercase_hex_after_normalization() {
        let lower = "a".repeat(64);
        let upper = lower.to_uppercase();
        assert_eq!(
            validate_public_file_hash(Some(upper)).expect("valid"),
            lower
        );
    }

    #[test]
    fn rejects_sql_injection_payload() {
        assert!(validate_public_file_hash(Some("'; DROP TABLE events;--".into())).is_err());
    }

    #[test]
    fn generated_public_ids_match_validator() {
        for _ in 0..256 {
            let id = generate_public_id();
            assert!(validate_public_proof_id(&id), "generator output must pass validator: {id}");
        }
    }

    #[test]
    fn rejects_invalid_public_proof_id_shapes() {
        assert!(!validate_public_proof_id("not-a-valid-id"));
        assert!(!validate_public_proof_id("pv_"));
        assert!(!validate_public_proof_id("pv_test123"));
        assert!(!validate_public_proof_id(&"a".repeat(64)));
    }
}
