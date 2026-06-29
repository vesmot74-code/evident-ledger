use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::to_string_pretty;

use super::types::TsaAttestation;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TsaWriterPayload {
    bundle_hash: String,
    attestation: TsaAttestation,
}

#[derive(Debug, Clone)]
pub struct FileSystemTsaWriter {
    root: PathBuf,
}

impl FileSystemTsaWriter {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn write_attestation(
        &self,
        repo: &str,
        bundle_hash: &str,
        attestation: &TsaAttestation,
    ) -> Result<PathBuf> {
        let path = self.attestation_path(repo, bundle_hash)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create tsa writer dir {}", parent.display()))?;
        }

        let payload = TsaWriterPayload {
            bundle_hash: bundle_hash.to_string(),
            attestation: attestation.clone(),
        };
        let content = to_string_pretty(&payload).context("serialize tsa attestation")?;
        fs::write(&path, content)
            .with_context(|| format!("write tsa attestation {}", path.display()))?;
        Ok(path)
    }

    fn attestation_path(&self, repo: &str, bundle_hash: &str) -> Result<PathBuf> {
        let (owner, name) = super::job_store::parse_repo_full_name(repo)
            .with_context(|| format!("parse repo full_name {repo}"))?;
        Ok(self
            .root
            .join(owner)
            .join(name)
            .join(".audit")
            .join("tsa")
            .join(format!("{bundle_hash}.json")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tsa::create_stub_attestation;

    #[test]
    fn writes_attestation_json_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemTsaWriter::new(dir.path());
        let attestation = create_stub_attestation("a".repeat(64).as_str(), "stub");

        let path = writer
            .write_attestation("owner/repo", "hash123", &attestation)
            .unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("stub"));
        assert!(content.contains("hash123"));
    }
}
