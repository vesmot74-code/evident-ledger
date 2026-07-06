use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SacTarget {
    ChainId(String),
    DocumentHash(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SacChainState {
    pub chain_id: String,
    pub merkle_root: String,
    pub head_event_id: String,
    /// Момент последнего события цепочки — источник времени ledger,
    /// НЕ TSA. См. `tsa` ниже для внешней временной метки.
    pub last_event_timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SacTsaStatus {
    /// TSA-токен найден в tsa_tokens для текущего merkle_root
    Present,
    /// TSA не запрашивался или запрос не удался (freetsa.org недоступен и т.п.)
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SacTsaSnapshot {
    pub status: SacTsaStatus,
    pub provider: Option<String>,
    pub timestamp: Option<i64>,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SacVerificationStatus {
    Verified,
    NotFound,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SacVerification {
    pub status: SacVerificationStatus,
    pub signature: Option<String>,
    pub public_key_fingerprint: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SacExclusions {
    pub content_not_verified: bool,
    pub authorship_not_verified: bool,
    pub future_chain_state_not_guaranteed: bool,
}

impl Default for SacExclusions {
    fn default() -> Self {
        Self {
            content_not_verified: true,
            authorship_not_verified: true,
            future_chain_state_not_guaranteed: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SacDocument {
    pub version: String,
    pub issued_at: String,
    pub target: SacTarget,
    pub state: Option<SacChainState>,
    pub tsa: Option<SacTsaSnapshot>,
    pub verification: SacVerification,
    pub exclusions: SacExclusions,
}
