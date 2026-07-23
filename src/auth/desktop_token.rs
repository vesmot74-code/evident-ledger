//! Desktop token generation and lookup hashing (Stage 13.4).

use rand::RngCore;
use sha2::{Digest, Sha256};

pub const DESKTOP_TOKEN_PREFIX: &str = "desktop_";
pub const SECRET_HEX_LEN: usize = 32;

#[derive(Debug, Clone)]
pub struct GeneratedDesktopToken {
    pub plaintext: String,
    pub token_hash: String,
}

pub fn generate_desktop_token() -> GeneratedDesktopToken {
    let mut secret_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut secret_bytes);
    let secret = hex::encode(secret_bytes);
    GeneratedDesktopToken {
        plaintext: format!("{DESKTOP_TOKEN_PREFIX}{secret}"),
        token_hash: hash_secret_hex(&secret),
    }
}

pub fn hash_secret_hex(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Lookup hash for `Authorization: Bearer desktop_<secret>`.
pub fn hash_desktop_token_for_lookup(raw: &str) -> Option<String> {
    let secret = raw.strip_prefix(DESKTOP_TOKEN_PREFIX)?;
    if secret.len() == SECRET_HEX_LEN && secret.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hash_secret_hex(secret))
    } else {
        None
    }
}

pub fn is_desktop_bearer_token(raw: &str) -> bool {
    hash_desktop_token_for_lookup(raw).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_shape() {
        let t = generate_desktop_token();
        assert!(t.plaintext.starts_with(DESKTOP_TOKEN_PREFIX));
        assert_eq!(
            t.plaintext.len(),
            DESKTOP_TOKEN_PREFIX.len() + SECRET_HEX_LEN
        );
        assert_eq!(
            hash_desktop_token_for_lookup(&t.plaintext).as_deref(),
            Some(t.token_hash.as_str())
        );
    }

    #[test]
    fn rejects_non_desktop_tokens() {
        assert!(hash_desktop_token_for_lookup("ev_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
        assert!(hash_desktop_token_for_lookup("desktop_short").is_none());
    }
}
