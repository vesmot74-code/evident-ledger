//! API key generation and lookup hashing (Stage 8.1).

use rand::RngCore;
use sha2::{Digest, Sha256};

pub const API_KEY_PREFIX: &str = "ev_";
pub const SECRET_HEX_LEN: usize = 32;
pub const KEY_PREFIX_DISPLAY_LEN: usize = 8;

/// Stored in `api_keys.key_prefix` for rows created before Stage 8.1 (prefix not recoverable).
pub const LEGACY_KEY_PREFIX_STORED: &str = "legacy:no-prefix";

/// Shown in `GET /accounts/api-keys` for [`LEGACY_KEY_PREFIX_STORED`] rows.
pub const LEGACY_KEY_PREFIX_DISPLAY: &str = "legacy key — prefix unavailable";

/// First migration backfill used this sentinel; normalized at read time like [`LEGACY_KEY_PREFIX_STORED`].
const LEGACY_KEY_PREFIX_STORED_V1: &str = "ev_legacy";

#[derive(Debug, Clone)]
pub struct GeneratedApiKey {
    pub full_key: String,
    pub key_hash: String,
    pub key_prefix: String,
}

pub fn generate_api_key() -> GeneratedApiKey {
    let mut secret_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut secret_bytes);
    let secret = hex::encode(secret_bytes);
    build_from_secret(&secret)
}

fn build_from_secret(secret: &str) -> GeneratedApiKey {
    let full_key = format!("{API_KEY_PREFIX}{secret}");
    GeneratedApiKey {
        key_prefix: display_prefix(&full_key),
        key_hash: hash_secret_hex(secret),
        full_key,
    }
}

pub fn hash_secret_hex(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Lookup hash for `X-API-KEY`: `ev_<secret>` hashes the secret only; legacy keys hash the full string.
pub fn hash_api_key_for_lookup(raw_key: &str) -> String {
    if let Some(secret) = raw_key.strip_prefix(API_KEY_PREFIX) {
        if secret.len() == SECRET_HEX_LEN && secret.chars().all(|c| c.is_ascii_hexdigit()) {
            return hash_secret_hex(secret);
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn display_prefix(full_key: &str) -> String {
    let visible = full_key.chars().take(KEY_PREFIX_DISPLAY_LEN).collect::<String>();
    format!("{visible}…")
}

/// Maps a stored `key_prefix` to the value returned by account APIs.
pub fn key_prefix_for_listing(stored: &str) -> String {
    if stored == LEGACY_KEY_PREFIX_STORED || stored == LEGACY_KEY_PREFIX_STORED_V1 {
        LEGACY_KEY_PREFIX_DISPLAY.to_string()
    } else {
        stored.to_string()
    }
}

/// Whether lookup uses legacy full-string hashing (pre-Stage 8.1 keys).
pub fn is_legacy_lookup_key(raw_key: &str) -> bool {
    !(raw_key.starts_with(API_KEY_PREFIX)
        && raw_key.len() == API_KEY_PREFIX.len() + SECRET_HEX_LEN
        && raw_key[API_KEY_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_expected_shape() {
        let key = generate_api_key();
        assert!(key.full_key.starts_with(API_KEY_PREFIX));
        assert_eq!(
            key.full_key.len(),
            API_KEY_PREFIX.len() + SECRET_HEX_LEN
        );
        assert_eq!(key.key_hash, hash_secret_hex(&key.full_key[API_KEY_PREFIX.len()..]));
        assert!(key.key_prefix.starts_with("ev_"));
    }

    #[test]
    fn lookup_hashes_secret_not_full_key_for_ev_prefix() {
        let secret = "a".repeat(SECRET_HEX_LEN);
        let full = format!("{API_KEY_PREFIX}{secret}");
        let secret_hash = hash_secret_hex(&secret);
        let full_hash = {
            let mut hasher = Sha256::new();
            hasher.update(full.as_bytes());
            hex::encode(hasher.finalize())
        };
        assert_eq!(hash_api_key_for_lookup(&full), secret_hash);
        assert_ne!(secret_hash, full_hash);
    }

    #[test]
    fn legacy_keys_hash_full_string() {
        let legacy = "plain-dev-key";
        let mut hasher = Sha256::new();
        hasher.update(legacy.as_bytes());
        let expected = hex::encode(hasher.finalize());
        assert_eq!(hash_api_key_for_lookup(legacy), expected);
        assert!(is_legacy_lookup_key(legacy));
    }

    #[test]
    fn legacy_prefix_sentinels_map_to_display_label() {
        assert_eq!(
            key_prefix_for_listing(LEGACY_KEY_PREFIX_STORED),
            LEGACY_KEY_PREFIX_DISPLAY
        );
        assert_eq!(
            key_prefix_for_listing(LEGACY_KEY_PREFIX_STORED_V1),
            LEGACY_KEY_PREFIX_DISPLAY
        );
        assert_eq!(key_prefix_for_listing("ev_abcd1234…"), "ev_abcd1234…");
    }
}
