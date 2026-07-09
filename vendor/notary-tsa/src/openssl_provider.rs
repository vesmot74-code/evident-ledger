use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tempfile::NamedTempFile;
use thiserror::Error;

const TSA_HTTP_TIMEOUT_SECS: u64 = 30;
const FREETSA_CA_CERT_URL: &str = "https://freetsa.org/files/cacert.pem";

#[derive(Debug, Error)]
pub enum OpensslAdapterError {
    #[error("openssl command failed: {stderr}")]
    CommandFailed { stderr: String },
    #[error("openssl ts -verify failed: exit={exit_code}, stdout={stdout}, stderr={stderr}")]
    VerifyFailed {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    #[error("TSA HTTP request failed: status={status}, body={body}")]
    HttpFailed { status: u16, body: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct OpenSslTsaProvider {
    pub tsa_url: String,
    pub ca_cert_path: PathBuf,
    pub untrusted_cert_path: PathBuf,
}

impl OpenSslTsaProvider {
    pub fn new(tsa_url: String, ca_cert_path: PathBuf, untrusted_cert_path: PathBuf) -> Self {
        Self {
            tsa_url,
            ca_cert_path,
            untrusted_cert_path,
        }
    }

    pub async fn send_tsq(&self, tsq_path: &Path) -> Result<PathBuf, OpensslAdapterError> {
        let tsq_bytes = tokio::fs::read(tsq_path).await?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TSA_HTTP_TIMEOUT_SECS))
            .build()
            .map_err(|e| {
                OpensslAdapterError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;

        let response = client
            .post(&self.tsa_url)
            .header("Content-Type", "application/timestamp-query")
            .body(tsq_bytes)
            .send()
            .await
            .map_err(|e| {
                OpensslAdapterError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;

        let status = response.status();
        let body = response.bytes().await.map_err(|e| {
            OpensslAdapterError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;

        if !status.is_success() {
            return Err(OpensslAdapterError::HttpFailed {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).to_string(),
            });
        }

        let mut temp = NamedTempFile::with_suffix(".tsr")?;
        temp.write_all(&body)?;
        temp.flush()?;
        let (_file, path) = temp.keep().map_err(|e| OpensslAdapterError::Io(e.error))?;
        Ok(path)
    }

    pub fn verify_reply(
        &self,
        tsr_path: &Path,
        digest_path: &Path,
    ) -> Result<(), OpensslAdapterError> {
        if !self.ca_cert_path.is_file() {
            return Err(OpensslAdapterError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("CA certificate not found: {}", self.ca_cert_path.display()),
            )));
        }

        if !self.untrusted_cert_path.is_file() {
            return Err(OpensslAdapterError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "untrusted certificate not found: {}",
                    self.untrusted_cert_path.display()
                ),
            )));
        }

        let output = Self::openssl_verify_output(
            tsr_path,
            digest_path,
            &self.ca_cert_path,
            &self.untrusted_cert_path,
        )?;

        Self::validate_verify_output(&output)
    }

    fn openssl_verify_output(
        tsr_path: &Path,
        digest_path: &Path,
        ca_cert_path: &Path,
        untrusted_cert_path: &Path,
    ) -> Result<std::process::Output, OpensslAdapterError> {
        let output = Command::new("openssl")
            .arg("ts")
            .arg("-verify")
            .arg("-in")
            .arg(tsr_path)
            .arg("-data")
            .arg(digest_path)
            .arg("-CAfile")
            .arg(ca_cert_path)
            .arg("-untrusted")
            .arg(untrusted_cert_path)
            .output()?;
        Ok(output)
    }

    fn validate_verify_output(output: &std::process::Output) -> Result<(), OpensslAdapterError> {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let all_output = format!("{}\n{}", stdout, stderr);
        let verification_ok = all_output.contains("Verification: OK");

        if output.status.success() && verification_ok {
            Ok(())
        } else {
            Err(OpensslAdapterError::VerifyFailed {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            })
        }
    }

    fn write_digest(hash: &[u8]) -> Result<PathBuf, OpensslAdapterError> {
        if hash.is_empty() {
            return Err(OpensslAdapterError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "digest must not be empty",
            )));
        }

        let mut file = NamedTempFile::new()?;
        file.write_all(hash)?;
        file.flush()?;
        let (_file, path) = file.keep().map_err(|e| OpensslAdapterError::Io(e.error))?;
        Ok(path)
    }

    fn build_tsq(digest_path: &Path) -> Result<PathBuf, OpensslAdapterError> {
        let digest_path_str = Self::path_to_str(digest_path)?;
        let tsq_temp = NamedTempFile::with_suffix(".tsq")?;
        let tsq_out = tsq_temp.path().to_path_buf();
        let tsq_out_str = Self::path_to_str(&tsq_out)?;

        let output = Command::new("openssl")
            .args([
                "ts",
                "-query",
                "-data",
                digest_path_str,
                "-sha256",
                "-no_nonce",
                "-out",
                tsq_out_str,
            ])
            .output()?;

        if !output.status.success() {
            return Err(OpensslAdapterError::CommandFailed {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let (_file, path) = tsq_temp
            .keep()
            .map_err(|e| OpensslAdapterError::Io(e.error))?;
        Ok(path)
    }

    fn path_to_str(path: &Path) -> Result<&str, OpensslAdapterError> {
        path.to_str().ok_or_else(|| {
            OpensslAdapterError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("non-UTF-8 path: {}", path.display()),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex;

    #[tokio::test]
    #[ignore = "requires FREETSA_CA_CERT_PATH env and network"]
    async fn send_tsq_freetsa_smoke() {
        const FREETSA_URL: &str = "https://freetsa.org/tsr";

        let ca_cert_path = std::env::var("FREETSA_CA_CERT_PATH")
            .map(PathBuf::from)
            .expect("set FREETSA_CA_CERT_PATH to cacert.pem");
        let untrusted_cert_path = std::env::var("FREETSA_UNTRUSTED_CERT_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("tsa.crt"));

        let hash = hex::decode("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
            .expect("decode test hash");
        let digest_path = OpenSslTsaProvider::write_digest(&hash).expect("write digest");
        let tsq_path = OpenSslTsaProvider::build_tsq(&digest_path).expect("build tsq");

        let provider =
            OpenSslTsaProvider::new(FREETSA_URL.into(), ca_cert_path, untrusted_cert_path);
        let tsr_path = provider.send_tsq(&tsq_path).await.expect("send tsq");

        std::fs::remove_file(digest_path).expect("cleanup digest");
        std::fs::remove_file(tsq_path).expect("cleanup tsq");
        std::fs::remove_file(tsr_path).expect("cleanup tsr");
    }

    #[tokio::test]
    #[ignore = "requires FREETSA_CA_CERT_PATH env and network"]
    async fn verify_reply_freetsa_smoke() {
        const FREETSA_URL: &str = "https://freetsa.org/tsr";

        let ca_cert_path = std::env::var("FREETSA_CA_CERT_PATH")
            .map(PathBuf::from)
            .expect("set FREETSA_CA_CERT_PATH to cacert.pem");
        let untrusted_cert_path = std::env::var("FREETSA_UNTRUSTED_CERT_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("tsa.crt"));

        let hash = hex::decode("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
            .expect("decode test hash");
        let digest_path = OpenSslTsaProvider::write_digest(&hash).expect("write digest");
        let tsq_path = OpenSslTsaProvider::build_tsq(&digest_path).expect("build tsq");

        let provider = OpenSslTsaProvider::new(
            FREETSA_URL.into(),
            ca_cert_path.clone(),
            untrusted_cert_path.clone(),
        );
        let tsr_path = provider.send_tsq(&tsq_path).await.expect("send tsq");

        provider
            .verify_reply(&tsr_path, &digest_path)
            .expect("verify_reply must pass for FreeTSA TSR");

        std::fs::remove_file(digest_path).expect("cleanup digest");
        std::fs::remove_file(tsq_path).expect("cleanup tsq");
        std::fs::remove_file(tsr_path).expect("cleanup tsr");
    }
}
