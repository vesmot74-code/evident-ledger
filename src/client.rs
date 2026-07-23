use reqwest::blocking::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

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
    #[serde(default)]
    pub version: Option<String>,
    #[serde(rename = "type", default)]
    pub proof_type: Option<String>,
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
    pub leaf_version: String,
    pub chain_id: String,
    pub head_event_id: String,
    pub proof: ProofPayload,
    pub events: Vec<EventLeaf>,
    pub tsa: Option<TsaData>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct VerifyResponse {
    pub chain_id: String,
    pub valid: bool,
    pub blocks: usize,
    pub errors: Vec<String>,
    pub head_event_id: String,
    pub proof: ProofPayload,
}

pub struct EvidentClient {
    base_url: String,
    client: Client,
    api_key: Option<String>,
    desktop_token: Option<String>,
}

#[derive(Debug)]
pub enum ClientError {
    Http(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Server(String),
}
impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        ClientError::Http(e)
    }
}
impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::Io(e)
    }
}
impl From<serde_json::Error> for ClientError {
    fn from(e: serde_json::Error) -> Self {
        ClientError::Json(e)
    }
}

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

#[derive(Debug, Deserialize, Clone)]
pub struct BackupCreateResponse {
    pub backup_id: String,
    pub status: String,
    pub event_count: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BackupListItem {
    pub backup_id: String,
    pub chain_id: String,
    pub created_at: String,
    pub event_count: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DevChangePlanResponse {
    pub success: bool,
    pub old_plan: String,
    pub new_plan: String,
}

impl EvidentClient {
    /// Creates a client and loads credentials automatically:
    /// 1) `EVIDENT_DESKTOP_TOKEN` (Bearer desktop_…),
    /// 2) `EVIDENT_API_KEY` / `~/.evident/api_key` (X-API-KEY).
    pub fn new(base_url: impl Into<String>) -> Self {
        let desktop_token = Self::load_desktop_token();
        let api_key = if desktop_token.is_some() {
            None
        } else {
            Self::load_api_key()
        };
        Self {
            base_url: base_url.into(),
            client: Client::new(),
            api_key,
            desktop_token,
        }
    }

    /// Explicitly set/override API key (CLI / legacy GUI path).
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self.desktop_token = None;
        self
    }

    /// Desktop Bearer token (Stage 13.4). Takes precedence over API key.
    pub fn with_desktop_token(mut self, token: impl Into<String>) -> Self {
        self.desktop_token = Some(token.into());
        self.api_key = None;
        self
    }

    fn load_desktop_token() -> Option<String> {
        if let Ok(key) = std::env::var("EVIDENT_DESKTOP_TOKEN") {
            let key = key.trim().to_string();
            if key.starts_with("desktop_") && !key.is_empty() {
                return Some(key);
            }
        }
        None
    }

    fn load_api_key() -> Option<String> {
        if let Ok(key) = std::env::var("EVIDENT_API_KEY") {
            let key = key.trim().to_string();
            if !key.is_empty() {
                return Some(key);
            }
        }
        let key_path = Self::evident_dir().join("api_key");
        fs::read_to_string(key_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Adds auth headers: `Authorization: Bearer` for desktop tokens, else `X-API-KEY`.
    fn authed(&self, builder: RequestBuilder) -> RequestBuilder {
        if let Some(token) = &self.desktop_token {
            return builder.header("Authorization", format!("Bearer {token}"));
        }
        match &self.api_key {
            Some(key) => builder.header("X-API-KEY", key),
            None => builder,
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.desktop_token.is_some() || self.api_key.is_some()
    }

    pub fn uses_desktop_token(&self) -> bool {
        self.desktop_token.is_some()
    }

    fn evident_dir() -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".evident")
    }

    /// `GET /v1/me` — account profile for the current credential.
    pub fn fetch_me(&self) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .authed(self.client.get(format!("{}/v1/me", self.base_url)))
            .send()?;
        if !resp.status().is_success() {
            return Err(ClientError::Server(format!(
                "GET /v1/me failed: {}",
                resp.status()
            )));
        }
        Ok(resp.json()?)
    }

    /// Best-effort `GET /public/verify?file_hash=` → `public_proof_id` (`pv_…`).
    pub fn lookup_public_proof_id(&self, file_hash: &str) -> Option<String> {
        let resp = self
            .client
            .get(format!("{}/public/verify", self.base_url))
            .query(&[("file_hash", file_hash)])
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let v: serde_json::Value = resp.json().ok()?;
        v.get("public_proof_id")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
    pub fn head_event_id(&self, chain_id: &Uuid) -> Result<Option<Uuid>, ClientError> {
        let resp = self
            .client
            .get(format!("{}/verify/{}", self.base_url, chain_id))
            .send()?;
        let json: serde_json::Value = resp.json()?;
        Ok(json["head_event_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok()))
    }

    /// Запрашивает у сервера текущий публичный ключ подписи (Ed25519, hex).
    /// Не сохраняет ничего на диск — сохранение/пиннинг ключа делает вызывающий
    /// код осознанно (TOFU при первом использовании, либо явное обновление
    /// по нажатию пользователя).
    pub fn fetch_identity(&self) -> Result<String, ClientError> {
        let resp = self
            .client
            .get(format!("{}/identity", self.base_url))
            .send()?;
        let json: serde_json::Value = resp.json()?;
        json["public_key"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ClientError::Server("identity response missing public_key".to_string()))
    }

    /// Отправляет событие на сервер, сохраняет ProofFile на диск.
    /// Возвращает (CommitResponse, путь_к_сохранённому_proof_json, sha256_файла).
    pub fn submit_event(
        &self,
        chain_id: Uuid,
        file_bytes: &[u8],
    ) -> Result<(CommitResponse, PathBuf, String), ClientError> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(file_bytes);
        let file_hash = format!("{:x}", hasher.finalize());

        let parent_event_id = self.head_event_id(&chain_id)?;
        let idempotency_key = Uuid::new_v4().to_string();

        let resp = self
            .authed(self.client.post(format!("{}/events", self.base_url)))
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
        let mut proof = commit.proof.clone();
        if proof.version.is_none() {
            proof.version = Some(crate::proof_format::PROOF_VERSION.to_string());
        }
        if proof.proof_type.is_none() {
            proof.proof_type = Some(crate::proof_format::PROOF_TYPE.to_string());
        }
        let proof_file = ProofFile {
            leaf_version: crate::proof_format::LEAF_VERSION.to_string(),
            chain_id: commit.chain_id.clone(),
            head_event_id: commit.head_event_id.clone(),
            proof,
            events: commit.events.clone(),
            tsa: commit.tsa.clone(),
        };
        fs::write(&proof_path, serde_json::to_string_pretty(&proof_file)?)?;

        Ok((commit, proof_path, file_hash))
    }

    pub fn verify_chain(&self, chain_id: Uuid) -> Result<VerifyResponse, ClientError> {
        let resp = self
            .client
            .get(format!("{}/verify/{}", self.base_url, chain_id))
            .send()?;
        if !resp.status().is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(ClientError::Server(body));
        }
        let result: VerifyResponse = resp.json()?;
        Ok(result)
    }

    pub fn fetch_proof(&self, chain_id: Uuid) -> Result<ProofFile, ClientError> {
        let resp = self
            .client
            .get(format!("{}/verify/proof/{}", self.base_url, chain_id))
            .send()?;
        if !resp.status().is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(ClientError::Server(body));
        }
        let proof: ProofFile = resp.json()?;
        Ok(proof)
    }

    /// Запрашивает account capabilities текущего пользователя (тариф, лимиты,
    /// включённые продуктовые слои). Требует X-API-KEY — если ключ не
    /// загружен, сервер вернёт 401, что будет отражено в ClientError::Server.
    pub fn fetch_capabilities(&self) -> Result<serde_json::Value, ClientError> {
        let resp = self
            .authed(
                self.client
                    .get(format!("{}/account/capabilities", self.base_url)),
            )
            .send()?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }
        let json: serde_json::Value = resp.json()?;
        Ok(json)
    }

    pub fn dev_change_plan(
        &self,
        account_id: Uuid,
        plan: &str,
    ) -> Result<DevChangePlanResponse, ClientError> {
        let resp = self
            .authed(
                self.client
                    .post(format!("{}/account/dev/change-plan", self.base_url)),
            )
            .json(&serde_json::json!({
                "account_id": account_id,
                "plan": plan,
            }))
            .send()?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }
        Ok(resp.json()?)
    }

    pub fn backup_create(&self, chain_id: Uuid) -> Result<BackupCreateResponse, ClientError> {
        let resp = self
            .authed(self.client.post(format!("{}/backup/create", self.base_url)))
            .json(&serde_json::json!({ "chain_id": chain_id }))
            .send()?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }
        Ok(resp.json()?)
    }

    pub fn backup_list(&self) -> Result<Vec<BackupListItem>, ClientError> {
        let resp = self
            .authed(self.client.get(format!("{}/backup/list", self.base_url)))
            .send()?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }
        Ok(resp.json()?)
    }

    pub fn backup_download(&self, backup_id: Uuid) -> Result<Vec<u8>, ClientError> {
        let resp = self
            .authed(
                self.client
                    .get(format!("{}/backup/{backup_id}/download", self.base_url)),
            )
            .send()?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(Self::map_http_error(status, &body));
        }
        Ok(resp.bytes()?.to_vec())
    }

    fn map_http_error(status: reqwest::StatusCode, body: &str) -> ClientError {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
            if json["error"].as_str() == Some("feature_not_available")
                && json["feature"].as_str() == Some("server_backup")
            {
                let plan = json["plan"].as_str().unwrap_or("current");
                return ClientError::Server(format!(
                    "Server backup is not available on your plan ({plan}). Upgrade to Vault or Identity to use this feature."
                ));
            }
            if json["error"].as_str() == Some("not_found") {
                return ClientError::Server(
                    "Backup not found (or not owned by this account).".into(),
                );
            }
            if let Some(msg) = json["error"].as_str() {
                return ClientError::Server(format!("server error {status}: {msg}"));
            }
        }
        ClientError::Server(format!("server error {status}: {body}"))
    }
}

pub fn fetch_capabilities(client: &EvidentClient) -> Result<serde_json::Value, ClientError> {
    client.fetch_capabilities()
}

pub fn verify_chain(client: &EvidentClient, chain_id: Uuid) -> Result<VerifyResponse, ClientError> {
    client.verify_chain(chain_id)
}

pub fn fetch_proof(client: &EvidentClient, chain_id: Uuid) -> Result<ProofFile, ClientError> {
    client.fetch_proof(chain_id)
}
