//! Request metadata propagated from public verification middleware (Stage 6.6).

use crate::public_verification_audit::PublicVerificationRateLimitAction;

#[derive(Debug, Clone)]
pub struct PublicRequestMetadata {
    pub client_ip_hash: Option<String>,
    pub rate_limit_action: PublicVerificationRateLimitAction,
}
