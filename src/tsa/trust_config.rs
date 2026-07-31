//! TSA trust-material configuration checks (deployment vs runtime availability).

use std::fmt;
use std::path::{Path, PathBuf};

/// Configuration failure for FreeTSA / RFC3161 trust material.
///
/// Distinct from network/TSA outage: these errors mean local deployment
/// trust files are missing or mis-pointed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TsaConfigError {
    MissingCaPath,
    CaNotFound(PathBuf),
    MissingTsaCertPath,
    TsaCertNotFound(PathBuf),
}

impl fmt::Display for TsaConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCaPath => write!(f, "missing CA certificate path"),
            Self::CaNotFound(_) => write!(f, "CA certificate file not found"),
            Self::MissingTsaCertPath => write!(f, "missing TSA certificate path"),
            Self::TsaCertNotFound(_) => write!(f, "TSA certificate file not found"),
        }
    }
}

impl std::error::Error for TsaConfigError {}

/// Validate that CA and TSA certificate paths are configured and point at files.
///
/// Does not read environment variables — callers pass resolved paths.
pub fn check_tsa_configuration(
    ca_path: Option<&Path>,
    untrusted_path: Option<&Path>,
) -> Result<(), TsaConfigError> {
    let ca = ca_path.ok_or(TsaConfigError::MissingCaPath)?;
    if !ca.is_file() {
        return Err(TsaConfigError::CaNotFound(ca.to_path_buf()));
    }

    let untrusted = untrusted_path.ok_or(TsaConfigError::MissingTsaCertPath)?;
    if !untrusted.is_file() {
        return Err(TsaConfigError::TsaCertNotFound(untrusted.to_path_buf()));
    }

    Ok(())
}

/// Read FreeTSA trust path env vars without validating files.
///
/// Empty values are treated as unset.
pub fn freetsa_trust_path_options_from_env() -> (Option<PathBuf>, Option<PathBuf>) {
    let ca = std::env::var("FREETSA_CA_CERT_PATH")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let untrusted = std::env::var("FREETSA_UNTRUSTED_CERT_PATH")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    (ca, untrusted)
}

/// Map a config error to the API/audit `verification_reason` vocabulary.
pub fn verification_reason_for_config_error(err: &TsaConfigError) -> &'static str {
    match err {
        TsaConfigError::MissingCaPath | TsaConfigError::MissingTsaCertPath => {
            "trust_material_missing"
        }
        TsaConfigError::CaNotFound(_) | TsaConfigError::TsaCertNotFound(_) => {
            "trust_material_invalid"
        }
    }
}

/// Whether the process should treat the deployment as production for TSA trust policy.
///
/// Uses runtime env labels (`ENVIRONMENT` / `APP_ENV`), not compile-time features.
pub fn is_production_tsa_env(environment: &str, app_env: Option<&str>) -> bool {
    environment.eq_ignore_ascii_case("production")
        || app_env.is_some_and(|v| v.eq_ignore_ascii_case("production"))
}

/// Apply startup policy for TSA trust configuration.
///
/// Production: controlled process exit on invalid config.
/// Non-production: warn and continue.
pub fn enforce_tsa_trust_at_startup(is_production: bool, err: &TsaConfigError) {
    if is_production {
        eprintln!("TSA trust configuration invalid: {}", err);
        std::process::exit(1);
    }
    tracing::warn!("TSA trust configuration incomplete: {}", err);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "evident-tsa-trust-{}-{}-{}",
            label,
            std::process::id(),
            nanos
        ));
        fs::write(&path, b"placeholder-cert\n").expect("write temp cert");
        path
    }

    #[test]
    fn missing_ca_path_errors() {
        let tsa = temp_file("tsa");
        let err = check_tsa_configuration(None, Some(tsa.as_path())).unwrap_err();
        assert_eq!(err, TsaConfigError::MissingCaPath);
        assert_eq!(
            verification_reason_for_config_error(&err),
            "trust_material_missing"
        );
        let _ = fs::remove_file(tsa);
    }

    #[test]
    fn missing_tsa_cert_path_errors() {
        let ca = temp_file("ca");
        let err = check_tsa_configuration(Some(ca.as_path()), None).unwrap_err();
        assert_eq!(err, TsaConfigError::MissingTsaCertPath);
        assert_eq!(
            verification_reason_for_config_error(&err),
            "trust_material_missing"
        );
        let _ = fs::remove_file(ca);
    }

    #[test]
    fn valid_configuration_ok() {
        let ca = temp_file("ca-ok");
        let tsa = temp_file("tsa-ok");
        check_tsa_configuration(Some(ca.as_path()), Some(tsa.as_path())).expect("valid");
        let _ = fs::remove_file(ca);
        let _ = fs::remove_file(tsa);
    }

    #[test]
    fn ca_path_not_a_file_errors() {
        let dir = std::env::temp_dir();
        let tsa = temp_file("tsa-dir");
        let err = check_tsa_configuration(Some(dir.as_path()), Some(tsa.as_path())).unwrap_err();
        assert!(matches!(err, TsaConfigError::CaNotFound(_)));
        assert_eq!(
            verification_reason_for_config_error(&err),
            "trust_material_invalid"
        );
        let _ = fs::remove_file(tsa);
    }

    #[test]
    fn production_env_detection() {
        assert!(is_production_tsa_env("production", None));
        assert!(is_production_tsa_env("development", Some("production")));
        assert!(!is_production_tsa_env("development", Some("staging")));
        assert!(!is_production_tsa_env("staging", None));
    }
}
