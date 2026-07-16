use std::env::var_os;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::audit::{AuditEvent, AuditStore, ChainAnchorProof};

#[derive(Debug, Clone)]
pub struct FixationResult {
    pub proof_path: PathBuf,
    pub audit_path: PathBuf,
    pub event_id: String,
    pub chain_id: String,
    pub root: String,
}

#[derive(Debug, Deserialize)]
struct CommitResponse {
    event_id: String,
    chain_id: String,
    head_event_id: String,
    proof: ProofPayload,
    events: Vec<EventLeaf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofFile {
    leaf_version: String,
    chain_id: String,
    head_event_id: String,
    proof: ProofPayload,
    events: Vec<EventLeaf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofPayload {
    root: String,
    chain_head: String,
    signature: String,
    public_key: String,
    leaves_count: usize,
    #[serde(default)]
    version: Option<String>,
    #[serde(rename = "type", default)]
    proof_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct EventLeaf {
    sequence: i64,
    event_id: String,
    parent_event_id: String,
    file_hash: String,
}

pub fn fixate_file(
    path: &Path,
    chain_id: &str,
    audit_path: Option<&Path>,
) -> Result<FixationResult> {
    let bytes = fs::read(path).context("failed to read file")?;
    let file_hash = sha256_hex(&bytes);
    let chain_uuid = Uuid::parse_str(chain_id).context("invalid chain id")?;
    let event_id = Uuid::new_v4();
    let idempotency_key = Uuid::new_v4().to_string();

    let home = var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let audit_path = match audit_path {
        Some(path) => path.to_path_buf(),
        None => home.join(".evident").join("audit.jsonl"),
    };
    let store = AuditStore::new(&audit_path);

    let created = AuditEvent::created(event_id, chain_uuid, file_hash.clone(), None);
    store.append(&created)?;

    let client = reqwest::blocking::Client::new();
    let response = client
        .post("http://127.0.0.1:3000/events")
        .json(&json!({
            "file_hash": file_hash,
            "chain_id": chain_uuid,
            "idempotency_key": idempotency_key
        }))
        .send()
        .context("failed to reach ledger server")?;

    let status = response.status();
    let body = response.text().context("failed to read ledger response")?;
    if !status.is_success() {
        let failed = AuditEvent::failed(
            event_id,
            chain_uuid,
            file_hash.clone(),
            None,
            format!("server error {status}: {body}"),
        );
        store.append(&failed)?;
        anyhow::bail!("server error {status}: {body}");
    }

    let commit: CommitResponse = serde_json::from_str(&body).context("invalid ledger response")?;
    let submitted = AuditEvent::submitted(
        event_id,
        chain_uuid,
        file_hash.clone(),
        None,
        idempotency_key,
    );
    store.append(&submitted)?;

    let proof_path = PathBuf::from("proofs")
        .join(commit.chain_id.clone())
        .join(format!("{}.json", commit.event_id));
    fs::create_dir_all(proof_path.parent().context("invalid proof path")?)?;
    let mut proof_payload = commit.proof.clone();
    if proof_payload.version.is_none() {
        proof_payload.version = Some(crate::proof_format::PROOF_VERSION.to_string());
    }
    if proof_payload.proof_type.is_none() {
        proof_payload.proof_type = Some(crate::proof_format::PROOF_TYPE.to_string());
    }
    let proof = ProofFile {
        leaf_version: crate::proof_format::LEAF_VERSION.to_string(),
        chain_id: commit.chain_id.clone(),
        head_event_id: commit.head_event_id.clone(),
        proof: proof_payload,
        events: commit.events.clone(),
    };
    fs::write(&proof_path, serde_json::to_string_pretty(&proof)?)?;

    // Get sequence number from commit.events
    let leaf = commit
        .events
        .iter()
        .find(|leaf| leaf.event_id == commit.event_id)
        .ok_or_else(|| anyhow::anyhow!("commit.event_id not found in commit.events"))?;

    let sequence = leaf.sequence;

    // Get parent_event_id from commit.events
    let parent_event_id = commit
        .events
        .iter()
        .find(|leaf| leaf.event_id == commit.event_id)
        .and_then(|leaf| Uuid::parse_str(&leaf.parent_event_id).ok());

    let server_event_id = Uuid::parse_str(&commit.event_id)
        .map_err(|e| anyhow::anyhow!("commit.event_id is not a valid UUID: {e}"))?;

    let anchored = AuditEvent::anchored(
        Uuid::new_v4(),
        chain_uuid,
        file_hash,
        parent_event_id,
        sequence,
        server_event_id,
        Some(ChainAnchorProof::new(
            commit.proof.root.clone(),
            commit.proof.signature.clone(),
            "evident-ledger".into(),
        )),
    );
    store.append(&anchored)?;

    Ok(FixationResult {
        proof_path,
        audit_path: audit_path.to_path_buf(),
        event_id: commit.event_id,
        chain_id: commit.chain_id,
        root: commit.proof.root,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
