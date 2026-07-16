//! Derived `proof_status` model for API v1.
//!
//! Proof material is assembled outside `derive_proof_status` (DB / verification layer).
//! `derive_proof_status` is pure: it maps a [`ProofContext`] to [`ProofStatus`].

use sqlx::PgConnection;
use uuid::Uuid;

use crate::signing::ServerSigner;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofStatus {
    Pending,
    Anchored,
    Failed,
}

impl ProofStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProofStatus::Pending => "pending",
            ProofStatus::Anchored => "anchored",
            ProofStatus::Failed => "failed",
        }
    }
}

/// Snapshot of proof-generation inputs for a single event, assembled upstream.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProofContext {
    pub merkle_root_present: bool,
    pub signature_present: bool,
    pub signature_valid: bool,
    /// Explicit persisted failure signal (reserved until a source exists in storage).
    pub failure_signal: bool,
}

impl ProofContext {
    /// Loads proof inputs for the commit-time snapshot of an event.
    ///
    /// Merkle root uses the prefix `sequence <= target_sequence` only.
    /// `chain_head` in the signature is `target_event_id`, not the current chain head.
    pub async fn load(
        conn: &mut PgConnection,
        signer: &ServerSigner,
        chain_id: Uuid,
        target_event_id: Uuid,
        target_sequence: i64,
    ) -> Result<Self, sqlx::Error> {
        super::proof_material::proof_context_at_event(
            conn,
            signer,
            chain_id,
            target_event_id,
            target_sequence,
        )
        .await
    }
}

/// Derives API `proof_status` from assembled proof context.
///
/// TSA availability is intentionally excluded — see `docs/API.md` §4 and SYSTEM_CONTRACT §7.
pub fn derive_proof_status(ctx: &ProofContext) -> ProofStatus {
    if ctx.failure_signal {
        return ProofStatus::Failed;
    }

    if ctx.merkle_root_present && ctx.signature_present && ctx.signature_valid {
        ProofStatus::Anchored
    } else {
        ProofStatus::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(
        merkle_root_present: bool,
        signature_present: bool,
        signature_valid: bool,
        failure_signal: bool,
    ) -> ProofContext {
        ProofContext {
            merkle_root_present,
            signature_present,
            signature_valid,
            failure_signal,
        }
    }

    #[test]
    fn empty_context_is_pending() {
        assert_eq!(derive_proof_status(&ProofContext::default()), ProofStatus::Pending);
    }

    #[test]
    fn merkle_only_is_pending() {
        assert_eq!(
            derive_proof_status(&ctx(true, false, false, false)),
            ProofStatus::Pending
        );
    }

    #[test]
    fn signature_only_is_pending() {
        assert_eq!(
            derive_proof_status(&ctx(false, true, true, false)),
            ProofStatus::Pending
        );
        assert_eq!(
            derive_proof_status(&ctx(false, true, false, false)),
            ProofStatus::Pending
        );
    }

    #[test]
    fn merkle_with_invalid_signature_is_pending() {
        assert_eq!(
            derive_proof_status(&ctx(true, true, false, false)),
            ProofStatus::Pending
        );
    }

    #[test]
    fn merkle_with_valid_signature_is_anchored() {
        assert_eq!(
            derive_proof_status(&ctx(true, true, true, false)),
            ProofStatus::Anchored
        );
    }

    #[test]
    fn failure_signal_takes_priority_over_anchored() {
        assert_eq!(
            derive_proof_status(&ctx(true, true, true, true)),
            ProofStatus::Failed,
            "failure_signal must override merkle + valid signature"
        );
    }

    #[test]
    fn failure_signal_is_failed() {
        assert_eq!(
            derive_proof_status(&ctx(true, true, true, true)),
            ProofStatus::Failed
        );
        assert_eq!(
            derive_proof_status(&ctx(false, false, false, true)),
            ProofStatus::Failed
        );
    }

    #[test]
    fn tsa_missing_with_valid_proof_is_anchored() {
        // TSA is not part of ProofContext; valid merkle + signature => anchored.
        assert_eq!(
            derive_proof_status(&ctx(true, true, true, false)),
            ProofStatus::Anchored
        );
    }

    #[test]
    fn status_serializes_to_api_snake_case_strings() {
        assert_eq!(ProofStatus::Pending.as_str(), "pending");
        assert_eq!(ProofStatus::Anchored.as_str(), "anchored");
        assert_eq!(ProofStatus::Failed.as_str(), "failed");
    }
}
