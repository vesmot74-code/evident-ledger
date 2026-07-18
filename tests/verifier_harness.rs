//! Shared harness for `evident-verify` integration tests.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub fn evident_verify_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_evident_verify") {
        return PathBuf::from(path);
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_root = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("target"));
    let profile = std::env::var("CARGO_PROFILE").unwrap_or_else(|_| "debug".into());
    let path = target_root.join(profile).join("evident-verify");
    assert!(
        path.exists(),
        "evident-verify binary not found at {} (run via `cargo test`)",
        path.display()
    );
    path
}

pub fn fixture_proof_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proof.json")
}

pub fn load_proof() -> serde_json::Value {
    let content = std::fs::read_to_string(fixture_proof_path()).expect(
        "tests/fixtures/proof.json missing — run: curl -s http://localhost:3000/verify/proof/169ec981-a564-49ce-8425-20a90b97adc6 > tests/fixtures/proof.json",
    );
    serde_json::from_str(&content).expect("invalid proof fixture JSON")
}

fn seed_evident_home(home: &Path) {
    let evident_dir = home.join(".evident");
    std::fs::create_dir_all(&evident_dir).expect("create isolated .evident");
    let fixture_key =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server_identity.pub");
    std::fs::copy(&fixture_key, evident_dir.join("server_identity.pub"))
        .expect("tests/fixtures/server_identity.pub missing");
}

/// Runs the pre-built `evident-verify` binary with isolated `HOME` and proof file.
pub fn run_verifier(proof: &serde_json::Value) -> (String, i32) {
    let home_dir = TempDir::new().expect("isolated HOME tempdir");
    seed_evident_home(home_dir.path());

    let proof_dir = TempDir::new().expect("proof tempdir");
    let proof_path = proof_dir.path().join("proof.json");
    std::fs::write(
        &proof_path,
        serde_json::to_string(proof).expect("serialize proof"),
    )
    .expect("write proof temp file");

    let output = Command::new(evident_verify_bin())
        .arg(&proof_path)
        .env("HOME", home_dir.path())
        .output()
        .expect("spawn evident-verify");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}{stderr}");
    let code = output.status.code().unwrap_or(-1);
    (combined, code)
}
