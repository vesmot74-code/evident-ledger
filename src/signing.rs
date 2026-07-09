use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use std::path::Path;

pub struct ServerSigner {
    signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
}

impl ServerSigner {
    pub fn load_or_create(path: &str) -> Self {
        if Path::new(path).exists() {
            let bytes = std::fs::read(path).expect("Failed to read signing key");
            let array: [u8; 32] = bytes.try_into().expect("Invalid key length");
            let signing_key = SigningKey::from_bytes(&array);
            let verifying_key = signing_key.verifying_key();
            return Self {
                signing_key,
                verifying_key,
            };
        }
        let signing_key = SigningKey::generate(&mut OsRng);
        std::fs::write(path, signing_key.to_bytes()).expect("Failed to write signing key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
        }
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    pub fn sign_root(&self, chain_id: &str, merkle_root: &str, chain_head: &str) -> String {
        let message = format!("{}:{}:{}", chain_id, merkle_root, chain_head);
        let signature: Signature = self.signing_key.sign(message.as_bytes());
        hex::encode(signature.to_bytes())
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.to_bytes())
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

    // Пробуем новый формат (с chain_id)
    let message_new = format!("{}:{}:{}", chain_id, merkle_root, chain_head);
    if verifying_key
        .verify(message_new.as_bytes(), &signature)
        .is_ok()
    {
        return true;
    }

    // Пробуем старый формат (без chain_id)
    let message_old = format!("{}:{}", merkle_root, chain_head);
    verifying_key
        .verify(message_old.as_bytes(), &signature)
        .is_ok()
}
