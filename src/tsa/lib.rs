//! External TSA attestation layer — optional time evidence, not chain truth.
//!
//! Adapter hook for `notary-core` TSA (`crates/tsa`, FreeTSA/openssl) lives here;
//! stage 12 ships with stub attestation so bundles stay offline-verifiable.

mod attest;
mod job_store;
mod types;
mod verify;
mod writer;

pub use attest::{create_stub_attestation, submit_bundle_hash_stub};
pub use job_store::{process_pending_job, FileSystemTsaJobStore, TsaJobStore};
pub use types::{TsaAttestation, TsaJob, TsaJobState, TsaStatus};
pub use verify::{tsa_status_for_bundle, verify_tsa_attestation};
pub use writer::FileSystemTsaWriter;
