mod report;
mod builder;
mod pdf;

pub use report::{ProofData, EventSummary, TsaData};
pub use builder::generate_report;
pub use pdf::ReportError;

pub type Result<T> = std::result::Result<T, ReportError>;
