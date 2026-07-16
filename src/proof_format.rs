//! Canonical proof file and Merkle leaf version identifiers.

/// Nested `proof.version` — proof artifact format version.
pub const PROOF_VERSION: &str = "proof_v1";

/// Nested `proof.type` — top-level proof mechanism type.
pub const PROOF_TYPE: &str = "merkle-root-v1";

/// Top-level `leaf_version` — Merkle leaf canonicalization version.
/// Formula: SHA256(sequence || event_id || parent_event_id || file_hash)
pub const LEAF_VERSION: &str = "leaf_v1";

pub const LEGACY_UNVERSIONED_MESSAGE: &str =
    "unversioned legacy proof format — unsupported, please regenerate";

pub const UNSUPPORTED_PROOF_FORMAT_MESSAGE: &str = "unsupported proof format";

pub const LEGACY_EXIT_CODE: i32 = 4;

pub fn is_versioned(proof_version: Option<&str>, leaf_version: Option<&str>) -> bool {
    proof_version.is_some_and(|s| !s.is_empty()) && leaf_version.is_some_and(|s| !s.is_empty())
}

pub fn is_supported(proof_version: Option<&str>, leaf_version: Option<&str>) -> bool {
    matches!(
        (proof_version, leaf_version),
        (Some(pv), Some(lv)) if pv == PROOF_VERSION && lv == LEAF_VERSION
    )
}
