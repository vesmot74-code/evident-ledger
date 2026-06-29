use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitEventRequest {
    pub chain_id: Uuid,
    pub file_hash: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub parent_event_id: Option<Uuid>,
    #[serde(default)]
    pub signature: Option<String>,
}
