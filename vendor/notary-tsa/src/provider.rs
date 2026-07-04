use async_trait::async_trait;

use crate::{TsaError, TsaResponse};

/// Pluggable TSA backend used by the notarization pipeline.
#[async_trait]
pub trait TsaProvider: Send + Sync {
    async fn timestamp(&self, hash: &[u8]) -> Result<TsaResponse, TsaError>;
}
