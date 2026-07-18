//! Argon2id password hashing for web authentication (Stage 8.3.0).

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2, Params, Version,
};

const MEMORY_KIB: u32 = 19456;
const ITERATIONS: u32 = 2;
const PARALLELISM: u32 = 1;

#[derive(Debug)]
pub enum PasswordError {
    HashFailed,
    InvalidHash,
}

pub fn validate_password(password: &str) -> bool {
    password.chars().count() >= 8
}

fn argon2_instance() -> Result<Argon2<'static>, PasswordError> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, None)
        .map_err(|_| PasswordError::HashFailed)?;
    Ok(Argon2::new(
        argon2::Algorithm::Argon2id,
        Version::V0x13,
        params,
    ))
}

pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = argon2_instance()?;
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordError::HashFailed)
}

pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(password_hash).map_err(|_| PasswordError::InvalidHash)?;
    Ok(argon2_instance()?
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_does_not_contain_plaintext() {
        let password = "super_secret_password";
        let hash = hash_password(password).expect("hash");
        assert!(!hash.contains(password));
        assert!(verify_password(password, &hash).expect("verify"));
        assert!(!verify_password("wrong_password", &hash).expect("verify"));
    }
}
