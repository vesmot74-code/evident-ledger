mod client_adapter;
mod config;
mod core;
mod facade;
mod http;
mod openssl_provider;
mod provider;
mod response;

use thiserror::Error;

pub use client_adapter::{Rfc3161Client, Rfc3161Provider};
pub use config::{HashAlgorithm, TsaConfig, TsaMode};
pub use core::{
    build_timestamp_query, inspect_tsa_token, normalize_provider, parse_and_validate_tsr,
    request_external_timestamp, validate_tsa_token, validate_tsa_token_for_hash, TsaValidation,
};
pub use facade::provider_from_config;
pub use http::{HttpTsaProvider, JsonTsaProvider};
pub use provider::TsaProvider;
pub use response::{TsaProof, TsaResponse};

#[derive(Debug, Error)]
pub enum TsaError {
    #[error("TSA request failed: {0}")]
    RequestFailed(String),
}
