use std::env;
use std::fs;
use std::path::Path;

use serde::Deserialize;

/// TSA transport mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsaMode {
    /// RFC3161 `application/timestamp-query` to an external TSA.
    External,
    /// JSON `POST /v1/timestamp` (dev stub / legacy adapter).
    Json,
}

impl TsaMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "external" | "rfc3161" => Some(Self::External),
            "json" | "internal" | "stub" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Supported message digest for RFC3161 requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
}

impl HashAlgorithm {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "sha256" | "sha-256" => Some(Self::Sha256),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
        }
    }
}

/// Production TSA settings for notary-core.
#[derive(Debug, Clone)]
pub struct TsaConfig {
    pub enabled: bool,
    pub mode: TsaMode,
    pub provider_url: String,
    pub timeout_secs: u64,
    pub retries: u32,
    pub hash_alg: HashAlgorithm,
}

impl Default for TsaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: TsaMode::Json,
            provider_url: "http://localhost:3001/v1/timestamp".into(),
            timeout_secs: 10,
            retries: 3,
            hash_alg: HashAlgorithm::Sha256,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct TomlTsaSection {
    enabled: Option<bool>,
    mode: Option<String>,
    provider_url: Option<String>,
    timeout_secs: Option<u64>,
    retries: Option<u32>,
    hash_alg: Option<String>,
}

impl TsaConfig {
    /// Load from optional `config.toml` `[tsa]` section with env overrides.
    pub fn load() -> Self {
        let mut cfg = Self::default();
        if let Some(section) = load_toml_section() {
            if let Some(enabled) = section.enabled {
                cfg.enabled = enabled;
            }
            if let Some(mode) = section.mode.as_deref().and_then(TsaMode::parse) {
                cfg.mode = mode;
            }
            if let Some(url) = section.provider_url.filter(|u| !u.trim().is_empty()) {
                cfg.provider_url = url;
            }
            if let Some(timeout) = section.timeout_secs.filter(|&v| v > 0) {
                cfg.timeout_secs = timeout;
            }
            if let Some(retries) = section.retries.filter(|&v| v > 0) {
                cfg.retries = retries;
            }
            if let Some(alg) = section.hash_alg.as_deref().and_then(HashAlgorithm::parse) {
                cfg.hash_alg = alg;
            }
        }
        cfg.apply_env_overrides();
        cfg
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = env::var("TSA_ENABLED") {
            self.enabled = matches!(v.to_lowercase().as_str(), "1" | "true" | "yes");
        }
        if let Some(mode) = env::var("TSA_MODE").ok().and_then(|m| TsaMode::parse(&m)) {
            self.mode = mode;
        }
        if let Ok(url) = env::var("TSA_PROVIDER_URL") {
            if !url.trim().is_empty() {
                self.provider_url = url;
            }
        } else if let Ok(url) = env::var("TSA_ENDPOINT") {
            if !url.trim().is_empty() {
                self.provider_url = url;
            }
        }
        if let Some(timeout) = env::var("TSA_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .or_else(|| {
                env::var("TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|v| v.trim().trim_end_matches('s').parse().ok())
            })
        {
            if timeout > 0 {
                self.timeout_secs = timeout;
            }
        }
        if let Some(retries) = env::var("TSA_RETRIES").ok().and_then(|v| v.parse().ok()) {
            if retries > 0 {
                self.retries = retries;
            }
        }
        if let Some(alg) = env::var("TSA_HASH_ALG")
            .ok()
            .and_then(|v| HashAlgorithm::parse(&v))
        {
            self.hash_alg = alg;
        }
    }
}

fn load_toml_section() -> Option<TomlTsaSection> {
    let path = env::var("NOTARY_CONFIG").unwrap_or_else(|_| "config.toml".into());
    if !Path::new(&path).exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    table.get("tsa")?.clone().try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes_and_algorithms() {
        assert_eq!(TsaMode::parse("external"), Some(TsaMode::External));
        assert_eq!(TsaMode::parse("json"), Some(TsaMode::Json));
        assert_eq!(HashAlgorithm::parse("sha256"), Some(HashAlgorithm::Sha256));
    }

    #[test]
    fn env_overrides_toml_mode_and_url() {
        let key_mode = "TSA_MODE";
        let key_url = "TSA_PROVIDER_URL";
        let prev_mode = env::var(key_mode).ok();
        let prev_url = env::var(key_url).ok();

        env::set_var(key_mode, "external");
        env::set_var(key_url, "https://freetsa.org/tsr");

        let cfg = TsaConfig::load();
        assert_eq!(cfg.mode, TsaMode::External);
        assert_eq!(cfg.provider_url, "https://freetsa.org/tsr");

        match prev_mode {
            Some(v) => env::set_var(key_mode, v),
            None => env::remove_var(key_mode),
        }
        match prev_url {
            Some(v) => env::set_var(key_url, v),
            None => env::remove_var(key_url),
        }
    }
}
