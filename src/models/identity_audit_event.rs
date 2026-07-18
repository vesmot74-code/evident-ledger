//! Identity key audit event model (Stage 9.6).

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct IdentityAuditEvent {
    pub id: Uuid,
    pub key_id: Uuid,
    pub actor_type: String,
    pub actor_id: Uuid,
    pub action: String,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}
