use std::path::PathBuf;
use std::process::Command;

fn setup_isolated_home() -> PathBuf {
    let home = PathBuf::from(format!("/tmp/evident_test_home_{}", uuid_simple()));
    let evident_dir = home.join(".evident");
    std::fs::create_dir_all(&evident_dir).expect("failed to create isolated test home");

    let fixture_key =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/server_identity.pub");
    std::fs::copy(&fixture_key, evident_dir.join("server_identity.pub"))
        .expect("tests/fixtures/server_identity.pub missing");

    home
}

fn run_verifier(proof_path: &str) -> (String, i32) {
    let home = setup_isolated_home();
    let output = Command::new("cargo")
        .args(["run", "--bin", "evident-verify", "--", proof_path])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HOME", &home)
        .output()
        .expect("failed to run verifier");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr);
    let code = output.status.code().unwrap_or(-1);
    (combined, code)
}

fn load_proof() -> serde_json::Value {
    let content = std::fs::read_to_string("tests/fixtures/proof.json")
        .expect("tests/fixtures/proof.json missing — run: curl -s http://localhost:3000/verify/proof/169ec981-a564-49ce-8425-20a90b97adc6 > tests/fixtures/proof.json");
    serde_json::from_str(&content).expect("invalid JSON")
}

fn write_temp(value: &serde_json::Value) -> String {
    let path = format!("/tmp/evident_test_{}.json", uuid_simple());
    std::fs::write(&path, serde_json::to_string(value).unwrap()).unwrap();
    path
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
        .to_string()
}

#[test]
fn valid_proof_exits_0() {
    let proof = load_proof();
    let path = write_temp(&proof);
    let (_, code) = run_verifier(&path);
    assert_eq!(code, 0);
}

#[test]
fn tampered_signature_exits_2() {
    let mut proof = load_proof();
    proof["proof"]["signature"] = serde_json::json!("ff".repeat(64));
    let path = write_temp(&proof);
    let (output, code) = run_verifier(&path);
    assert_eq!(code, 2);
    assert!(output.contains("signature invalid"));
}

#[test]
fn tampered_head_event_id_exits_2() {
    let mut proof = load_proof();
    proof["head_event_id"] = serde_json::json!("00000000-0000-0000-0000-000000000000");
    let path = write_temp(&proof);
    let (output, code) = run_verifier(&path);
    assert_eq!(code, 2);
    assert!(output.contains("head_event_id"));
}

#[test]
fn missing_event_exits_2() {
    let mut proof = load_proof();
    let events = proof["events"].as_array_mut().unwrap();
    events.pop();
    let path = write_temp(&proof);
    let (output, code) = run_verifier(&path);
    assert_eq!(code, 2);
    assert!(output.contains("leaves_count"));
}
