//! Unit coverage for typed write-path TSA error classification (Issue A gate).
//!
//! These tests exercise `notary-tsa` classifiers via the public crate API so they
//! run under `cargo test --lib tsa` without network access.

#[cfg(test)]
mod tests {
    use notary_tsa::{
        classify_core_error, classify_join_error, classify_tsp_http_box_error,
        classify_tsp_http_client_error, classify_ureq_error, provider_from_config, CoreError,
        HashAlgorithm, TsaConfig, TsaError, TsaMode, TsaProvider,
    };

    #[test]
    fn test_transport_timeout() {
        let err = ureq::Error::Timeout(ureq::Timeout::Global);
        let core = classify_ureq_error(&err);
        let tsa = classify_core_error(core);
        assert!(matches!(tsa, TsaError::Transport(_)));
        assert!(!matches!(tsa, TsaError::RequestFailed(_)));
        assert!(!matches!(tsa, TsaError::Protocol(_)));
    }

    #[test]
    fn test_transport_connection_failure() {
        let core = classify_ureq_error(&ureq::Error::ConnectionFailed);
        assert!(matches!(
            classify_core_error(core),
            TsaError::Transport(_)
        ));

        let dns = classify_ureq_error(&ureq::Error::HostNotFound);
        assert!(matches!(classify_core_error(dns), TsaError::Transport(_)));
    }

    #[test]
    fn test_digest_mismatch() {
        let core = classify_tsp_http_client_error(&tsp_http_client::Error::DigestMismatch);
        let tsa = classify_core_error(core);
        assert!(matches!(tsa, TsaError::Protocol(_)));
        assert!(!matches!(tsa, TsaError::Transport(_)));
    }

    #[test]
    fn test_protocol_rejection() {
        let rejected = classify_tsp_http_client_error(&tsp_http_client::Error::RequestNotAccepted(
            None,
        ));
        assert!(matches!(
            classify_core_error(rejected),
            TsaError::Protocol(_)
        ));

        let invalid =
            classify_tsp_http_client_error(&tsp_http_client::Error::InvalidServerResponse);
        assert!(matches!(
            classify_core_error(invalid),
            TsaError::Protocol(_)
        ));
    }

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
        assert!(matches!(mapped, TsaError::RequestFailed(_)));
        assert!(!matches!(mapped, TsaError::Transport(_)));
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
        let provider = provider_from_config(&cfg);
        let err = provider
            .timestamp(&[0u8; 32])
            .await
            .expect_err("disabled");
        assert!(matches!(err, TsaError::Disabled));
    }

    #[test]
    fn test_box_roundtrip_preserves_categories() {
        let transport_box: Box<dyn std::error::Error + 'static> =
            Box::new(ureq::Error::Timeout(ureq::Timeout::Global));
        assert!(matches!(
            classify_core_error(classify_tsp_http_box_error(transport_box)),
            TsaError::Transport(_)
        ));

        let protocol_box: Box<dyn std::error::Error + 'static> =
            Box::new(tsp_http_client::Error::DigestMismatch);
        assert!(matches!(
            classify_core_error(classify_tsp_http_box_error(protocol_box)),
            TsaError::Protocol(_)
        ));

        // Unclassified fallback stays RequestFailed, not Transport.
        let fallback = classify_core_error(CoreError::InvalidResponse("opaque".into()));
        assert!(matches!(fallback, TsaError::RequestFailed(_)));
    }
}
