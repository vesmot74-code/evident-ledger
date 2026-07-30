use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TsaAttestation {
    pub provider: String,
    pub timestamp: i64,
    pub tsr_hash: String,
    pub signature_valid: bool,
    pub raw_token_b64: String,
    pub trust_level: TsaTrustLevel,
}

/// Distinguishes a deterministic offline simulation from a real
/// external TSA provider response. `signature_valid` is only a
/// cryptographic guarantee when `trust_level == External`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TsaTrustLevel {
    Stub,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TsaStatus {
    Verified,
    Failed,
    NotProvided,
}

/// Read-path TSA verification outcome exposed on proof/verify APIs.
///
/// - `Verified` — fresh cryptographic check just succeeded
/// - `VerifiedCached` — prior check reused (`token_sha256` match + status verified)
/// - `Failed` — token invalid, stub outside dev, or OpenSSL reject
/// - `Unavailable` — imprint OK but trust bundle / OpenSSL cannot run
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TsaVerificationStatus {
    Verified,
    VerifiedCached,
    Failed,
    Unavailable,
}

impl TsaVerificationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::VerifiedCached => "verified_cached",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed)
    }

    /// Persistable cache value (never stores `verified_cached`).
    pub fn cache_value(self) -> Option<&'static str> {
        match self {
            Self::Verified | Self::VerifiedCached => Some("verified"),
            Self::Failed => Some("failed"),
            Self::Unavailable => Some("unavailable"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TsaJobState {
    Pending,
    Sent,
    Verified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TsaJob {
    pub repo: String,
    pub bundle_hash: String,
    pub state: TsaJobState,
    pub attestation: Option<TsaAttestation>,
    pub error: Option<String>,
}
