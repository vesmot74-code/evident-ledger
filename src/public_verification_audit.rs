//! Structured audit logging for public verification endpoints (Stage 6.6).
//!
//! Logs what happened without storing file_hash, public_proof_id, raw IP, or exists.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicVerificationRequestType {
    Verify,
    CertificatePdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicVerificationOutcome {
    Success,
    NotFound,
    InvalidRequest,
    RateLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicVerificationRateLimitAction {
    Allowed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicVerificationAuditEvent {
    pub timestamp: DateTime<Utc>,
    pub request_type: PublicVerificationRequestType,
    pub outcome: PublicVerificationOutcome,
    pub rate_limit_action: PublicVerificationRateLimitAction,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip_hash: Option<String>,
}

impl PublicVerificationAuditEvent {
    pub fn new(
        request_type: PublicVerificationRequestType,
        outcome: PublicVerificationOutcome,
        rate_limit_action: PublicVerificationRateLimitAction,
        request_id: impl Into<String>,
        client_ip_hash: Option<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            request_type,
            outcome,
            rate_limit_action,
            request_id: request_id.into(),
            client_ip_hash,
        }
    }
}

static TEST_CAPTURE: OnceLock<Mutex<Vec<PublicVerificationAuditEvent>>> = OnceLock::new();

fn test_capture() -> Option<&'static Mutex<Vec<PublicVerificationAuditEvent>>> {
    TEST_CAPTURE.get()
}

/// Enable in-memory capture of audit events (integration tests only).
pub fn enable_test_capture() {
    let _ = TEST_CAPTURE.set(Mutex::new(Vec::new()));
}

pub fn take_test_events() -> Vec<PublicVerificationAuditEvent> {
    test_capture()
        .map(|lock| std::mem::take(&mut *lock.lock().expect("audit test lock")))
        .unwrap_or_default()
}

pub fn log_public_verification_audit(event: &PublicVerificationAuditEvent) {
    if let Some(lock) = test_capture() {
        lock.lock().expect("audit test lock").push(event.clone());
    }
    if let Ok(line) = serde_json::to_string(event) {
        tracing::info!(target: "public_verification_audit", "{line}");
    }
}

pub fn client_ip_hash_hex(client_key: [u8; 32]) -> String {
    client_key.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_event_serializes_required_fields() {
        let event = PublicVerificationAuditEvent::new(
            PublicVerificationRequestType::Verify,
            PublicVerificationOutcome::Success,
            PublicVerificationRateLimitAction::Allowed,
            "req-1",
            Some("abc".repeat(64)),
        );
        let json = serde_json::to_value(&event).expect("json");
        assert!(json.get("timestamp").is_some());
        assert_eq!(json["request_type"], "verify");
        assert_eq!(json["outcome"], "success");
        assert_eq!(json["rate_limit_action"], "allowed");
        assert_eq!(json["request_id"], "req-1");
        assert_eq!(json["client_ip_hash"], "abc".repeat(64));
    }
}
