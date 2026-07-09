use crate::pdf::write_pdf;
use crate::{ProofData, ReportError, Result, VerificationContext};
use std::path::Path;

pub fn generate_report(
    chain_id: &str,
    proof_data: &ProofData,
    verification: &VerificationContext,
    output_path: &Path,
) -> Result<()> {
    // Проверка: chain_id должен совпадать
    if chain_id != proof_data.chain_id {
        return Err(ReportError::InvalidProofData);
    }

    // Проверка: должен быть хотя бы один event
    if proof_data.events.is_empty() {
        return Err(ReportError::InvalidProofData);
    }

    write_pdf(proof_data, verification, output_path)
}
