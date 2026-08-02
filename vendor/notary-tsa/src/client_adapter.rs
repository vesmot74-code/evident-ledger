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
                    last_err = Some(classify_join_error(err));
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
        Err(TsaError::Disabled)
    }
}

fn map_core_error(err: CoreError) -> TsaError {
    match err {
        CoreError::Transport(msg) => TsaError::Transport(msg),
        CoreError::Protocol(msg) => TsaError::Protocol(msg),
        CoreError::InvalidHashLength(n) => {
            TsaError::RequestFailed(format!("unsupported hash length for SHA-256: {n}"))
        }
        CoreError::UnsupportedAlgorithm => {
            TsaError::RequestFailed("unsupported hash algorithm".into())
        }
        CoreError::InvalidResponse(msg) => TsaError::RequestFailed(msg),
    }
}

/// Map core classification onto public `TsaError` (no string matching).
pub fn classify_core_error(err: CoreError) -> TsaError {
    map_core_error(err)
}

/// JoinError is local runtime failure, never Transport/Protocol.
pub fn classify_join_error(err: tokio::task::JoinError) -> TsaError {
    TsaError::RequestFailed(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HashAlgorithm, TsaConfig, TsaMode};
    use crate::core::CoreError;
    use crate::TsaError;

    #[test]
    fn test_join_error() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let join_err = rt.block_on(async {
            tokio::task::spawn_blocking(|| panic!("forced join failure"))
                .await
                .expect_err("join must fail")
        });
        let mapped = classify_join_error(join_err);
        assert!(
            matches!(mapped, TsaError::RequestFailed(_)),
            "JoinError must be RequestFailed, got {mapped:?}"
        );
        assert!(!matches!(mapped, TsaError::Transport(_)));
        assert!(!matches!(mapped, TsaError::Protocol(_)));
    }

    #[tokio::test]
    async fn test_disabled_provider() {
        let cfg = TsaConfig {
            enabled: false,
            mode: TsaMode::External,
            provider_url: "https://example.invalid/tsr".into(),
            timeout_secs: 1,
            retries: 1,
            hash_alg: HashAlgorithm::Sha256,
        };
        let provider = build_tsa_provider(&cfg);
        let err = provider
            .timestamp(&[0u8; 32])
            .await
            .expect_err("disabled provider must err");
        assert!(matches!(err, TsaError::Disabled));
    }

    #[test]
    fn test_core_transport_maps_to_tsa_transport() {
        let err = classify_core_error(CoreError::Transport("x".into()));
        assert!(matches!(err, TsaError::Transport(_)));
    }

    #[test]
    fn test_core_protocol_maps_to_tsa_protocol() {
        let err = classify_core_error(CoreError::Protocol("x".into()));
        assert!(matches!(err, TsaError::Protocol(_)));
    }
}
