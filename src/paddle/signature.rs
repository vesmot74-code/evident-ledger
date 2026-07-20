//! Paddle Billing webhook signature verification (HMAC-SHA256).

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify `Paddle-Signature` header against raw request body bytes.
/// Signed payload format: `{ts}:{raw_body}` (UTF-8).
pub fn verify_paddle_signature(raw_body: &[u8], signature_header: &str, secret: &str) -> bool {
    let Some((timestamp, signatures)) = parse_signature_header(signature_header) else {
        return false;
    };

    let body_str = match std::str::from_utf8(raw_body) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let signed_payload = format!("{timestamp}:{body_str}");
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(signed_payload.as_bytes());
    let computed = mac.finalize().into_bytes();

    signatures.iter().any(|sig_hex| {
        let Ok(expected) = hex::decode(sig_hex) else {
            return false;
        };
        constant_time_eq(&computed, &expected)
    })
}

fn parse_signature_header(header: &str) -> Option<(String, Vec<String>)> {
    let mut timestamp = None;
    let mut signatures = Vec::new();

    for part in header.split(';') {
        let part = part.trim();
        if let Some(ts) = part.strip_prefix("ts=") {
            timestamp = Some(ts.to_string());
        } else if let Some(h1) = part.strip_prefix("h1=") {
            signatures.push(h1.to_string());
        }
    }

    let ts = timestamp?;
    if signatures.is_empty() {
        return None;
    }
    Some((ts, signatures))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Build a valid `Paddle-Signature` header for tests and integration tests.
pub fn sign_payload_for_test(secret: &str, raw_body: &str, timestamp: i64) -> String {
    let signed_payload = format!("{timestamp}:{raw_body}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(signed_payload.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    format!("ts={timestamp};h1={digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_valid_signature() {
        let secret = "test-secret";
        let body = r#"{"event_id":"evt_1","event_type":"subscription.created"}"#;
        let header = sign_payload_for_test(secret, body, 1_700_000_000);
        assert!(verify_paddle_signature(body.as_bytes(), &header, secret));
    }

    #[test]
    fn rejects_tampered_body() {
        let secret = "test-secret";
        let body = r#"{"event_id":"evt_1"}"#;
        let header = sign_payload_for_test(secret, body, 1_700_000_000);
        assert!(!verify_paddle_signature(
            b"{\"event_id\":\"evt_2\"}",
            &header,
            secret
        ));
    }
}
