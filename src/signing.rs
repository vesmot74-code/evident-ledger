use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct ServerSigner {
    signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
}

#[derive(Debug)]
pub enum SigningError {
    ReadFailed(std::io::Error),
    InvalidKeyLength,
    CreateDirFailed(std::io::Error),
    WriteFailed(std::io::Error),
}

impl std::fmt::Display for SigningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SigningError::ReadFailed(e) => write!(f, "failed to read signing key: {e}"),
            SigningError::InvalidKeyLength => write!(f, "invalid signing key length"),
            SigningError::CreateDirFailed(e) => {
                write!(f, "failed to create signing key directory: {e}")
            }
            SigningError::WriteFailed(e) => write!(f, "failed to write signing key: {e}"),
        }
    }
}

impl ServerSigner {
    pub fn load_or_create(path: &str) -> Result<Self, SigningError> {
        let path_ref = Path::new(path);
        if path_ref.exists() {
            let bytes = std::fs::read(path).map_err(SigningError::ReadFailed)?;
            let array: [u8; 32] = bytes
                .try_into()
                .map_err(|_| SigningError::InvalidKeyLength)?;
            let signing_key = SigningKey::from_bytes(&array);
            let verifying_key = signing_key.verifying_key();
            return Ok(Self {
                signing_key,
                verifying_key,
            });
        }
        let signing_key = SigningKey::generate(&mut OsRng);
        if let Some(parent) = path_ref.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(SigningError::CreateDirFailed)?;
            }
        }
        std::fs::write(path, signing_key.to_bytes()).map_err(SigningError::WriteFailed)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
        }
        let display = PathBuf::from(path);
        let display = if display.is_absolute() {
            display
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or(display)
        };
        eprintln!(
            "WARNING: created new server signing key at {}",
            display.display()
        );
        let verifying_key = signing_key.verifying_key();
        Ok(Self {
            signing_key,
            verifying_key,
        })
    }

    pub fn sign_root(&self, chain_id: &str, merkle_root: &str, chain_head: &str) -> String {
        let message = format!("{}:{}:{}", chain_id, merkle_root, chain_head);
        let signature: Signature = self.signing_key.sign(message.as_bytes());
        hex::encode(signature.to_bytes())
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.to_bytes())
    }

    pub fn sha256_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.signing_key.to_bytes());
        hex::encode(hasher.finalize())
    }
}

pub fn verify_root(
    chain_id: &str,
    merkle_root: &str,
    chain_head: &str,
    signature_hex: &str,
    public_key_hex: &str,
) -> bool {
    let Ok(sig_bytes) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(pk_bytes) = hex::decode(public_key_hex) else {
        return false;
    };
    let Ok(sig_array): Result<[u8; 64], _> = sig_bytes.try_into() else {
        return false;
    };
    let Ok(pk_array): Result<[u8; 32], _> = pk_bytes.try_into() else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_array) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);

    // Only the versioned format is accepted: chain_id:merkle_root:chain_head.
    // No legacy fallback (merkle_root:chain_head) — that would accept a signature
    // for any chain_id and break cryptographic chain binding.
    let message = format!("{}:{}:{}", chain_id, merkle_root, chain_head);
    verifying_key.verify(message.as_bytes(), &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;
    use rand::rngs::OsRng;
    use uuid::Uuid;

    #[test]
    fn legacy_signature_without_chain_id_is_rejected() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let merkle_root = "a".repeat(64);
        let chain_head = Uuid::new_v4().to_string();
        let expected_chain_id = Uuid::new_v4().to_string();
        let other_chain_id = Uuid::new_v4().to_string();

        // Old format: no chain_id in the signed message.
        let message_old = format!("{}:{}", merkle_root, chain_head);
        let signature_hex = hex::encode(signing_key.sign(message_old.as_bytes()).to_bytes());

        for chain_id in [expected_chain_id.as_str(), other_chain_id.as_str(), ""] {
            assert!(
                !verify_root(
                    chain_id,
                    &merkle_root,
                    &chain_head,
                    &signature_hex,
                    &public_key_hex,
                ),
                "legacy signature must be rejected for chain_id={chain_id:?}"
            );
        }
    }

    #[test]
    fn versioned_signature_binds_chain_id() {
        let signer = ServerSigner::load_or_create(
            &std::env::temp_dir()
                .join(format!("evident-signing-test-{}.bin", Uuid::new_v4()))
                .to_string_lossy(),
        )
        .expect("test signer");
        let merkle_root = "b".repeat(64);
        let chain_head = Uuid::new_v4().to_string();
        let chain_id = Uuid::new_v4().to_string();
        let other_chain_id = Uuid::new_v4().to_string();

        let signature_hex = signer.sign_root(&chain_id, &merkle_root, &chain_head);
        let public_key_hex = signer.public_key_hex();

        assert!(verify_root(
            &chain_id,
            &merkle_root,
            &chain_head,
            &signature_hex,
            &public_key_hex,
        ));
        assert!(!verify_root(
            &other_chain_id,
            &merkle_root,
            &chain_head,
            &signature_hex,
            &public_key_hex,
        ));
    }
}
