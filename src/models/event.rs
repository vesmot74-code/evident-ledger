use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitEventRequest {
    pub chain_id: Uuid,
    pub file_hash: String,
    pub idempotency_key: String,
    pub parent_event_id: Option<Uuid>, // None only for genesis
    /// Pre-assigned event id (v1 submit computes hash before insert).
    pub event_id: Option<Uuid>,
    pub identity_key_id: Option<Uuid>,
    pub identity_signature: Option<String>,
    pub identity_fingerprint: Option<String>,
}

/// Persisted ledger event (subset used by identity signing pipeline).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub event_id: Uuid,
    pub chain_id: Uuid,
    pub parent_event_id: Uuid,
    pub file_hash: String,
    pub sequence: i64,
    pub identity_key_id: Option<Uuid>,
    pub identity_signature: Option<String>,
    pub identity_fingerprint: Option<String>,
}
