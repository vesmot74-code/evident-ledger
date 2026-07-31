//! External TSA attestation layer — optional time evidence, not chain truth.
//!
//! Adapter hook for `notary-core` TSA (`crates/tsa`, FreeTSA/openssl) lives here;
//! stage 12 ships with stub attestation so bundles stay offline-verifiable.

mod attest;
mod job_store;
mod read_verify;
mod trust_config;
mod types;
mod verify;
mod writer;

pub use attest::{create_stub_attestation, submit_bundle_hash_stub};
pub use job_store::{process_pending_job, FileSystemTsaJobStore, TsaJobStore};
pub use read_verify::{
    cached_status_if_fresh, is_evident_stub_json_token, resolve_and_cache_tsa_verification,
    stubs_allowed_in_current_env, token_sha256_hex, verify_token_fresh, CachedTsaVerification,
};
pub use trust_config::{
    check_tsa_configuration, enforce_tsa_trust_at_startup, freetsa_trust_path_options_from_env,
    is_production_tsa_env, verification_reason_for_config_error, TsaConfigError,
};
pub use types::{
    TsaAttestation, TsaJob, TsaJobState, TsaStatus, TsaTrustLevel, TsaVerificationOutcome,
    TsaVerificationReason, TsaVerificationStatus,
};
pub use verify::{tsa_status_for_bundle, verify_tsa_attestation};
pub use writer::FileSystemTsaWriter;
