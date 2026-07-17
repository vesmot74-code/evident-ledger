//! File hash claim verification for `GET /v1/verify/{event_id}` (Stage 5.4).
//!
//! Compares a caller-provided hash against the stored `event.file_hash` without
//! revealing the stored value in API responses.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVerification {
    pub provided: bool,
    pub provided_hash: Option<String>,
    pub is_valid_file_hash: Option<bool>,
}

/// Normalizes and validates an optional `file_hash` query parameter.
///
/// Returns `Ok(None)` when the parameter is absent. Returns `Ok(Some(hash))` when
/// present and valid after `trim()` + lowercase. Returns `Err(())` on invalid format.
pub fn normalize_query_file_hash(raw: Option<String>) -> Result<Option<String>, ()> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.len() != 64 {
        return Err(());
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(());
    }
    Ok(Some(normalized))
}

/// Compares caller-provided hash (already normalized) against stored canonical hash.
///
/// `stored` is used only for comparison — never serialized in responses.
pub fn verify_file_hash(provided: Option<String>, stored: &str) -> FileVerification {
    match provided {
        None => FileVerification {
            provided: false,
            provided_hash: None,
            is_valid_file_hash: None,
        },
        Some(hash) if hash == stored => FileVerification {
            provided: true,
            provided_hash: Some(hash),
            is_valid_file_hash: Some(true),
        },
        Some(hash) => FileVerification {
            provided: true,
            provided_hash: Some(hash),
            is_valid_file_hash: Some(false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORED: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn absent_query_is_not_provided() {
        let file = verify_file_hash(None, STORED);
        assert!(!file.provided);
        assert!(file.provided_hash.is_none());
        assert!(file.is_valid_file_hash.is_none());
    }

    #[test]
    fn matching_hash_is_valid() {
        let file = verify_file_hash(Some(STORED.to_string()), STORED);
        assert!(file.provided);
        assert_eq!(file.provided_hash.as_deref(), Some(STORED));
        assert_eq!(file.is_valid_file_hash, Some(true));
    }

    #[test]
    fn mismatched_hash_is_invalid() {
        let other = "a".repeat(64);
        let file = verify_file_hash(Some(other.clone()), STORED);
        assert!(file.provided);
        assert_eq!(file.provided_hash.as_deref(), Some(other.as_str()));
        assert_eq!(file.is_valid_file_hash, Some(false));
    }

    #[test]
    fn normalize_accepts_uppercase_hex() {
        let upper = STORED.to_uppercase();
        let normalized = normalize_query_file_hash(Some(upper)).expect("valid");
        assert_eq!(normalized.as_deref(), Some(STORED));
    }

    #[test]
    fn normalize_rejects_non_hex() {
        assert!(normalize_query_file_hash(Some("zz".repeat(32))).is_err());
    }

    #[test]
    fn normalize_rejects_wrong_length() {
        assert!(normalize_query_file_hash(Some("abc".to_string())).is_err());
        assert!(normalize_query_file_hash(Some("a".repeat(63))).is_err());
    }

    #[test]
    fn normalize_rejects_empty_string() {
        assert!(normalize_query_file_hash(Some(String::new())).is_err());
        assert!(normalize_query_file_hash(Some("   ".to_string())).is_err());
    }
}
