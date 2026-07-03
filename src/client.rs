use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TsaData {
    pub timestamp: Option<i64>,
    pub serial: Option<String>,
    pub token_bytes: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProofPayload {
    pub root: String,
    pub chain_head: String,
    pub signature: String,
    pub public_key: String,
    pub leaves_count: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EventLeaf {
    pub sequence: i64,
    pub event_id: String,
    pub parent_event_id: String,
    pub file_hash: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CommitResponse {
    pub event_id: String,
    pub chain_id: String,
    pub head_event_id: String,
    pub sequence: i64,
    pub proof: ProofPayload,
    pub events: Vec<EventLeaf>,
    pub tsa: Option<TsaData>,
    #[serde(default)]
    pub cached: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProofFile {
    pub chain_id: String,
    pub head_event_id: String,
    pub proof: ProofPayload,
    pub events: Vec<EventLeaf>,
    pub tsa: Option<TsaData>,
}

pub struct EvidentClient {
    base_url: String,
    client: Client,
}

#[derive(Debug)]
pub enum ClientError {
    Http(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Server(String),
}
impl From<reqwest::Error> for ClientError { fn from(e: reqwest::Error) -> Self { ClientError::Http(e) } }
impl From<std::io::Error> for ClientError { fn from(e: std::io::Error) -> Self { ClientError::Io(e) } }
impl From<serde_json::Error> for ClientError { fn from(e: serde_json::Error) -> Self { ClientError::Json(e) } }

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Http(e) => write!(f, "HTTP error: {e}"),
            ClientError::Io(e) => write!(f, "IO error: {e}"),
            ClientError::Json(e) => write!(f, "JSON error: {e}"),
            ClientError::Server(s) => write!(f, "Server error: {s}"),
        }
    }
}

impl EvidentClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), client: Client::new() }
    }

    fn evident_dir() -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".evident")
    }

    pub fn head_event_id(&self, chain_id: &Uuid) -> Result<Option<Uuid>, ClientError> {
        let resp = self.client
            .get(format!("{}/verify/{}", self.base_url, chain_id))
            .send()?;
        let json: serde_json::Value = resp.json()?;
        Ok(json["head_event_id"].as_str().and_then(|s| Uuid::parse_str(s).ok()))
    }

    /// Отправляет событие на сервер, сохраняет ProofFile на диск.
    /// Возвращает (CommitResponse, путь_к_сохранённому_proof_json, sha256_файла).
    pub fn submit_event(&self, chain_id: Uuid, file_bytes: &[u8]) -> Result<(CommitResponse, PathBuf, String), ClientError> {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(file_bytes);
        let file_hash = format!("{:x}", hasher.finalize());

        let parent_event_id = self.head_event_id(&chain_id)?;
        let idempotency_key = Uuid::new_v4().to_string();

        let resp = self.client
            .post(format!("{}/events", self.base_url))
            .json(&serde_json::json!({
                "chain_id": chain_id,
                "parent_event_id": parent_event_id,
                "file_hash": file_hash,
                "idempotency_key": idempotency_key,
            }))
            .send()?;

        if !resp.status().is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(ClientError::Server(body));
        }
        let commit: CommitResponse = resp.json()?;

        let proof_dir = Self::evident_dir().join("proofs").join(&commit.chain_id);
        fs::create_dir_all(&proof_dir)?;
        let proof_path = proof_dir.join(format!("{}.json", commit.event_id));
        let proof_file = ProofFile {
            chain_id: commit.chain_id.clone(),
            head_event_id: commit.head_event_id.clone(),
            proof: commit.proof.clone(),
            events: commit.events.clone(),
            tsa: commit.tsa.clone(),
        };
        fs::write(&proof_path, serde_json::to_string_pretty(&proof_file)?)?;

        Ok((commit, proof_path, file_hash))
    }
}
