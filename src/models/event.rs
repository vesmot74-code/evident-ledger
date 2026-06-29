use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitEventRequest {
    pub chain_id: Uuid,
    pub parent_event_id: Uuid,
    pub file_hash: String,
    pub idempotency_key: String,
    pub signature: String,
}
