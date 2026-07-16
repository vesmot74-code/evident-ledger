use crate::db::EventRow;
use crate::merkle::MerkleTree;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub chain_id: Uuid,
    pub valid: bool,
    pub blocks: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub head_event_id: Option<Uuid>,
    pub merkle_recomputed: String,
    pub merkle_match: bool,
    pub last_event_created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn verify_chain_hardened(
    pool: &PgPool,
    chain_id: Uuid,
) -> Result<VerificationReport, sqlx::Error> {
    let records = sqlx::query!(
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await?;

    let mut report = VerificationReport {
        chain_id,
        valid: true,
        blocks: records.len(),
        errors: Vec::new(),
        warnings: Vec::new(),
        head_event_id: None,
        merkle_recomputed: String::new(),
        merkle_match: false,
        last_event_created_at: None,
    };

    if records.is_empty() {
        report.warnings.push("Chain is empty".to_string());
        report.valid = false;
        report.errors.push("Chain is empty".to_string());
        return Ok(report);
    }

    let events: Vec<EventRow> = records
        .iter()
        .map(|r| EventRow {
            event_id: r.event_id,
            parent_event_id: r.parent_event_id,
            file_hash: r.file_hash.clone(),
            created_at: r.created_at,
            sequence: r.sequence,
        })
        .collect();

    // === ПРОВЕРКА 1–2: parent chain + sequence (shared checker) ===
    if let Err(failure) = crate::service::verification::check_event_structure(&events) {
        report.valid = false;
        report
            .errors
            .push(format!("Structural check failed: {failure:?}"));
    }

    // === ПРОВЕРКА 3: Монотонность времени ===
    for i in 1..events.len() {
        if events[i].created_at < events[i - 1].created_at {
            report.warnings.push(format!(
                "Time regression: {} -> {}",
                events[i - 1].created_at,
                events[i].created_at
            ));
        }
    }

    // === ПРОВЕРКА 4: MERKLE ROOT RECOMPUTE ===
    let recomputed_root = crate::service::verification::check_event_structure(&events)
        .unwrap_or_else(|_| MerkleTree::recompute_root_from_events(&events));
    report.merkle_recomputed = recomputed_root.clone();

    report.head_event_id = events.last().map(|e| e.event_id);
    report.last_event_created_at = events.last().map(|e| e.created_at);

    Ok(report)
}
