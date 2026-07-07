//! Hash Attestation (L1 Evidence Resolution Certificate) — independent
//! product artifact, frozen contract v1.0. Does NOT reuse SacDocument's
//! JSON shape; it aggregates over possibly-many chains that share a hash.
//! Underlying per-chain facts are still sourced from build_attestation(),
//! so there is no duplicate verification logic — only aggregation.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::sac::{SacTsaStatus, SacVerificationStatus};
use crate::service::attestation::build_attestation;
use crate::signing::ServerSigner;

pub const HASH_ATTESTATION_FORMAT_VERSION: &str = "1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashMatchEntry {
    pub chain_id: String,
    pub event_id: String,
    pub timestamp: String,

    pub merkle_root: Option<String>,
    pub head_event_id: Option<String>,

    pub verification_status: String, // "VERIFIED" | "FAILED" | "NOT FOUND"
    pub tsa_status: String,          // "PRESENT" | "MISSING"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashAttestationDocument {
    pub format_version: String,
    pub request_id: String,
    pub issued_at: String,
    pub hash: String,
    /// "FOUND" | "NO_MATCH_FOUND" — derived from count, kept as an
    /// explicit field so renderers don't re-derive status from count == 0.
    pub resolution_status: String,
    pub count: usize,
    pub matches: Vec<HashMatchEntry>,
}

struct RawMatch {
    chain_id: Uuid,
    event_id: Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn verification_status_label(status: &SacVerificationStatus) -> &'static str {
    match status {
        SacVerificationStatus::Verified => "VERIFIED",
        SacVerificationStatus::Failed => "FAILED",
        SacVerificationStatus::NotFound => "NOT FOUND",
    }
}

fn tsa_status_label(tsa: &Option<crate::sac::SacTsaSnapshot>) -> &'static str {
    match tsa {
        Some(t) if matches!(t.status, SacTsaStatus::Present) => "PRESENT",
        _ => "MISSING",
    }
}

/// Builds the full multi-chain attestation for a hash. Per the frozen
/// contract: ALL matches are included, no filtering, no ranking, and the
/// document is generated even when count == 0.
pub async fn build_hash_attestation(
    pool: &PgPool,
    signer: &Arc<ServerSigner>,
    hash: &str,
) -> Result<HashAttestationDocument, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT chain_id, event_id, created_at
        FROM events
        WHERE file_hash = $1
        ORDER BY created_at ASC
        "#,
        hash
    )
    .fetch_all(pool)
    .await?;

    let raw_matches: Vec<RawMatch> = rows
        .into_iter()
        .map(|r| RawMatch {
            chain_id: r.chain_id,
            event_id: r.event_id,
            created_at: r.created_at,
        })
        .collect();

    // Cache per-chain attestation lookups: a hash can occur multiple times
    // within the same chain, and we must not call build_attestation twice
    // for the same chain_id (no duplicate verification work).
    let mut cache: HashMap<Uuid, crate::sac::SacDocument> = HashMap::new();

    let mut matches = Vec::with_capacity(raw_matches.len());
    for raw in &raw_matches {
        if !cache.contains_key(&raw.chain_id) {
            let doc = build_attestation(pool, signer, raw.chain_id).await?;
            cache.insert(raw.chain_id, doc);
        }
        let doc = cache.get(&raw.chain_id).unwrap();

        matches.push(HashMatchEntry {
            chain_id: raw.chain_id.to_string(),
            event_id: raw.event_id.to_string(),
            timestamp: raw.created_at.to_rfc3339(),
            merkle_root: doc.state.as_ref().map(|s| s.merkle_root.clone()),
            head_event_id: doc.state.as_ref().map(|s| s.head_event_id.clone()),
            verification_status: verification_status_label(&doc.verification.status).to_string(),
            tsa_status: tsa_status_label(&doc.tsa).to_string(),
        });
    }

    let count = matches.len();
    let resolution_status = if count > 0 { "FOUND" } else { "NO_MATCH_FOUND" }.to_string();

    Ok(HashAttestationDocument {
        format_version: HASH_ATTESTATION_FORMAT_VERSION.to_string(),
        request_id: Uuid::new_v4().to_string(),
        issued_at: chrono::Utc::now().to_rfc3339(),
        hash: hash.to_string(),
        resolution_status,
        count,
        matches,
    })
}
