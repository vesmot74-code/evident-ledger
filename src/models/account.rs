//! Account model fields relevant to web authentication (Stage 8.3.0).

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Subset of `accounts` row fields used by web auth flows.
#[derive(Debug, Clone)]
pub struct AccountWebAuthFields {
    pub account_id: Uuid,
    pub email: String,
    pub password_hash: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
}
