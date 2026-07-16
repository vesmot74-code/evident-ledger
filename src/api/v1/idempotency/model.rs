use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

/// Matches `accounts.account_id` / future `idempotency_records.account_id` (UUID).
pub type AccountId = Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyRecord {
    pub id: Uuid,
    pub account_id: AccountId,
    pub idempotency_key: String,
    pub request_hash: String,
    pub response_json: Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl IdempotencyRecord {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn record_holds_required_fields() {
        let now = Utc::now();
        let record = IdempotencyRecord {
            id: Uuid::new_v4(),
            account_id: Uuid::new_v4(),
            idempotency_key: "key-1".into(),
            request_hash: "abc123".into(),
            response_json: serde_json::json!({ "event_id": "e1" }),
            created_at: now,
            expires_at: now + Duration::hours(24),
        };

        assert_eq!(record.idempotency_key, "key-1");
        assert_eq!(record.request_hash, "abc123");
        assert!(!record.is_expired(now + Duration::hours(1)));
        assert!(record.is_expired(now + Duration::hours(25)));
    }
}
