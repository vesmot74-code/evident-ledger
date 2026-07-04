use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::provider::TsaProvider;
use crate::{TsaError, TsaResponse};

/// JSON HTTP adapter for dev stub (`POST {"hash":"..."}`).
#[derive(Debug, Clone)]
pub struct JsonTsaProvider {
    endpoint: String,
    timeout_seconds: u64,
    max_attempts: u32,
}

impl JsonTsaProvider {
    pub fn new(endpoint: impl Into<String>, timeout_seconds: u64, max_attempts: u32) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout_seconds: timeout_seconds.clamp(1, 120),
            max_attempts: max_attempts.max(1),
        }
    }

    async fn request_once(&self, client: &Client, hash_hex: &str) -> Result<TsaResponse, TsaError> {
        let response = client
            .post(&self.endpoint)
            .json(&TimestampRequest { hash: hash_hex })
            .send()
            .await
            .map_err(|e| TsaError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(TsaError::RequestFailed(format!("HTTP {status}: {body}")));
        }

        let body: TimestampResponseBody = response
            .json()
            .await
            .map_err(|e| TsaError::RequestFailed(e.to_string()))?;

        let token = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &body.token)
            .unwrap_or_else(|_| body.token.into_bytes());

        Ok(TsaResponse {
            token,
            timestamp: body.timestamp,
            serial: body.serial,
            verified: body.verified.unwrap_or(true),
            source: body.source.unwrap_or_else(|| self.endpoint.clone()),
        })
    }
}

#[derive(Debug, Serialize)]
struct TimestampRequest<'a> {
    hash: &'a str,
}

#[derive(Debug, Deserialize)]
struct TimestampResponseBody {
    token: String,
    timestamp: u64,
    serial: String,
    verified: Option<bool>,
    source: Option<String>,
}

#[async_trait]
impl TsaProvider for JsonTsaProvider {
    async fn timestamp(&self, hash: &[u8]) -> Result<TsaResponse, TsaError> {
        if hash.len() != 32 {
            return Err(TsaError::RequestFailed(format!(
                "expected 32-byte SHA-256 digest, got {} bytes",
                hash.len()
            )));
        }

        let hash_hex = hex::encode(hash);
        let client = Client::builder()
            .timeout(Duration::from_secs(self.timeout_seconds))
            .build()
            .map_err(|e| TsaError::RequestFailed(e.to_string()))?;

        let mut last_err = None;
        for attempt in 1..=self.max_attempts {
            match self.request_once(&client, &hash_hex).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    if attempt < self.max_attempts {
                        warn!(attempt, error = %err, "json tsa request failed, retrying");
                    }
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.expect("at least one attempt"))
    }
}

/// Backward-compatible alias.
pub type HttpTsaProvider = JsonTsaProvider;
