use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TsaAttestation {
    pub provider: String,
    pub timestamp: i64,
    pub tsr_hash: String,
    pub signature_valid: bool,
    pub raw_token_b64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TsaStatus {
    Verified,
    Failed,
    NotProvided,
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
