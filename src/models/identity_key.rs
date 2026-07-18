//! User identity key model (Stage 9.1).

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct IdentityKey {
    pub id: Uuid,
    pub account_id: Uuid,
    pub public_key: String,
    pub fingerprint: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub verified_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl IdentityKey {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    pub fn can_sign(&self) -> bool {
        self.revoked_at.is_none()
    }
}
