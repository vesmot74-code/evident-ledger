use serde::Serialize;

/// RFC3161 timestamp result returned by all TSA providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsaResponse {
    /// DER-encoded TimeStampToken / TSR bytes.
    pub token: Vec<u8>,
    pub timestamp: u64,
    pub serial: String,
    /// Structural RFC3161 validation succeeded.
    pub verified: bool,
    /// Provider identifier (URL or adapter name).
    pub source: String,
}

/// API-facing TSA proof attached to notarize responses.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TsaProof {
    pub token: String,
    pub timestamp: u64,
    pub serial: String,
    pub verified: bool,
    pub source: String,
}

impl From<TsaResponse> for TsaProof {
    fn from(value: TsaResponse) -> Self {
        Self {
            token: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &value.token),
            timestamp: value.timestamp,
            serial: value.serial,
            verified: value.verified,
            source: value.source,
        }
    }
}

impl From<&TsaResponse> for TsaProof {
    fn from(value: &TsaResponse) -> Self {
        value.clone().into()
    }
}
