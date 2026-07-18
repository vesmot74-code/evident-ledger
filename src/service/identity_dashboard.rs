//! Read-only identity dashboard queries (Stage 9.5).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::merkle::MerkleTree;
use crate::models::event::Event;
use crate::service::identity_verification::IdentityVerificationService;

pub struct IdentityDashboardService;

#[derive(Debug)]
pub enum IdentityDashboardError {
    KeyNotFound,
    InvalidCursor,
    Database(sqlx::Error),
    Verification(crate::service::identity_verification::IdentityVerificationError),
}

#[derive(Debug, Clone)]
pub struct IdentityKeySummary {
    pub key_id: Uuid,
    pub fingerprint: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub verified_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub events_count: i64,
}

#[derive(Debug, Clone)]
pub struct IdentityKeyEventRow {
    pub event_id: Uuid,
    pub chain_id: Uuid,
    pub sequence: i64,
    pub signed_at: DateTime<Utc>,
    pub identity_signature_valid: bool,
}

#[derive(Debug, Clone)]
pub struct IdentityKeyEventsPage {
    pub key_id: Uuid,
    pub key_status: String,
    pub events: Vec<IdentityKeyEventRow>,
    pub next_cursor: Option<String>,
}

pub fn key_status(revoked_at: Option<DateTime<Utc>>) -> &'static str {
    if revoked_at.is_some() {
        "revoked"
    } else {
        "active"
    }
}

impl IdentityDashboardService {
    pub async fn list_keys(
        db: &PgPool,
        account_id: Uuid,
    ) -> Result<Vec<IdentityKeySummary>, IdentityDashboardError> {
        let rows = sqlx::query_as::<_, KeySummaryRow>(
            r#"
            SELECT
                ik.id AS key_id,
                ik.fingerprint,
                ik.created_at,
                ik.verified_at,
                ik.revoked_at,
                COUNT(e.event_id)::bigint AS events_count
            FROM identity_keys ik
            LEFT JOIN events e ON e.identity_key_id = ik.id
            WHERE ik.account_id = $1
            GROUP BY ik.id
            ORDER BY ik.created_at ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(db)
        .await
        .map_err(IdentityDashboardError::Database)?;

        Ok(rows
            .into_iter()
            .map(|row| IdentityKeySummary {
                key_id: row.key_id,
                fingerprint: row.fingerprint,
                status: key_status(row.revoked_at).to_string(),
                created_at: row.created_at,
                verified_at: row.verified_at,
                revoked_at: row.revoked_at,
                events_count: row.events_count,
            })
            .collect())
    }

    pub async fn list_key_events(
        db: &PgPool,
        account_id: Uuid,
        key_id: Uuid,
        limit: i64,
        cursor: Option<&str>,
    ) -> Result<IdentityKeyEventsPage, IdentityDashboardError> {
        let key = sqlx::query_as::<_, KeyStatusRow>(
            r#"
            SELECT id, revoked_at
            FROM identity_keys
            WHERE id = $1 AND account_id = $2
            "#,
        )
        .bind(key_id)
        .bind(account_id)
        .fetch_optional(db)
        .await
        .map_err(IdentityDashboardError::Database)?
        .ok_or(IdentityDashboardError::KeyNotFound)?;

        let (cursor_sequence, cursor_event_id) = match cursor {
            Some(value) => decode_cursor(value).map_err(|_| IdentityDashboardError::InvalidCursor)?,
            None => (None, None),
        };

        let fetch_limit = limit + 1;
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT
                event_id,
                chain_id,
                parent_event_id,
                file_hash,
                sequence,
                created_at,
                identity_key_id,
                identity_signature,
                identity_fingerprint
            FROM events
            WHERE identity_key_id = $1
              AND (
                $2::bigint IS NULL
                OR sequence < $2
                OR (sequence = $2 AND event_id < $3)
              )
            ORDER BY sequence DESC, event_id DESC
            LIMIT $4
            "#,
        )
        .bind(key_id)
        .bind(cursor_sequence)
        .bind(cursor_event_id)
        .bind(fetch_limit)
        .fetch_all(db)
        .await
        .map_err(IdentityDashboardError::Database)?;

        let has_more = rows.len() as i64 > limit;
        let page_rows = if has_more {
            &rows[..limit as usize]
        } else {
            &rows
        };

        let mut events = Vec::with_capacity(page_rows.len());
        for row in page_rows {
            let identity_event = Event {
                event_id: row.event_id,
                chain_id: row.chain_id,
                parent_event_id: row.parent_event_id,
                file_hash: row.file_hash.clone(),
                sequence: row.sequence,
                identity_key_id: row.identity_key_id,
                identity_signature: row.identity_signature.clone(),
                identity_fingerprint: row.identity_fingerprint.clone(),
            };
            let canonical_hash = MerkleTree::build_leaf(
                row.sequence,
                &row.event_id,
                &row.parent_event_id,
                &row.file_hash,
            );
            let verification = IdentityVerificationService::verify(db, &identity_event, &canonical_hash)
                .await
                .map_err(IdentityDashboardError::Verification)?;
            events.push(IdentityKeyEventRow {
                event_id: row.event_id,
                chain_id: row.chain_id,
                sequence: row.sequence,
                signed_at: row.created_at,
                identity_signature_valid: verification.valid,
            });
        }

        let next_cursor = if has_more {
            let last = page_rows.last().expect("has_more implies non-empty page");
            Some(encode_cursor(last.sequence, last.event_id))
        } else {
            None
        };

        Ok(IdentityKeyEventsPage {
            key_id: key.id,
            key_status: key_status(key.revoked_at).to_string(),
            events,
            next_cursor,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct KeySummaryRow {
    key_id: Uuid,
    fingerprint: String,
    created_at: DateTime<Utc>,
    verified_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    events_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct KeyStatusRow {
    id: Uuid,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct EventRow {
    event_id: Uuid,
    chain_id: Uuid,
    parent_event_id: Uuid,
    file_hash: String,
    sequence: i64,
    created_at: DateTime<Utc>,
    identity_key_id: Option<Uuid>,
    identity_signature: Option<String>,
    identity_fingerprint: Option<String>,
}

pub fn encode_cursor(sequence: i64, event_id: Uuid) -> String {
    use base64::Engine;
    let payload = format!("{sequence}:{event_id}");
    base64::engine::general_purpose::STANDARD.encode(payload.as_bytes())
}

pub fn decode_cursor(cursor: &str) -> Result<(Option<i64>, Option<Uuid>), ()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cursor.trim())
        .map_err(|_| ())?;
    let payload = String::from_utf8(bytes).map_err(|_| ())?;
    let (sequence, event_id) = payload.split_once(':').ok_or(())?;
    let sequence: i64 = sequence.parse().map_err(|_| ())?;
    let event_id = Uuid::parse_str(event_id).map_err(|_| ())?;
    Ok((Some(sequence), Some(event_id)))
}

pub fn clamp_events_limit(limit: Option<u32>) -> i64 {
    match limit {
        None => 20,
        Some(value) if value == 0 => 20,
        Some(value) if value > 100 => 100,
        Some(value) => value as i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let event_id = Uuid::new_v4();
        let encoded = encode_cursor(42, event_id);
        let (seq, id) = decode_cursor(&encoded).expect("decode");
        assert_eq!(seq, Some(42));
        assert_eq!(id, Some(event_id));
    }

    #[test]
    fn key_status_active_and_revoked() {
        assert_eq!(key_status(None), "active");
        assert_eq!(key_status(Some(Utc::now())), "revoked");
    }

    #[test]
    fn clamp_events_limit_defaults_and_caps() {
        assert_eq!(clamp_events_limit(None), 20);
        assert_eq!(clamp_events_limit(Some(0)), 20);
        assert_eq!(clamp_events_limit(Some(50)), 50);
        assert_eq!(clamp_events_limit(Some(200)), 100);
    }
}
