mod builder;
mod pdf;
mod report;

pub use builder::generate_report;
pub use pdf::ReportError;
pub use report::{EventSummary, FileStatus, ProofData, TsaData, VerificationContext};

pub type Result<T> = std::result::Result<T, ReportError>;
