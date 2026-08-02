mod client_adapter;
mod config;
mod core;
mod facade;
mod http;
mod openssl_provider;
mod provider;
mod response;

use thiserror::Error;

pub use client_adapter::{
    classify_core_error, classify_join_error, Rfc3161Client, Rfc3161Provider,
};
pub use config::{HashAlgorithm, TsaConfig, TsaMode};
pub use core::{
    build_timestamp_query, classify_tsp_http_box_error, classify_tsp_http_client_error,
    classify_ureq_error, inspect_tsa_token, normalize_provider, parse_and_validate_tsr,
    request_external_timestamp, validate_tsa_token, validate_tsa_token_for_hash, CoreError,
    TsaValidation,
};
pub use facade::provider_from_config;
pub use http::{HttpTsaProvider, JsonTsaProvider};
pub use openssl_provider::{
    freetsa_trust_paths, verify_tsr_bytes, OpenSslTsaProvider, OpensslAdapterError,
};
pub use provider::TsaProvider;
pub use response::{TsaProof, TsaResponse};

/// Write-path TSA client error with preserved failure category.
///
/// Category is carried by the enum variant (not by parsing display text).
/// The `String` payload is for logging/Display only.
#[derive(Debug, Error)]
pub enum TsaError {
    /// TCP / DNS / TLS / timeout — transport failed before a usable TSA reply.
    #[error("TSA transport error: {0}")]
    Transport(String),

    /// HTTP/TSA protocol rejection, malformed response, digest mismatch, etc.
    #[error("TSA protocol error: {0}")]
    Protocol(String),

    /// Unclassified / local input / join failures without a preserved type.
    #[error("TSA request failed: {0}")]
    RequestFailed(String),

    /// TSA subsystem disabled in configuration.
    #[error("TSA subsystem is disabled")]
    Disabled,
}
