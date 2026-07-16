//! Request validation for `POST /v1/events`.
//!
//! Allowed `event_type` values are defined here as the single source for Stage 2.

const ALLOWED_EVENT_TYPES: &[&str] = &["submission", "amendment", "cancellation"];

/// Lowercase hex SHA-256: exactly 64 characters, no `0x` prefix.
pub fn is_valid_file_hash(file_hash: &str) -> bool {
    let hash = file_hash.trim();
    hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn is_valid_event_type(event_type: &str) -> bool {
    ALLOWED_EVENT_TYPES.contains(&event_type)
}

pub fn allowed_event_types() -> &'static [&'static str] {
    ALLOWED_EVENT_TYPES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_sha256_hex() {
        assert!(is_valid_file_hash(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
    }

    #[test]
    fn rejects_short_or_non_hex_file_hash() {
        assert!(!is_valid_file_hash("abc"));
        assert!(!is_valid_file_hash(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85g"
        ));
        assert!(!is_valid_file_hash(""));
    }

    #[test]
    fn accepts_allowed_event_types_only() {
        assert!(is_valid_event_type("submission"));
        assert!(is_valid_event_type("amendment"));
        assert!(is_valid_event_type("cancellation"));
        assert!(!is_valid_event_type("commit"));
        assert!(!is_valid_event_type(""));
    }
}
