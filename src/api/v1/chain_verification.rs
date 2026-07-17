//! Prefix-scoped chain integrity checks for `GET /v1/verify/{event_id}` (Stage 5.3).
//!
//! Pure function over in-memory prefix events — no DB or TSA access.

use uuid::Uuid;

use crate::db::EventRow;
use crate::merkle::MerkleTree;
use crate::service::verification::check_event_structure;
use crate::signing::verify_root;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainVerification {
    pub valid: bool,
    pub merkle_valid: bool,
    pub signature_valid: bool,
    pub errors: Vec<String>,
}

/// Verifies chain integrity for events `sequence <= target` using prefix semantics.
///
/// `expected_root` is the commit-time merkle root bound by the persisted signature
/// (from [`super::proof_state::ResolvedProofState::resolved_root`]).
/// Merkle validity compares a fresh recompute against that anchor; signature validity
/// checks the persisted signature against `expected_root`, not the recomputed root.
pub fn verify_chain_prefix(
    chain_id: Uuid,
    event_id: Uuid,
    event_signature: &str,
    public_key: &str,
    prefix: &[EventRow],
    expected_root: &str,
) -> ChainVerification {
    let mut errors = Vec::new();

    if let Err(failure) = check_event_structure(prefix) {
        errors.push(format!("Structural check failed: {failure:?}"));
    }

    let recomputed_root = MerkleTree::recompute_root_from_events(prefix);
    let merkle_valid = if prefix.is_empty() || recomputed_root.is_empty() {
        if !prefix.is_empty() {
            errors.push("Merkle root could not be computed".to_string());
        }
        false
    } else {
        recomputed_root == expected_root
    };

    let signature_valid = !event_signature.is_empty()
        && verify_root(
            &chain_id.to_string(),
            expected_root,
            &event_id.to_string(),
            event_signature,
            public_key,
        );

    let valid = merkle_valid && signature_valid && errors.is_empty();

    ChainVerification {
        valid,
        merkle_valid,
        signature_valid,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing::ServerSigner;
    use chrono::Utc;

    fn row(id: Uuid, parent: Uuid, seq: i64, hash: &str) -> EventRow {
        EventRow {
            event_id: id,
            parent_event_id: parent,
            file_hash: hash.to_string(),
            created_at: Utc::now(),
            sequence: seq,
        }
    }

    fn anchored_fixture() -> (ServerSigner, Uuid, Uuid, Vec<EventRow>, String, String) {
        let signer = ServerSigner::load_or_create("target/test_chain_verify_signing.key");
        let chain_id = Uuid::new_v4();
        let e1 = Uuid::new_v4();
        let prefix = vec![row(e1, Uuid::nil(), 1, &"aa".repeat(32))];
        let expected_root = MerkleTree::recompute_root_from_events(&prefix);
        let signature = signer.sign_root(&chain_id.to_string(), &expected_root, &e1.to_string());
        (signer, chain_id, e1, prefix, expected_root, signature)
    }

    #[test]
    fn valid_prefix_is_fully_valid() {
        let (signer, chain_id, e1, prefix, expected_root, signature) = anchored_fixture();
        let chain = verify_chain_prefix(
            chain_id,
            e1,
            &signature,
            &signer.public_key_hex(),
            &prefix,
            &expected_root,
        );
        assert!(chain.valid);
        assert!(chain.merkle_valid);
        assert!(chain.signature_valid);
        assert!(chain.errors.is_empty());
    }

    #[test]
    fn broken_parent_chain_fails_valid_with_errors() {
        let (signer, chain_id, e1, mut prefix, expected_root, signature) = anchored_fixture();
        prefix[0].parent_event_id = Uuid::new_v4();

        let chain = verify_chain_prefix(
            chain_id,
            e1,
            &signature,
            &signer.public_key_hex(),
            &prefix,
            &expected_root,
        );
        assert!(!chain.valid);
        // Parent is part of the merkle leaf; commit anchor no longer matches recomputed prefix.
        assert!(!chain.merkle_valid);
        assert!(chain.signature_valid);
        assert!(!chain.errors.is_empty());
    }

    #[test]
    fn broken_merkle_keeps_signature_check_independent() {
        let (signer, chain_id, e1, mut prefix, expected_root, signature) = anchored_fixture();
        prefix[0].file_hash = "bb".repeat(32);

        let chain = verify_chain_prefix(
            chain_id,
            e1,
            &signature,
            &signer.public_key_hex(),
            &prefix,
            &expected_root,
        );
        assert!(!chain.valid);
        assert!(!chain.merkle_valid);
        assert!(chain.signature_valid);
        assert!(chain.errors.is_empty());
    }

    #[test]
    fn broken_signature_keeps_merkle_check_independent() {
        let (signer, chain_id, e1, prefix, expected_root, _signature) = anchored_fixture();

        let chain = verify_chain_prefix(
            chain_id,
            e1,
            &"aa".repeat(64),
            &signer.public_key_hex(),
            &prefix,
            &expected_root,
        );
        assert!(!chain.valid);
        assert!(chain.merkle_valid);
        assert!(!chain.signature_valid);
        assert!(chain.errors.is_empty());
    }
}
