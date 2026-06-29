use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command as ProcessCommand};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug)]
enum CliError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Http(reqwest::Error),
    Server(String),
    Usage(String),
}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        CliError::Io(err)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(err: serde_json::Error) -> Self {
        CliError::Json(err)
    }
}

impl From<reqwest::Error> for CliError {
    fn from(err: reqwest::Error) -> Self {
        CliError::Http(err)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Io(err) => write!(f, "I/O error: {err}"),
            CliError::Json(err) => write!(f, "JSON error: {err}"),
            CliError::Http(err) => write!(f, "HTTP error: {err}"),
            CliError::Server(message) => write!(f, "{message}"),
            CliError::Usage(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CliError {}

#[derive(Debug)]
enum CliCommand {
    Hash { path: String },
    Commit { path: String, chain_id: String },
    Verify { proof_path: String },
}

#[derive(Debug, Deserialize)]
struct CommitResponse {
    event_id: String,
    chain_id: String,
    head_event_id: String,
    sequence: i64,
    cached: bool,
    proof: ProofPayload,
    events: Vec<EventLeaf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofFile {
    chain_id: String,
    head_event_id: String,
    proof: ProofPayload,
    events: Vec<EventLeaf>,
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
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args().skip(1);
    let command = match args.next().as_deref() {
        Some("hash") => {
            let path = args.next().ok_or_else(|| CliError::Usage("usage: evident hash <file>".into()))?;
            CliCommand::Hash { path }
        }
        Some("commit") => {
            let path = args.next().ok_or_else(|| CliError::Usage("usage: evident commit <file> --chain <id>".into()))?;
            let mut chain_id = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--chain" => chain_id = args.next(),
                    _ => return Err(CliError::Usage("unknown argument".into())),
                }
            }
            let chain_id = chain_id.ok_or_else(|| CliError::Usage("missing --chain".into()))?;
            CliCommand::Commit { path, chain_id }
        }
        Some("verify") => {
            let proof_path = args.next().ok_or_else(|| CliError::Usage("usage: evident verify <proof.json>".into()))?;
            CliCommand::Verify { proof_path }
        }
        Some(other) => return Err(CliError::Usage(format!("unknown command: {other}"))),
        None => return Err(CliError::Usage("usage: evident <hash|commit|verify>".into())),
    };

    match command {
        CliCommand::Hash { path } => cmd_hash(&path),
        CliCommand::Commit { path, chain_id } => cmd_commit(&path, &chain_id),
        CliCommand::Verify { proof_path } => cmd_verify(&proof_path),
    }
}

fn cmd_hash(path: &str) -> Result<(), CliError> {
    let bytes = fs::read(path)?;
    let hash = sha256_hex(&bytes);
    println!("{hash}");
    Ok(())
}

fn cmd_commit(path: &str, chain_id: &str) -> Result<(), CliError> {
    let bytes = fs::read(path)?;
    let file_hash = sha256_hex(&bytes);

    let chain_uuid = Uuid::parse_str(chain_id).map_err(|_| CliError::Usage("invalid chain id".into()))?;
    let idempotency_key = Uuid::new_v4().to_string();

    let client = reqwest::blocking::Client::new();
    let response = client
        .post("http://127.0.0.1:3000/events")
        .json(&json!({
            "file_hash": file_hash,
            "chain_id": chain_uuid,
            "idempotency_key": idempotency_key
        }))
        .send()?;

    let status = response.status();
    let body = response.text()?;
    if !status.is_success() {
        return Err(CliError::Server(format!("server error {status}: {body}")));
    }

    let commit: CommitResponse = serde_json::from_str(&body)?;
    let proof_path = PathBuf::from("proofs")
        .join(commit.chain_id.clone())
        .join(format!("{}.json", commit.event_id));

    fs::create_dir_all(proof_path.parent().unwrap())?;
    let proof = ProofFile {
        chain_id: commit.chain_id.clone(),
        head_event_id: commit.head_event_id.clone(),
        proof: commit.proof.clone(),
        events: commit.events.clone(),
    };

    fs::write(&proof_path, serde_json::to_string_pretty(&proof)?)?;
    println!("saved proof to {}", proof_path.display());
    Ok(())
}

fn cmd_verify(proof_path: &str) -> Result<(), CliError> {
    let path = Path::new(proof_path);
    let files: Vec<PathBuf> = if path.is_dir() {
        let mut entries = fs::read_dir(path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|entry| entry.is_file())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    } else {
        vec![path.to_path_buf()]
    };

    for file in files {
        let content = fs::read_to_string(&file)?;
        let proof: ProofFile = serde_json::from_str(&content)?;
        let verifier = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("evident-verify");
        let status = ProcessCommand::new(verifier).arg(&file).status().map_err(|err| CliError::Server(format!("failed to run verifier: {err}")))?;
        if !status.success() {
            return Err(CliError::Server(format!("verification failed for {}", file.display())));
        }
        println!("OK: proof valid");
        println!("  file:       {}", file.display());
        println!("  chain_id:   {}", proof.chain_id);
        println!("  chain_head: {}", proof.head_event_id);
        println!("  root:       {}", proof.proof.root);
        println!("  events:     {}", proof.events.len());
    }

    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
