use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitEventRequest {
    pub chain_id: Uuid,
    pub file_hash: String,
    pub idempotency_key: String,
    pub parent_event_id: Option<Uuid>, // None only for genesis
    pub signature: String,             // always required
}
