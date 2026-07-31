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

/// Additive diagnostic for API clients; does not change `verification_status` vocabulary.
///
/// Distinguishes deployment misconfiguration from crypto failure / external outage
/// without introducing a new top-level status (backward compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TsaVerificationReason {
    TrustMaterialMissing,
    TrustMaterialInvalid,
    TsaNetworkUnavailable,
    VerificationFailed,
}

impl TsaVerificationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrustMaterialMissing => "trust_material_missing",
            Self::TrustMaterialInvalid => "trust_material_invalid",
            Self::TsaNetworkUnavailable => "tsa_network_unavailable",
            Self::VerificationFailed => "verification_failed",
        }
    }
}

/// Read-path verification result: stable status + optional reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TsaVerificationOutcome {
    pub status: TsaVerificationStatus,
    pub reason: Option<TsaVerificationReason>,
}

impl TsaVerificationOutcome {
    pub fn verified() -> Self {
        Self {
            status: TsaVerificationStatus::Verified,
            reason: None,
        }
    }

    pub fn verified_cached() -> Self {
        Self {
            status: TsaVerificationStatus::VerifiedCached,
            reason: None,
        }
    }

    pub fn failed(reason: TsaVerificationReason) -> Self {
        Self {
            status: TsaVerificationStatus::Failed,
            reason: Some(reason),
        }
    }

    pub fn unavailable(reason: TsaVerificationReason) -> Self {
        Self {
            status: TsaVerificationStatus::Unavailable,
            reason: Some(reason),
        }
    }

    pub fn as_str(self) -> &'static str {
        self.status.as_str()
    }

    pub fn is_failure(self) -> bool {
        self.status.is_failure()
    }

    pub fn cache_value(self) -> Option<&'static str> {
        self.status.cache_value()
    }
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
