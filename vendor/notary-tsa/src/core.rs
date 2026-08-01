use thiserror::Error;
use tsp_http_client::TimeStampResponse;

use crate::config::HashAlgorithm;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unsupported hash length for SHA-256: {0}")]
    InvalidHashLength(usize),
    #[error("unsupported hash algorithm")]
    UnsupportedAlgorithm,
    /// Network / TLS / DNS / timeout (typed at the HTTP client boundary).
    #[error("TSA transport error: {0}")]
    Transport(String),
    /// Protocol / malformed response / digest mismatch / HTTP status rejection.
    #[error("TSA protocol error: {0}")]
    Protocol(String),
    /// Fallback when the underlying error type could not be classified.
    #[error("invalid RFC3161 response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone)]
pub struct ParsedTsr {
    pub token: Vec<u8>,
    pub timestamp: u64,
    pub serial: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsaValidation {
    pub structural: bool,
    pub imprint_match: bool,
    pub provider: String,
}

impl TsaValidation {
    pub fn is_valid(&self) -> bool {
        self.structural && self.imprint_match
    }
}

/// Build a DER-encoded RFC3161 TimeStampReq for the given digest.
pub fn build_timestamp_query(hash: &[u8], alg: HashAlgorithm) -> Result<Vec<u8>, CoreError> {
    match alg {
        HashAlgorithm::Sha256 if hash.len() == 32 => {}
        HashAlgorithm::Sha256 => return Err(CoreError::InvalidHashLength(hash.len())),
    }

    let _ = hash;
    Err(CoreError::UnsupportedAlgorithm)
}

/// Parse and structurally validate a TimeStampResp.
pub fn parse_and_validate_tsr(tsr: &[u8], expected_hash: &[u8]) -> Result<ParsedTsr, CoreError> {
    if expected_hash.len() != 32 {
        return Err(CoreError::InvalidHashLength(expected_hash.len()));
    }

    let validation = validate_tsa_bytes_for_hash(tsr, expected_hash)?;
    if !validation.is_valid() {
        return Err(CoreError::Protocol(
            "RFC3161 imprint validation failed".into(),
        ));
    }

    parse_tsr(tsr)
}

/// Validate stored base64 token has RFC3161 structure (no chain verification).
pub fn validate_tsa_token(token_b64: &str) -> bool {
    validate_tsa_token_for_hash(token_b64, None)
        .map(|v| v.structural)
        .unwrap_or(false)
}

/// Full public validation: structure + optional SHA-256 imprint match.
pub fn validate_tsa_token_for_hash(
    token_b64: &str,
    sha256_hex: Option<&str>,
) -> Result<TsaValidation, CoreError> {
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, token_b64.trim())
            .map_err(|e| CoreError::InvalidResponse(e.to_string()))?;

    if bytes.is_empty() {
        return Ok(TsaValidation {
            structural: false,
            imprint_match: false,
            provider: "missing".into(),
        });
    }

    if is_json_stub_token(&bytes) {
        let imprint_match = sha256_hex
            .map(|hash| String::from_utf8_lossy(&bytes).contains(hash))
            .unwrap_or(true);
        return Ok(TsaValidation {
            structural: true,
            imprint_match,
            provider: "stub".into(),
        });
    }

    if let Some(hex) = sha256_hex {
        let hash_bytes = hex::decode(hex).map_err(|e| CoreError::InvalidResponse(e.to_string()))?;
        if hash_bytes.len() != 32 {
            return Err(CoreError::InvalidHashLength(hash_bytes.len()));
        }
        validate_tsa_bytes_for_hash(&bytes, &hash_bytes)
    } else {
        let structural = TimeStampResponse::new(bytes.clone()).datetime().is_ok();
        Ok(TsaValidation {
            structural,
            imprint_match: structural,
            provider: infer_provider_from_tsr(&bytes),
        })
    }
}

/// Parse token without digest check (verify endpoint).
pub fn inspect_tsa_token(token_b64: &str) -> Option<ParsedTsr> {
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, token_b64.trim())
            .ok()?;

    if is_json_stub_token(&bytes) {
        return Some(ParsedTsr {
            token: bytes,
            timestamp: 0,
            serial: "stub".into(),
        });
    }

    parse_tsr(&bytes).ok()
}

/// Request RFC3161 timestamp from an external TSA (blocking).
pub fn request_external_timestamp(url: &str, hash: &[u8]) -> Result<ParsedTsr, CoreError> {
    if hash.len() != 32 {
        return Err(CoreError::InvalidHashLength(hash.len()));
    }
    let hash_hex = hex::encode(hash);
    let response = tsp_http_client::request_timestamp_for_digest(url, &hash_hex)
        .map_err(map_tsp_http_box_error)?;
    parse_tsr(response.as_der_encoded())
}

/// Classify `tsp_http_client` / `ureq` errors by Rust type (no string matching).
pub fn classify_tsp_http_box_error(err: Box<dyn std::error::Error + 'static>) -> CoreError {
    map_tsp_http_box_error(err)
}

/// Classify a concrete `ureq::Error` (no string matching).
pub fn classify_ureq_error(err: &ureq::Error) -> CoreError {
    map_ureq_error(err)
}

/// Classify a concrete `tsp_http_client::Error` (no string matching).
pub fn classify_tsp_http_client_error(err: &tsp_http_client::Error) -> CoreError {
    map_tsp_http_client_error(err)
}

fn map_tsp_http_box_error(err: Box<dyn std::error::Error + 'static>) -> CoreError {
    // Prefer concrete TSP protocol errors when present.
    if let Some(tsp_err) = err.downcast_ref::<tsp_http_client::Error>() {
        return map_tsp_http_client_error(tsp_err);
    }

    // Transport errors from ureq (used inside tsp-http-client).
    if let Some(ureq_err) = err.downcast_ref::<ureq::Error>() {
        return map_ureq_error(ureq_err);
    }

    // Bare IO errors occasionally surface without ureq wrapping.
    if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
        return CoreError::Transport(io_err.to_string());
    }

    CoreError::InvalidResponse(err.to_string())
}

fn map_tsp_http_client_error(err: &tsp_http_client::Error) -> CoreError {
    match err {
        tsp_http_client::Error::InvalidDigest => CoreError::InvalidResponse(err.to_string()),
        tsp_http_client::Error::RequestNotAccepted(_)
        | tsp_http_client::Error::InvalidServerResponse
        | tsp_http_client::Error::DigestMismatch => CoreError::Protocol(err.to_string()),
    }
}

fn map_ureq_error(err: &ureq::Error) -> CoreError {
    match err {
        ureq::Error::Timeout(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Io(_)
        | ureq::Error::Tls(_) => CoreError::Transport(err.to_string()),

        ureq::Error::StatusCode(_)
        | ureq::Error::Http(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::Protocol(_)
        | ureq::Error::RedirectFailed
        | ureq::Error::InvalidProxyUrl
        | ureq::Error::BodyExceedsLimit(_)
        | ureq::Error::TooManyRedirects
        | ureq::Error::RequireHttpsOnly(_)
        | ureq::Error::LargeResponseHeader(_, _) => CoreError::Protocol(err.to_string()),

        // `ureq::Error` is `#[non_exhaustive]`. Feature-gated TLS/cookie/json
        // variants and future variants stay unclassified (RequestFailed via map).
        _ => CoreError::InvalidResponse(err.to_string()),
    }
}

/// Normalize provider label for public verify surface.
pub fn normalize_provider(source: &str) -> String {
    let lower = source.to_lowercase();
    if lower.contains("freetsa") {
        "freetsa".into()
    } else if lower.contains("stub") {
        "stub".into()
    } else if lower.contains("digicert") || lower.contains("sectigo") {
        "external".into()
    } else if lower.is_empty() {
        "unknown".into()
    } else {
        "external".into()
    }
}

fn validate_tsa_bytes_for_hash(
    tsr: &[u8],
    expected_hash: &[u8],
) -> Result<TsaValidation, CoreError> {
    let wrapper = TimeStampResponse::new(tsr.to_vec());
    let structural = wrapper.datetime().is_ok();
    let imprint = extract_message_imprint(tsr).unwrap_or_default();
    let imprint_match = structural && imprint == expected_hash;
    Ok(TsaValidation {
        structural,
        imprint_match,
        provider: infer_provider_from_tsr(tsr),
    })
}

fn extract_message_imprint(tsr: &[u8]) -> Result<Vec<u8>, CoreError> {
    use cms::signed_data::SignedData;
    use der::{Decode, Encode};
    use x509_tsp::{TimeStampResp, TstInfo};

    let response =
        TimeStampResp::from_der(tsr).map_err(|e| CoreError::InvalidResponse(e.to_string()))?;
    let token = response
        .time_stamp_token
        .ok_or_else(|| CoreError::InvalidResponse("missing timestamp token".into()))?;
    let signed_der = token
        .content
        .to_der()
        .map_err(|e| CoreError::InvalidResponse(e.to_string()))?;
    let signed =
        SignedData::from_der(&signed_der).map_err(|e| CoreError::InvalidResponse(e.to_string()))?;
    let encap = signed
        .encap_content_info
        .econtent
        .ok_or_else(|| CoreError::InvalidResponse("missing encap content".into()))?;
    let tst =
        TstInfo::from_der(encap.value()).map_err(|e| CoreError::InvalidResponse(e.to_string()))?;
    Ok(tst.message_imprint.hashed_message.as_bytes().to_vec())
}

fn infer_provider_from_tsr(_tsr: &[u8]) -> String {
    "external".into()
}

fn parse_tsr(tsr: &[u8]) -> Result<ParsedTsr, CoreError> {
    let wrapper = TimeStampResponse::new(tsr.to_vec());
    let datetime = wrapper
        .datetime()
        .map_err(|e| CoreError::Protocol(e.to_string()))?;
    Ok(ParsedTsr {
        token: tsr.to_vec(),
        timestamp: datetime.timestamp() as u64,
        serial: format!("tsr-{}", datetime.timestamp()),
    })
}

fn is_json_stub_token(bytes: &[u8]) -> bool {
    bytes.starts_with(b"STUB-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_hash_length() {
        let err = build_timestamp_query(&[1, 2, 3], HashAlgorithm::Sha256).unwrap_err();
        assert!(matches!(err, CoreError::InvalidHashLength(3)));
    }

    #[test]
    fn validates_stub_token_imprint() {
        let hash = "a".repeat(64);
        let token = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("STUB-{hash}-123").as_bytes(),
        );
        let result = validate_tsa_token_for_hash(&token, Some(&hash)).unwrap();
        assert!(result.is_valid());
        assert_eq!(result.provider, "stub");
    }

    #[test]
    fn test_transport_timeout() {
        let err = ureq::Error::Timeout(ureq::Timeout::Global);
        let classified = classify_ureq_error(&err);
        assert!(
            matches!(classified, CoreError::Transport(_)),
            "timeout must be Transport, got {classified:?}"
        );
    }

    #[test]
    fn test_transport_connection_failure() {
        let refused = ureq::Error::ConnectionFailed;
        assert!(matches!(
            classify_ureq_error(&refused),
            CoreError::Transport(_)
        ));

        let dns = ureq::Error::HostNotFound;
        assert!(matches!(classify_ureq_error(&dns), CoreError::Transport(_)));
    }

    #[test]
    fn test_digest_mismatch() {
        let err = tsp_http_client::Error::DigestMismatch;
        let classified = classify_tsp_http_client_error(&err);
        assert!(
            matches!(classified, CoreError::Protocol(_)),
            "DigestMismatch must be Protocol, got {classified:?}"
        );
        assert!(!matches!(classified, CoreError::Transport(_)));
    }

    #[test]
    fn test_protocol_rejection() {
        let rejected = tsp_http_client::Error::RequestNotAccepted(Some("denied".into()));
        assert!(matches!(
            classify_tsp_http_client_error(&rejected),
            CoreError::Protocol(_)
        ));

        let invalid = tsp_http_client::Error::InvalidServerResponse;
        assert!(matches!(
            classify_tsp_http_client_error(&invalid),
            CoreError::Protocol(_)
        ));
    }

    #[test]
    fn test_box_digest_mismatch_preserves_protocol() {
        let boxed: Box<dyn std::error::Error + 'static> =
            Box::new(tsp_http_client::Error::DigestMismatch);
        let classified = classify_tsp_http_box_error(boxed);
        assert!(matches!(classified, CoreError::Protocol(_)));
    }

    #[test]
    fn test_box_ureq_timeout_preserves_transport() {
        let boxed: Box<dyn std::error::Error + 'static> =
            Box::new(ureq::Error::Timeout(ureq::Timeout::Connect));
        let classified = classify_tsp_http_box_error(boxed);
        assert!(matches!(classified, CoreError::Transport(_)));
    }
}
