mod builder;
mod pdf;
mod registration;
mod report;

pub use builder::generate_report;
pub use pdf::ReportError;
pub use registration::generate_registration_snapshot;
pub use report::{EventSummary, FileStatus, ProofData, TsaData, VerificationContext};

pub type Result<T> = std::result::Result<T, ReportError>;
