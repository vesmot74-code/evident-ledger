use sha2::{Digest, Sha256};

/// Deterministic SHA-256 hex digest of canonical JSON.
///
/// Object keys are sorted lexicographically at every nesting level.
/// Array element order is preserved. Does not use raw `serde_json::to_string`
/// on the top-level object (which would preserve input key order).
pub fn canonical_json_sha256(value: &serde_json::Value) -> String {
    let canonical = canonicalize_value(value);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hex::encode(hasher.finalize())
}

fn canonicalize_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        canonicalize_value(&serde_json::Value::String(key.clone())),
                        canonicalize_value(&map[key])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        serde_json::Value::Array(items) => {
            let body = items
                .iter()
                .map(canonicalize_value)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            serde_json::to_string(s).expect("string JSON serialization must succeed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_affect_hash() {
        let left = json!({ "b": 2, "a": 1, "nested": { "z": 9, "y": 8 } });
        let right = json!({ "a": 1, "nested": { "y": 8, "z": 9 }, "b": 2 });

        assert_eq!(canonical_json_sha256(&left), canonical_json_sha256(&right));
    }

    #[test]
    fn array_order_is_preserved() {
        let left = json!({ "items": [1, 2, 3] });
        let right = json!({ "items": [1, 3, 2] });

        assert_ne!(canonical_json_sha256(&left), canonical_json_sha256(&right));
    }

    #[test]
    fn hash_is_lowercase_hex_without_prefix() {
        let hash = canonical_json_sha256(&json!({ "a": 1 }));
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, hash.to_lowercase());
        assert!(!hash.starts_with("0x"));
    }
}
