use chrono::{DateTime, Utc};

/// Presentation-only scope for event-level PDFs (Stage A1).
/// Distinguishes the single evidence item shown from the chain-head crypto snapshot.
#[derive(Debug, Clone)]
pub struct EventReportScope {
    /// e.g. `EVENT_001`
    pub evidence_item_label: String,
    /// e.g. `EVENT_003` — chain head at report generation
    pub chain_head_label: String,
    /// Number of events included in the Merkle root / signature (full chain).
    pub proof_events_count: usize,
}

#[derive(Debug, Clone)]
pub struct ProofData {
    pub chain_id: String,
    pub head_event_id: String,
    pub events: Vec<EventSummary>,
    pub root: String,
    pub signature: String,
    pub public_key: String,
    pub tsa: Option<TsaData>,
    pub created_at: Option<DateTime<Utc>>,
    /// When set, PDF renders event-level scope notes (not used for project snapshots).
    pub event_report_scope: Option<EventReportScope>,
}

#[derive(Debug, Clone)]
pub struct EventSummary {
    pub event_id: String,
    pub file_hash: String,
    pub sequence: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TsaData {
    pub timestamp: i64,
    pub serial: String,
    pub token_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct FileStatus {
    pub file_name: String,
    pub chain_valid: bool,
    pub local_integrity_ok: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct VerificationContext {
    pub is_valid: bool,
    pub verified_at: DateTime<Utc>,
    pub first_failure_sequence: Option<i64>,
    pub first_failure_error: Option<String>,
    pub files: Vec<FileStatus>,
}
