use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

use crate::config::{HashAlgorithm, TsaConfig, TsaMode};
use crate::core::{parse_and_validate_tsr, request_external_timestamp, CoreError};
use crate::http::JsonTsaProvider;
use crate::provider::TsaProvider;
use crate::{TsaError, TsaResponse};

/// HTTP transport for RFC3161 timestamp queries (adapted from guardway `tsa_bridge`).
#[derive(Debug, Clone)]
pub struct Rfc3161Client {
    url: String,
    timeout_secs: u64,
    retries: u32,
    hash_alg: HashAlgorithm,
    source: String,
}

impl Rfc3161Client {
    pub fn new(
        url: impl Into<String>,
        timeout_secs: u64,
        retries: u32,
        hash_alg: HashAlgorithm,
    ) -> Self {
        let url = url.into();
        let source = url.clone();
        Self {
            url,
            timeout_secs: timeout_secs.clamp(1, 120),
            retries: retries.max(1),
            hash_alg,
            source,
        }
    }

    pub async fn timestamp(&self, hash: &[u8]) -> Result<TsaResponse, TsaError> {
        if hash.len() != 32 {
            return Err(TsaError::RequestFailed(format!(
                "expected 32-byte SHA-256 digest, got {} bytes",
                hash.len()
            )));
        }

        info!(
            hash = %hex::encode(hash),
            url = %self.url,
            alg = %self.hash_alg.as_str(),
            timeout_secs = self.timeout_secs,
            "tsa rfc3161 request"
        );

        let url = self.url.clone();
        let retries = self.retries;
        let source = self.source.clone();
        let hash_vec = hash.to_vec();

        let mut last_err = None;
        for attempt in 1..=retries {
            let url = url.clone();
            let hash_vec = hash_vec.clone();
            let parsed_result =
                tokio::task::spawn_blocking(move || request_external_timestamp(&url, &hash_vec))
                    .await;

            match parsed_result {
                Ok(Ok(parsed)) => {
                    if let Err(err) = parse_and_validate_tsr(&parsed.token, hash) {
                        last_err = Some(map_core_error(err));
                        if attempt < retries {
                            warn!(attempt, "tsa response validation failed, retrying");
                        }
                        continue;
                    }

                    info!(
                        hash = %hex::encode(hash),
                        timestamp = parsed.timestamp,
                        serial = %parsed.serial,
                        "tsa rfc3161 response"
                    );

                    return Ok(TsaResponse {
                        token: parsed.token,
                        timestamp: parsed.timestamp,
                        serial: parsed.serial,
                        verified: true,
                        source,
                    });
                }
                Ok(Err(err)) => {
                    if attempt < retries {
                        warn!(attempt, error = %err, "tsa request failed, retrying");
                    }
                    last_err = Some(map_core_error(err));
                }
                Err(err) => {
                    last_err = Some(TsaError::RequestFailed(err.to_string()));
                }
            }
        }

        Err(last_err.expect("at least one attempt"))
    }
}

#[derive(Debug, Clone)]
pub struct Rfc3161Provider {
    client: Rfc3161Client,
}

impl Rfc3161Provider {
    pub fn from_config(config: &TsaConfig) -> Self {
        Self {
            client: Rfc3161Client::new(
                config.provider_url.clone(),
                config.timeout_secs,
                config.retries,
                config.hash_alg,
            ),
        }
    }
}

#[async_trait]
impl TsaProvider for Rfc3161Provider {
    async fn timestamp(&self, hash: &[u8]) -> Result<TsaResponse, TsaError> {
        self.client.timestamp(hash).await
    }
}

/// Build the configured production TSA provider.
pub fn build_tsa_provider(config: &TsaConfig) -> Arc<dyn TsaProvider> {
    if !config.enabled {
        return Arc::new(DisabledTsaProvider);
    }

    match config.mode {
        TsaMode::External => Arc::new(Rfc3161Provider::from_config(config)),
        TsaMode::Json => Arc::new(JsonTsaProvider::new(
            config.provider_url.clone(),
            config.timeout_secs,
            config.retries,
        )),
    }
}

#[derive(Debug, Clone)]
struct DisabledTsaProvider;

#[async_trait]
impl TsaProvider for DisabledTsaProvider {
    async fn timestamp(&self, _hash: &[u8]) -> Result<TsaResponse, TsaError> {
        Err(TsaError::RequestFailed("TSA subsystem is disabled".into()))
    }
}

fn map_core_error(err: CoreError) -> TsaError {
    TsaError::RequestFailed(err.to_string())
}
