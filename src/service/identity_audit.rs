//! Identity key audit trail service (Stage 9.6).

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::models::identity_audit_event::IdentityAuditEvent;

pub struct IdentityAuditService;

#[derive(Debug)]
pub enum IdentityAuditError {
    InvalidActorType,
    Database(sqlx::Error),
}

impl IdentityAuditService {
    /// Append audit event to `identity_key_audit_events` within a transaction.
    pub async fn append(
        db: &mut PgConnection,
        key_id: Uuid,
        actor_type: &str,
        actor_id: Uuid,
        action: &str,
        metadata: Option<Value>,
    ) -> Result<IdentityAuditEvent, IdentityAuditError> {
        if actor_type != "account" {
            return Err(IdentityAuditError::InvalidActorType);
        }

        sqlx::query_as::<_, IdentityAuditEvent>(
            r#"
            INSERT INTO identity_key_audit_events (
                key_id, actor_type, actor_id, action, metadata
            )
            VALUES ($1, $2, $3, $4, $5)
            RETURNING
                id,
                key_id,
                actor_type,
                actor_id,
                action,
                metadata,
                created_at
            "#,
        )
        .bind(key_id)
        .bind(actor_type)
        .bind(actor_id)
        .bind(action)
        .bind(metadata)
        .fetch_one(db)
        .await
        .map_err(IdentityAuditError::Database)
    }
}
