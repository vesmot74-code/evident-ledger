use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ProofData {
    pub chain_id: String,
    pub head_event_id: String,
    pub events: Vec<EventSummary>,
    pub root: String,
    pub signature: String,
    pub public_key: String,
    pub tsa: Option<TsaData>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct EventSummary {
    pub event_id: String,
    pub file_hash: String,
    pub sequence: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TsaData {
    pub timestamp: i64,
    pub serial: String,
    pub token_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct VerificationContext {
    pub is_valid: bool,
    pub verified_at: DateTime<Utc>,
    pub first_failure_sequence: Option<i64>,
    pub first_failure_error: Option<String>,
}
