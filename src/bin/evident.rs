use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::process::Command as ProcessCommand;

use evident_ledger::freeze::{self, Event};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TsaData {
    timestamp: i64,
    serial: String,
    token_bytes: usize,
}

#[derive(Debug)]
enum CliError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Http(reqwest::Error),
    Server(String),
    Usage(String),
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}
impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::Json(e)
    }
}
impl From<reqwest::Error> for CliError {
    fn from(e: reqwest::Error) -> Self {
        CliError::Http(e)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Io(e) => write!(f, "I/O error: {e}"),
            CliError::Json(e) => write!(f, "JSON error: {e}"),
            CliError::Http(e) => write!(f, "HTTP error: {e}"),
            CliError::Server(m) => write!(f, "{m}"),
            CliError::Usage(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CliError {}

fn evident_dir() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".evident")
}

#[derive(Debug, Clone, Copy)]
enum KeySource {
    Env,
    File,
}

fn load_api_key_with_source() -> Result<(String, KeySource), CliError> {
    if let Ok(val) = std::env::var("EVIDENT_API_KEY") {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            return Ok((trimmed, KeySource::Env));
        }
    }

    if let Ok(content) = fs::read_to_string(evident_dir().join("api_key")) {
        let trimmed = content.trim().to_string();
        if !trimmed.is_empty() {
            return Ok((trimmed, KeySource::File));
        }
    }

    Err(CliError::Usage(
        "No API key found. Set EVIDENT_API_KEY or create ~/.evident/api_key".into(),
    ))
}

fn load_api_key() -> Result<String, CliError> {
    load_api_key_with_source().map(|(key, _)| key)
}

fn api_key_fingerprint(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    hash[..12.min(hash.len())].to_string()
}

#[derive(Debug, Deserialize)]
struct CommitResponse {
    event_id: String,
    chain_id: String,
    head_event_id: String,
    proof: ProofPayload,
    events: Vec<EventLeaf>,
    tsa: Option<TsaData>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofFile {
    chain_id: String,
    head_event_id: String,
    proof: ProofPayload,
    events: Vec<EventLeaf>,
    tsa: Option<TsaData>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofPayload {
    root: String,
    chain_head: String,
    signature: String,
    public_key: String,
    leaves_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct EventLeaf {
    sequence: i64,
    event_id: String,
    parent_event_id: String,
    file_hash: String,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("init") => cmd_init(),
        Some("help") | Some("--help") | Some("-h") => {
            println!(
                "usage: evident <init|new-chain|commit|verify|status|account|key status|key info|report generate>"
            );
            Ok(())
        }
        Some("new-chain") => cmd_new_chain(),
        Some("report") => {
            let subcommand = args.next().ok_or_else(|| {
                CliError::Usage("usage: evident report generate <chain_id>".into())
            })?;
            if subcommand != "generate" {
                return Err(CliError::Usage(
                    "usage: evident report generate <chain_id>".into(),
                ));
            }
            let chain_id = args.next().ok_or_else(|| {
                CliError::Usage("usage: evident report generate <chain_id>".into())
            })?;
            cmd_report_generate(&chain_id)
        }
        Some("status") => {
            let chain_id = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident status <chain_id>".into()))?;
            cmd_status(&chain_id)
        }
        Some("hash") => {
            let path = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident hash <file>".into()))?;
            cmd_hash(&path)
        }
        Some("commit") => {
            let path = args.next().ok_or_else(|| {
                CliError::Usage("usage: evident commit <file> --chain <id>".into())
            })?;
            let mut chain_id = None;
            while let Some(arg) = args.next() {
                if arg == "--chain" {
                    chain_id = args.next();
                }
            }
            let chain_id = chain_id.ok_or_else(|| CliError::Usage("missing --chain".into()))?;
            cmd_commit(&path, &chain_id)
        }
        Some("verify") => {
            let proof_path = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident verify <proof.json>".into()))?;
            cmd_verify(&proof_path)
        }
        Some("account") => cmd_account(),
        Some("key") => {
            let subcommand = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident key <status|info>".into()))?;
            match subcommand.as_str() {
                "status" => cmd_key_status(),
                "info" => cmd_key_info(),
                _ => Err(CliError::Usage("usage: evident key <status|info>".into())),
            }
        }
        _ => Err(CliError::Usage(
            "usage: evident <init|new-chain|commit|verify|status|account|key status|key info|report generate>"
                .into(),
        )),
    }
}

fn cmd_init() -> Result<(), CliError> {
    let dir = evident_dir();
    fs::create_dir_all(&dir)?;
    let key_path = dir.join("identity.key");
    let pub_path = dir.join("identity.pub");
    if key_path.exists() {
        let pub_bytes = fs::read(&pub_path)?;
        println!("identity already exists");
        println!("public key: {}", hex::encode(&pub_bytes));
        return Ok(());
    }
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();
    fs::write(&key_path, signing_key.to_bytes())?;
    fs::write(&pub_path, verifying_key.to_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    }
    println!("identity created");
    println!("public key: {}", hex::encode(verifying_key.to_bytes()));
    Ok(())
}

fn cmd_hash(path: &str) -> Result<(), CliError> {
    let bytes = fs::read(path)?;
    println!("{}", sha256_hex(&bytes));
    Ok(())
}

fn cmd_commit(path: &str, chain_id: &str) -> Result<(), CliError> {
    let bytes = fs::read(path)?;
    let chain_uuid =
        Uuid::parse_str(chain_id).map_err(|_| CliError::Usage("invalid chain id".into()))?;

    let client = evident_ledger::client::EvidentClient::new("http://127.0.0.1:3000");
    let (commit, proof_path, file_hash) =
        client
            .submit_event(chain_uuid, &bytes)
            .map_err(|e| match e {
                evident_ledger::client::ClientError::Http(err) => CliError::Http(err),
                evident_ledger::client::ClientError::Io(err) => CliError::Io(err),
                evident_ledger::client::ClientError::Json(err) => CliError::Json(err),
                evident_ledger::client::ClientError::Server(s) => CliError::Server(s),
            })?;

    let event = Event::from_payload(&commit.chain_id, 1, &file_hash, "", "commit");
    let event_log = evident_dir().join("events.jsonl");
    freeze::append_event_log(&event_log, &event)?;

    let capabilities = fetch_capabilities_best_effort();
    print_commit_success(&commit.event_id, &proof_path, capabilities.as_ref());
    Ok(())
}

/// Пытается получить capabilities аккаунта для расширенного вывода commit.
/// Best-effort: любая ошибка (нет ключа, сервер недоступен, битый JSON)
/// молча игнорируется — commit уже состоялся, отсутствие этой информации
/// не должно превращаться в ошибку всей команды.
fn fetch_capabilities_best_effort() -> Option<serde_json::Value> {
    let api_key = load_api_key().ok()?;

    let client = reqwest::blocking::Client::new();
    client
        .get("http://127.0.0.1:3000/account/capabilities")
        .header("X-API-KEY", &api_key)
        .send()
        .ok()?
        .json()
        .ok()
}

/// Печатает результат успешного commit. Вынесено в отдельную функцию, чтобы
/// в будущем сюда можно было добавить тариф-специфичный вывод (Machine vs
/// Qualified TSA, "Server backup enabled" для Vault, "Identity signature
/// attached" для Identity) без раздувания cmd_commit.
/// Если capabilities удалось получить (Some), дополнительно печатает
/// Trust Level / Plan / Available upgrades.
fn print_commit_success(
    event_id: &str,
    proof_path: &Path,
    capabilities: Option<&serde_json::Value>,
) {
    println!("anchored    event={}", event_id);
    println!("proof       {}", proof_path.display());

    let Some(caps) = capabilities else {
        return;
    };

    let plan = caps["plan_name"].as_str().unwrap_or("unknown");
    let tsa_mode = caps["tsa_mode"].as_str().unwrap_or("machine");
    let server_backup = caps["server_backup"].as_bool().unwrap_or(false);
    let identity_enabled = caps["identity_enabled"].as_bool().unwrap_or(false);

    let trust_level = if tsa_mode == "qualified" && identity_enabled {
        "High (Qualified TSA + Identity)"
    } else if tsa_mode == "qualified" {
        "Elevated (Qualified TSA)"
    } else {
        "Standard (Machine TSA)"
    };

    println!();
    println!("Trust Level {}", trust_level);
    println!("Plan        {}", plan.to_uppercase());

    let mut upgrades: Vec<&str> = Vec::new();
    if tsa_mode != "qualified" {
        upgrades.push("Qualified TSA");
    }
    if !server_backup {
        upgrades.push("Vault Backup");
    }
    if !identity_enabled {
        upgrades.push("Identity");
    }

    if !upgrades.is_empty() {
        println!("Available upgrades: {}", upgrades.join(", "));
    }
}

fn cmd_verify(proof_path: &str) -> Result<(), CliError> {
    let verifier = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("evident-verify");
    let status = ProcessCommand::new(verifier)
        .arg(proof_path)
        .status()
        .map_err(|e| CliError::Server(format!("failed to run verifier: {e}")))?;
    if !status.success() {
        return Err(CliError::Server("verification failed".into()));
    }
    Ok(())
}

fn report_artifact_paths(base_dir: &Path, chain_id: &str) -> (PathBuf, PathBuf) {
    let proof_dir = base_dir.join("proofs").join(chain_id);
    let proof_path = proof_dir.join("proof.json");
    let pdf_path = proof_dir.join("proof.pdf");
    (proof_path, pdf_path)
}

fn find_latest_proof_artifact(chain_id: &str) -> Result<PathBuf, CliError> {
    let proof_dir = evident_dir().join("proofs").join(chain_id);

    let mut paths: Vec<PathBuf> = fs::read_dir(&proof_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s != "proof.json")
                    .unwrap_or(true)
        })
        .collect();

    if paths.is_empty() {
        return Err(CliError::Usage(format!(
            "no proof found for chain {chain_id}"
        )));
    }

    paths.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    paths
        .pop()
        .ok_or_else(|| CliError::Usage(format!("no proof found for chain {chain_id}")))
}

fn cmd_report_generate(chain_id: &str) -> Result<(), CliError> {
    let source_path = find_latest_proof_artifact(chain_id)?;
    let output_path = evident_dir()
        .join("proofs")
        .join(chain_id)
        .join("proof.pdf");
    cmd_report(
        &source_path.to_string_lossy(),
        &output_path.to_string_lossy(),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn cmd_new_chain() -> Result<(), CliError> {
    let client = reqwest::blocking::Client::new();
    let mut req = client.post("http://127.0.0.1:3000/chains");
    if let Ok(key) = std::env::var("EVIDENT_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            req = req.header("X-API-KEY", key);
        }
    }
    let response = req.send()?;
    let status = response.status();
    let body = response.text()?;
    if !status.is_success() {
        return Err(CliError::Server(format!("server error {status}: {body}")));
    }
    let json: serde_json::Value = serde_json::from_str(&body)?;
    println!("chain created");
    println!("chain_id: {}", json["chain_id"].as_str().unwrap_or("?"));
    Ok(())
}

fn cmd_account() -> Result<(), CliError> {
    let client = reqwest::blocking::Client::new();
    let api_key = load_api_key()?;

    let capabilities: serde_json::Value = client
        .get("http://127.0.0.1:3000/account/capabilities")
        .header("X-API-KEY", &api_key)
        .send()?
        .json()?;

    let usage: serde_json::Value = client
        .get("http://127.0.0.1:3000/account/usage")
        .header("X-API-KEY", &api_key)
        .send()?
        .json()?;

    let plan = capabilities["plan_name"].as_str().unwrap_or("unknown");
    let tsa_mode = capabilities["tsa_mode"].as_str().unwrap_or("unknown");
    let server_backup = capabilities["server_backup"].as_bool().unwrap_or(false);
    let identity_enabled = capabilities["identity_enabled"].as_bool().unwrap_or(false);

    let server_commits = usage["server_commits"].as_i64().unwrap_or(0);
    let commits_limit = usage["monthly_commits_limit"]
        .as_i64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unlimited".to_string());
    let tsa_requests = usage["tsa_requests"].as_i64().unwrap_or(0);
    let tsa_limit = usage["monthly_tsa_limit"]
        .as_i64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unlimited".to_string());

    println!("Evident Ledger Account\n");
    println!("Plan: {}\n", plan.to_uppercase());
    println!("Capabilities:");
    println!("--------------------------------");
    println!(
        "TSA mode:        {}",
        if tsa_mode == "machine" {
            "Machine TSA"
        } else {
            "Qualified TSA"
        }
    );
    println!("Monthly commits:  {} / {}", server_commits, commits_limit);
    println!("TSA requests:     {} / {}", tsa_requests, tsa_limit);
    println!(
        "Server backup:    {}",
        if server_backup { "enabled" } else { "disabled" }
    );
    println!(
        "Identity:         {}",
        if identity_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("--------------------------------");

    Ok(())
}

fn cmd_key_status() -> Result<(), CliError> {
    let client = reqwest::blocking::Client::new();
    let api_key = load_api_key()?;

    let response = client
        .get("http://127.0.0.1:3000/account/key-status")
        .header("X-API-KEY", &api_key)
        .send()?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(CliError::Server("API key rejected by server".into()));
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(CliError::Server(format!("server error {status}: {body}")));
    }

    let json: serde_json::Value = response.json()?;

    let status = json["status"].as_str().unwrap_or("unknown").to_uppercase();
    let label = json["label"].as_str().unwrap_or("unknown");
    let created_at = json["created_at"].as_str().unwrap_or("unknown");

    println!("API Key");
    println!("Status:");
    println!("{status}");
    println!("Label:");
    println!("{label}");
    println!("Created:");
    println!("{created_at}");

    Ok(())
}

fn cmd_key_info() -> Result<(), CliError> {
    println!("API Key");

    match load_api_key_with_source() {
        Ok((key, source)) => {
            let source_label = match source {
                KeySource::Env => "env (EVIDENT_API_KEY)",
                KeySource::File => "file (~/.evident/api_key)",
            };
            println!("Configured: YES");
            println!("Source: {source_label}");
            println!("Fingerprint: {}", api_key_fingerprint(&key));
        }
        Err(_) => {
            println!("Status: NOT CONFIGURED");
            println!("Set EVIDENT_API_KEY or create:");
            println!("~/.evident/api_key");
        }
    }

    Ok(())
}

fn cmd_status(chain_id: &str) -> Result<(), CliError> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get(format!("http://127.0.0.1:3000/verify/{chain_id}"))
        .send()?;
    let status = response.status();
    let body = response.text()?;
    if !status.is_success() {
        return Err(CliError::Server(format!("server error {status}: {body}")));
    }
    let json: serde_json::Value = serde_json::from_str(&body)?;
    let valid = json["valid"].as_bool().unwrap_or(false);
    let blocks = json["blocks"].as_u64().unwrap_or(0);
    let head = json["head_event_id"].as_str().unwrap_or("none");
    let errors = json["errors"].as_array().map(|e| e.len()).unwrap_or(0);

    println!("chain:  {chain_id}");
    println!("events: {blocks}");
    println!("head:   {head}");
    println!("valid:  {}", if valid { "OK" } else { "FAIL" });
    if errors > 0 {
        println!("errors: {errors}");
    }
    Ok(())
}

fn cmd_report(proof_path: &str, output_path: &str) -> Result<(), CliError> {
    use notary_pdf::{generate_certificate_pdf, CertificateInput, CertificateStatus};

    let content = fs::read_to_string(proof_path)?;
    let proof_json: serde_json::Value = serde_json::from_str(&content)?;

    let sha256 = proof_json["events"]
        .as_array()
        .and_then(|e| e.first())
        .and_then(|e| e["file_hash"].as_str())
        .ok_or_else(|| CliError::Server("incomplete proof: missing file_hash".into()))?
        .to_string();

    let chain_id_str = proof_json["chain_id"]
        .as_str()
        .ok_or_else(|| CliError::Server("incomplete proof: missing chain_id".into()))?
        .to_string();

    let tsa_obj = proof_json.get("tsa").and_then(|t| t.as_object());

    let tsa_timestamp_raw = tsa_obj.and_then(|t| t["timestamp"].as_i64());
    let tsa_serial_raw = tsa_obj.and_then(|t| t["serial"].as_str());
    let tsa_token_raw = tsa_obj.and_then(|t| t["token_bytes"].as_i64());

    let tsa_complete =
        tsa_timestamp_raw.is_some() && tsa_serial_raw.is_some() && tsa_token_raw.is_some();

    let tsa_timestamp = tsa_timestamp_raw.unwrap_or(0) as u64;
    let tsa_serial = tsa_serial_raw.unwrap_or("").to_string();
    let tsa_token = tsa_token_raw.unwrap_or(0).to_string();

    println!("DEBUG: tsa_timestamp = {}", tsa_timestamp);
    println!("DEBUG: tsa_obj = {:?}", tsa_obj);

    let input = CertificateInput {
        status: CertificateStatus::Valid,
        file_hash_valid: true,
        tsa_valid: tsa_complete,
        proof_id: chain_id_str.clone(),
        sha256,
        object_type: "file".into(),
        created_at_utc: chrono::Utc::now().to_rfc3339(),
        tsa_provider: "FreeTSA".into(),
        tsa_timestamp_utc: CertificateInput::format_timestamp_unix(tsa_timestamp),
        tsa_token_base64: tsa_token,
        verify_url: format!("https://example.com/verify/{}", chain_id_str),
        file_size_kb: 0,
        file_name: proof_path.split('/').last().unwrap_or("proof").to_string(),
    };

    let pdf_bytes = generate_certificate_pdf(&input)
        .map_err(|e| CliError::Server(format!("PDF generation failed: {e}")))?;

    fs::write(Path::new(output_path), pdf_bytes).map_err(|e| CliError::Io(e))?;

    println!("report saved to {output_path}");
    Ok(())
}
