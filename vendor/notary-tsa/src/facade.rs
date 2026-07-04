use std::sync::Arc;

use crate::client_adapter::build_tsa_provider;
use crate::config::TsaConfig;
use crate::provider::TsaProvider;

/// Construct the configured `Arc<dyn TsaProvider>` for dependency injection.
pub fn provider_from_config(config: &TsaConfig) -> Arc<dyn TsaProvider> {
    build_tsa_provider(config)
}
