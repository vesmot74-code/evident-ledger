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
            println!("usage: evident <init|new-chain|commit|verify|status|report generate>");
            Ok(())
        }
        Some("new-chain") => cmd_new_chain(),
        Some("report") => {
            let subcommand = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident report generate <chain_id>".into()))?;
            if subcommand != "generate" {
                return Err(CliError::Usage("usage: evident report generate <chain_id>".into()));
            }
            let chain_id = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident report generate <chain_id>".into()))?;
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
            let path = args
                .next()
                .ok_or_else(|| CliError::Usage("usage: evident commit <file> --chain <id>".into()))?;
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
        _ => Err(CliError::Usage(
            "usage: evident <init|new-chain|commit|verify|status|report generate>".into(),
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
    let chain_uuid = Uuid::parse_str(chain_id)
        .map_err(|_| CliError::Usage("invalid chain id".into()))?;

    let client = evident_ledger::client::EvidentClient::new("http://127.0.0.1:3000");
    let (commit, proof_path, file_hash) = client
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

    println!("anchored    event={}", commit.event_id);
    println!("proof       {}", proof_path.display());
    Ok(())
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

    paths.pop()
        .ok_or_else(|| CliError::Usage(format!("no proof found for chain {chain_id}")))
}

fn cmd_report_generate(chain_id: &str) -> Result<(), CliError> {
    let source_path = find_latest_proof_artifact(chain_id)?;
    let output_path = evident_dir()
        .join("proofs")
        .join(chain_id)
        .join("proof.pdf");
    cmd_report(&source_path.to_string_lossy(), &output_path.to_string_lossy())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn cmd_new_chain() -> Result<(), CliError> {
    let client = reqwest::blocking::Client::new();
    let response = client.post("http://127.0.0.1:3000/chains").send()?;
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
        .unwrap_or("")
        .to_string();

    let tsa_obj = proof_json.get("tsa").and_then(|t| t.as_object());
    let tsa_timestamp = tsa_obj
        .and_then(|t| t["timestamp"].as_i64())
        .unwrap_or(0) as u64;
    let tsa_serial = tsa_obj
        .and_then(|t| t["serial"].as_str())
        .unwrap_or("")
        .to_string();
    let tsa_token = tsa_obj
        .and_then(|t| t["token_bytes"].as_i64())
        .unwrap_or(0)
        .to_string();

    println!("DEBUG: tsa_timestamp = {}", tsa_timestamp);
    println!("DEBUG: tsa_obj = {:?}", tsa_obj);

    let input = CertificateInput {
        status: CertificateStatus::Valid,
        file_hash_valid: true,
        tsa_valid: tsa_obj.is_some(),
        proof_id: proof_json["chain_id"].as_str().unwrap_or("").to_string(),
        sha256,
        object_type: "file".into(),
        created_at_utc: chrono::Utc::now().to_rfc3339(),
        tsa_provider: "FreeTSA".into(),
        tsa_timestamp_utc: CertificateInput::format_timestamp_unix(tsa_timestamp),
        tsa_token_base64: tsa_token,
        verify_url: format!(
            "https://example.com/verify/{}",
            proof_json["chain_id"].as_str().unwrap_or("")
        ),
        file_size_kb: 0,
        file_name: proof_path.split('/').last().unwrap_or("proof").to_string(),
    };

    let pdf_bytes = generate_certificate_pdf(&input)
        .map_err(|e| CliError::Server(format!("PDF generation failed: {e}")))?;

    fs::write(Path::new(output_path), pdf_bytes).map_err(|e| CliError::Io(e))?;

    println!("report saved to {output_path}");
    Ok(())
}
