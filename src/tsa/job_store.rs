use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{TsaAttestation, TsaJob, TsaJobState};

pub trait TsaJobStore: Send + Sync {
    fn get_job(&self, repo: &str, bundle_hash: &str) -> Result<Option<TsaJob>>;
    fn save_job(&self, job: &TsaJob) -> Result<()>;
    fn enqueue(&self, repo: &str, bundle_hash: &str) -> Result<TsaJob> {
        let job = TsaJob {
            repo: repo.to_string(),
            bundle_hash: bundle_hash.to_string(),
            state: TsaJobState::Pending,
            attestation: None,
            error: None,
        };
        self.save_job(&job)?;
        Ok(job)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct JobFile {
    jobs: Vec<TsaJob>,
}

#[derive(Debug, Clone)]
pub struct FileSystemTsaJobStore {
    root: PathBuf,
}

impl FileSystemTsaJobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn job_file_path(&self, repo: &str) -> Result<PathBuf> {
        let (owner, name) = parse_repo_full_name(repo)?;
        Ok(self
            .root
            .join(owner)
            .join(name)
            .join(".audit")
            .join("tsa-jobs.json"))
    }

    fn load_file(&self, repo: &str) -> Result<JobFile> {
        let path = self.job_file_path(repo)?;
        if !path.exists() {
            return Ok(JobFile::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read tsa jobs {}", path.display()))?;
        Ok(serde_json::from_str(&content)
            .with_context(|| format!("parse tsa jobs {}", path.display()))?)
    }

    fn save_file(&self, repo: &str, data: &JobFile) -> Result<()> {
        let path = self.job_file_path(repo)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create tsa job dir {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(data).context("serialize tsa jobs")?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, content)
            .with_context(|| format!("write temp tsa jobs {}", tmp.display()))?;
        fs::rename(&tmp, &path)
            .with_context(|| format!("atomic rename tsa jobs to {}", path.display()))?;
        Ok(())
    }
}

impl TsaJobStore for FileSystemTsaJobStore {
    fn get_job(&self, repo: &str, bundle_hash: &str) -> Result<Option<TsaJob>> {
        let file = self.load_file(repo)?;
        Ok(file
            .jobs
            .into_iter()
            .find(|j| j.bundle_hash == bundle_hash))
    }

    fn save_job(&self, job: &TsaJob) -> Result<()> {
        let mut file = self.load_file(&job.repo)?;
        if let Some(existing) = file
            .jobs
            .iter_mut()
            .find(|j| j.bundle_hash == job.bundle_hash)
        {
            *existing = job.clone();
        } else {
            file.jobs.push(job.clone());
        }
        self.save_file(&job.repo, &file)
    }
}

pub fn parse_repo_full_name(full_name: &str) -> Result<(&str, &str)> {
    let (owner, repo) = full_name
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("invalid repo full_name: {full_name}"))?;
    if owner.is_empty() || repo.is_empty() {
        anyhow::bail!("invalid repo full_name: {full_name}");
    }
    Ok((owner, repo))
}

pub async fn process_pending_job(
    store: &FileSystemTsaJobStore,
    repo: &str,
    bundle_hash: &str,
    provider: &str,
) -> Result<Option<TsaAttestation>> {
    let mut job = store
        .get_job(repo, bundle_hash)?
        .unwrap_or(TsaJob {
            repo: repo.to_string(),
            bundle_hash: bundle_hash.to_string(),
            state: TsaJobState::Pending,
            attestation: None,
            error: None,
        });

    job.state = TsaJobState::Sent;
    store.save_job(&job)?;

    match crate::attest::submit_bundle_hash_stub(bundle_hash, provider) {
        Ok(attestation) => {
            job.state = TsaJobState::Verified;
            job.attestation = Some(attestation.clone());
            job.error = None;
            store.save_job(&job)?;
            Ok(Some(attestation))
        }
        Err(err) => {
            job.state = TsaJobState::Failed;
            job.error = Some(err.to_string());
            store.save_job(&job)?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemTsaJobStore::new(dir.path());
        let job = store.enqueue("owner/repo", "hash123").unwrap();
        assert_eq!(job.state, TsaJobState::Pending);
        let loaded = store.get_job("owner/repo", "hash123").unwrap().unwrap();
        assert_eq!(loaded.bundle_hash, "hash123");
    }
}
