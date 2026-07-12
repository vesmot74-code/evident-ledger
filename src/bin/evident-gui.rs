use chrono::{TimeZone, Utc};
use eframe::egui;
use evident_ledger::audit::{AuditEvent, AuditStore, ChainAnchorProof};
use evident_ledger::client::{self, EvidentClient};
use evident_report::{
    generate_report, EventSummary, FileStatus, ProofData, TsaData as ReportTsaData,
    VerificationContext,
};
use notary_pdf::{generate_certificate_pdf, CertificateInput, CertificateStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;
use zip::write::{SimpleFileOptions, ZipWriter};

// ============================================================================
// LANGUAGE
// ============================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Lang {
    #[default]
    En,
    Ru,
}

// ============================================================================
// PROJECT MODEL
// ============================================================================
#[derive(Debug, Serialize, Deserialize)]
struct Project {
    name: String,
    chain_id: String,
    created_at: String,
}

impl Project {
    fn path(projects_dir: &PathBuf, name: &str) -> PathBuf {
        projects_dir.join(name)
    }

    fn save(&self, projects_dir: &PathBuf) -> Result<(), std::io::Error> {
        let project_path = Self::path(projects_dir, &self.name);
        fs::create_dir_all(&project_path)?;
        fs::create_dir_all(project_path.join("originals"))?;
        fs::create_dir_all(project_path.join("proofs"))?;
        // NOTE: the on-disk folder name is kept as "Аудит" for backward
        // compatibility with existing projects created before the EN/RU
        // language switch was added. This is a storage path, not UI text.
        fs::create_dir_all(project_path.join("Аудит"))?;
        let path = project_path.join("project.json");
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn list(projects_dir: &PathBuf) -> Vec<String> {
        if let Ok(entries) = fs::read_dir(projects_dir) {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().join("project.json").exists())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        } else {
            vec![]
        }
    }
}

// ============================================================================
// AUDIT MODEL FOR VERIFICATION
// ============================================================================
#[derive(Debug, Clone)]
struct VerificationEvent {
    sequence: i64,
    event_id: String,
    file_name: String,
    timestamp: String,
    valid: bool,
    error: Option<String>,
    error_type: ErrorType,
    local_integrity_ok: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
enum ErrorType {
    None,
    FileHashMismatch,
    ChainBreak,
    SignatureInvalid,
    TsaInvalid,
    TimestampMismatch,
}

impl Default for ErrorType {
    fn default() -> Self {
        Self::None
    }
}

// ============================================================================
// COLORS
// ============================================================================
const COLOR_NAVY: egui::Color32 = egui::Color32::from_rgb(22, 33, 62);
const COLOR_ACCENT: egui::Color32 = egui::Color32::from_rgb(55, 107, 140); // RAL 5007 Brilliant Blue
const COLOR_ACCENT_DARK: egui::Color32 = egui::Color32::from_rgb(41, 84, 111); // RAL 5007, darker for hover/active
const COLOR_VALID: egui::Color32 = egui::Color32::from_rgb(21, 128, 61);
const COLOR_INVALID: egui::Color32 = egui::Color32::from_rgb(185, 28, 28);
const COLOR_PARTIAL: egui::Color32 = egui::Color32::from_rgb(180, 83, 9);
const COLOR_SURFACE: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
const COLOR_BG: egui::Color32 = egui::Color32::from_rgb(245, 247, 251);
const COLOR_BORDER: egui::Color32 = egui::Color32::from_rgb(203, 213, 225);

// ============================================================================
// APP
// ============================================================================
fn file_hash_from_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq)]
enum ChainValidationError {
    InvalidSequence {
        index: usize,
        sequence: i64,
    },
    SequenceBreak {
        index: usize,
        expected: i64,
        actual: i64,
    },
    MissingHeadEventId,
}

#[derive(Default)]
struct AppState {
    head_event_id: Option<String>,
}

struct App {
    // === File ===
    file_path: String,
    file_name: String,
    file_size: u64,
    selected_file_hash: String,
    loading_hash: bool,

    // === Project ===
    projects: Vec<String>,
    project_name: String,
    selected_project: String,
    project_mode: ProjectMode,

    // === Status ===
    status: String,
    step: Step,
    commit_success: bool,
    event_id: String,
    proof_path: String,
    verify_status: VerifyStatus,
    verify_details: String,

    // === Project verification ===
    verification_events: Vec<VerificationEvent>,
    verification_report: String,
    verification_complete: bool,
    verification_project: String,

    // === UI state ===
    screen: Screen,
    error_message: String,
    state: AppState,
    lang: Lang,

    // === Async ===
    _runtime: tokio::runtime::Runtime,
    rt: tokio::runtime::Handle,
    tx_resp: tokio::sync::mpsc::UnboundedSender<WorkerResponse>,
    rx_resp: tokio::sync::mpsc::UnboundedReceiver<WorkerResponse>,
    loading_verify_project: bool,
    loading_verify_chain: bool,
    loading_commit: bool,
    last_proof: Option<client::ProofFile>,
}

impl Default for App {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        let rt_handle = rt.handle().clone();
        Self {
            file_path: Default::default(),
            file_name: Default::default(),
            file_size: Default::default(),
            selected_file_hash: Default::default(),
            loading_hash: false,
            projects: Default::default(),
            project_name: Default::default(),
            selected_project: Default::default(),
            project_mode: Default::default(),
            status: Default::default(),
            step: Default::default(),
            commit_success: Default::default(),
            event_id: Default::default(),
            proof_path: Default::default(),
            verify_status: Default::default(),
            verify_details: Default::default(),
            verification_events: Default::default(),
            verification_report: Default::default(),
            verification_complete: Default::default(),
            verification_project: Default::default(),
            screen: Default::default(),
            error_message: Default::default(),
            state: Default::default(),
            lang: Lang::default(),
            _runtime: rt,
            rt: rt_handle,
            tx_resp: tx,
            rx_resp: rx,
            loading_verify_project: Default::default(),
            loading_verify_chain: false,
            loading_commit: false,
            last_proof: None,
        }
    }
}

#[derive(PartialEq, Default)]
enum ProjectMode {
    #[default]
    New,
    Existing,
}

#[derive(PartialEq, Default)]
enum Step {
    #[default]
    Idle,
    Hashing,
    Committing,
    TsaWaiting,
    Done,
    Failed,
}

#[derive(PartialEq, Default)]
enum VerifyStatus {
    #[default]
    None,
    Valid,
    Invalid,
    Partial,
}

#[derive(PartialEq, Default)]
enum Screen {
    #[default]
    FileSelection,
    HashPreview,
    SelectProject,
    CommitProgress,
    Result,
    VerifyProject,
    VerifyResult,
}

#[derive(Debug)]
struct CommitSuccess {
    commit: client::CommitResponse,
    proof_path: PathBuf,
    file_hash: String,
    project_path: PathBuf,
    proofs_dir: PathBuf,
    chain_uuid: Uuid,
    source_file_path: String,
    file_name: String,
}

#[derive(Debug)]
struct CommitFailure {
    error: String,
    project_path: PathBuf,
    chain_uuid: Uuid,
    file_hash: String,
}

enum WorkerResponse {
    HashComputed(Result<String, String>),

VerifyChainDone(Result<evident_ledger::client::VerifyResponse, String>),
    CommitDone(Result<CommitSuccess, CommitFailure>),
}

impl App {
    /// Returns the Russian or English string depending on the currently
    /// selected UI language.
    fn tr(&self, ru: &'static str, en: &'static str) -> &'static str {
        match self.lang {
            Lang::Ru => ru,
            Lang::En => en,
        }
    }

    /// Formats a byte size using the appropriate localized unit.
    fn format_size(&self, bytes: u64) -> String {
        let size_kb = bytes / 1024;
        let size_mb = bytes / (1024 * 1024);
        if size_mb > 0 {
            format!(
                "{:.2} {}",
                bytes as f64 / (1024.0 * 1024.0),
                self.tr("МБ", "MB")
            )
        } else if size_kb > 0 {
            format!("{} {}", size_kb, self.tr("КБ", "KB"))
        } else {
            format!("{} {}", bytes, self.tr("байт", "bytes"))
        }
    }

    fn check_local_integrity(
        originals_dir: &Path,
        sequence: i64,
        expected_hash: &str,
    ) -> Option<bool> {
        let prefix = format!("{:04}_", sequence);
        let entries = fs::read_dir(originals_dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&prefix) && path.is_file() {
                    let bytes = fs::read(&path).ok()?;
                    let actual_hash = file_hash_from_bytes(&bytes);
                    return Some(actual_hash == expected_hash);
                }
            }
        }
        None
    }

fn find_original_name(originals_dir: &Path, sequence: i64) -> Option<String> {
        let prefix = format!("{:04}_", sequence);
        fs::read_dir(originals_dir)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .map(|e| e.file_name().to_string_lossy().into_owned())
    }

    /// Loads the most complete locally saved proof snapshot for a project,
    /// without touching the network. Every commit writes a full ProofFile
    /// snapshot to proofs/<event_id>.json, so the file with the most
    /// events is the latest known state of the chain.
    fn load_local_proof(proofs_dir: &Path) -> Option<client::ProofFile> {
        let mut best: Option<client::ProofFile> = None;
        let entries = fs::read_dir(proofs_dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let contents = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let proof = match serde_json::from_str::<client::ProofFile>(&contents) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let is_better = best
                .as_ref()
                .map_or(true, |b: &client::ProofFile| proof.events.len() > b.events.len());
            if is_better {
                best = Some(proof);
            }
        }
        best
    }

    /// Builds the event list purely from local data — no server needed.
    /// `valid` here means "locally self-consistent" (hash-chain sequence
    /// intact + file hash matches on disk), NOT a full signature check —
    /// that still needs the server's public key.
    fn build_local_events(proof: &client::ProofFile, originals_dir: &Path) -> Vec<VerificationEvent> {
        let mut sorted = proof.events.clone();
        sorted.sort_by_key(|e| e.sequence);

        let mut events = Vec::with_capacity(sorted.len());
        let mut expected_parent = "00000000-0000-0000-0000-000000000000".to_string();

        for leaf in sorted {
            let chain_ok = leaf.parent_event_id == expected_parent;
            expected_parent = leaf.event_id.clone();

            let local_integrity_ok =
                Self::check_local_integrity(originals_dir, leaf.sequence, &leaf.file_hash);
            let file_name = Self::find_original_name(originals_dir, leaf.sequence)
                .unwrap_or_else(|| format!("event_{:04}", leaf.sequence));

            events.push(VerificationEvent {
                sequence: leaf.sequence,
                event_id: leaf.event_id,
                file_name,
                timestamp: String::new(),
                valid: chain_ok,
                error: if chain_ok {
                    None
                } else {
                    Some("parent_event_id chain break (local)".to_string())
                },
                error_type: if chain_ok {
                    ErrorType::None
                } else {
                    ErrorType::ChainBreak
                },
                local_integrity_ok,
            });
        }
        events
    }

    fn export_event_pdf(
        projects_dir: &Path,
        project_name: &str,
        proof: &client::ProofFile,
        event: &VerificationEvent,
    ) -> Result<PathBuf, String> {
        let project_path = projects_dir.join(project_name);
        let originals_dir = project_path.join("originals");
        let proofs_dir = project_path.join("proofs");

        fs::create_dir_all(&proofs_dir).map_err(|e| format!("Failed to create proofs dir: {e}"))?;

        let event_leaf = proof
            .events
            .iter()
            .find(|e| e.event_id == event.event_id)
            .ok_or_else(|| "Event not found in proof chain".to_string())?;

        let file_name = Self::find_original_name(&originals_dir, event.sequence)
            .unwrap_or_else(|| event.file_name.clone());

        let fresh_local_integrity_ok =
            Self::check_local_integrity(&originals_dir, event.sequence, &event_leaf.file_hash);

        let single_event_summary = EventSummary {
            event_id: event_leaf.event_id.clone(),
            file_hash: event_leaf.file_hash.clone(),
            sequence: Some(event.sequence),
        };

        let tsa_complete =
            proof
                .tsa
                .as_ref()
                .and_then(|t| match (t.timestamp, &t.serial, t.token_bytes) {
                    (Some(ts), Some(serial), Some(tb)) => Some(ReportTsaData {
                        timestamp: ts,
                        serial: serial.clone(),
                        token_bytes: tb as usize,
                    }),
                    _ => None,
                });

        let created_at = proof
            .tsa
            .as_ref()
            .and_then(|t| t.timestamp)
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single());

        let proof_data = ProofData {
            chain_id: proof.chain_id.clone(),
            head_event_id: proof.head_event_id.clone(),
            events: vec![single_event_summary],
            root: proof.proof.root.clone(),
            signature: proof.proof.signature.clone(),
            public_key: proof.proof.public_key.clone(),
            tsa: tsa_complete,
            created_at,
        };

        let verify_valid_now = event.valid && fresh_local_integrity_ok == Some(true);

        let files = vec![FileStatus {
            file_name,
            chain_valid: event.valid,
            local_integrity_ok: fresh_local_integrity_ok,
        }];

        let verification = VerificationContext {
            is_valid: verify_valid_now,
            verified_at: Utc::now(),
            first_failure_sequence: if verify_valid_now {
                None
            } else {
                Some(event.sequence)
            },
            first_failure_error: if verify_valid_now {
                None
            } else {
                Some("Event-level verification failed".to_string())
            },
            files,
        };

        let pdf_path = proofs_dir.join(format!("EVENT_{:03}_attestation.pdf", event.sequence));

        generate_report(&proof_data.chain_id, &proof_data, &verification, &pdf_path)
            .map_err(|e| format!("Failed to generate PDF: {e}"))?;

        Ok(pdf_path)
    }

    fn build_certificate_input(
        proof: &client::ProofFile,
        events: &[VerificationEvent],
        originals_dir: &Path,
        verify_valid: bool,
    ) -> CertificateInput {
        let head_event = events.iter().find(|e| e.event_id == proof.head_event_id);

        let file_name = head_event
            .map(|e| e.file_name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let file_size_kb = fs::metadata(originals_dir.join(&file_name))
            .map(|m| m.len() / 1024)
            .unwrap_or(0);

        let sha256 = proof
            .events
            .iter()
            .find(|e| e.event_id == proof.head_event_id)
            .map(|e| e.file_hash.clone())
            .or_else(|| proof.events.last().map(|e| e.file_hash.clone()))
            .unwrap_or_default();

        let status = if !verify_valid {
            CertificateStatus::InvalidHash
        } else if proof.tsa.is_none() {
            CertificateStatus::MissingTsa
        } else {
            CertificateStatus::Valid
        };

        let tsa_timestamp_utc = match proof.tsa.as_ref().and_then(|t| t.timestamp) {
            Some(ts) => CertificateInput::format_timestamp_unix(ts as u64),
            None => "not confirmed".to_string(),
        };
        let tsa_token = proof
            .tsa
            .as_ref()
            .and_then(|t| t.token_bytes)
            .unwrap_or(0)
            .to_string();

        CertificateInput {
            status,
            file_hash_valid: verify_valid,
            tsa_valid: proof.tsa.is_some(),
            proof_id: proof.chain_id.clone(),
            sha256,
            object_type: "file".into(),
            created_at_utc: Utc::now().to_rfc3339(),
            tsa_provider: "FreeTSA".into(),
            tsa_timestamp_utc,
            tsa_token_base64: tsa_token,
            verify_url: format!("https://example.com/verify/{}", proof.chain_id),
            file_size_kb,
            file_name,
        }
    }

    fn export_chain_zip(
        projects_dir: &Path,
        project_name: &str,
        proof: &client::ProofFile,
        events: &[VerificationEvent],
        verify_valid: bool,
    ) -> Result<PathBuf, String> {
        let project_path = projects_dir.join(project_name);

        let originals_dir = project_path.join("originals");
        let proofs_dir = project_path.join("proofs");

        fs::create_dir_all(&proofs_dir).map_err(|e| e.to_string())?;

        let (proof_data, verification) = Self::build_evidence_snapshot(proof, events, verify_valid);

        let pdf_path = proofs_dir.join("evidence_snapshot.pdf");

        generate_report(&proof_data.chain_id, &proof_data, &verification, &pdf_path)
            .map_err(|e| format!("{:?}", e))?;

        let zip_path = proofs_dir.join(format!(
            "{}_evidence_package.zip",
            project_name.replace(" ", "_")
        ));

        let file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;

        let mut zip = ZipWriter::new(file);

        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for event in events {
            if let Some(name) = Self::find_original_name(&originals_dir, event.sequence) {
                let path = originals_dir.join(&name);

                if let Ok(bytes) = fs::read(&path) {
                    zip.start_file(format!("events/{}", name), options)
                        .map_err(|e| e.to_string())?;

                    zip.write_all(&bytes).map_err(|e| e.to_string())?;
                }
            }
        }

        zip.start_file("evidence_snapshot.pdf", options)
            .map_err(|e| e.to_string())?;

        let pdf = fs::read(&pdf_path).map_err(|e| e.to_string())?;

        zip.write_all(&pdf).map_err(|e| e.to_string())?;

        let manifest = serde_json::json!({
            "chain_id": proof.chain_id,
            "head_event_id": proof.head_event_id,

            "root": proof.proof.root,
            "signature": proof.proof.signature,
            "public_key": proof.proof.public_key,

            "tsa": proof.tsa,

            "verified": verify_valid,

            "events": proof.events.iter().map(|e| {
                serde_json::json!({
                    "sequence": e.sequence,
                    "event_id": e.event_id,
                    "file_hash": e.file_hash
                })
            }).collect::<Vec<_>>(),

            "exported_at":
                chrono::Utc::now()
                .to_rfc3339()
        });

        zip.start_file("chain_manifest.json", options)
            .map_err(|e| e.to_string())?;

        zip.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())
            .map_err(|e| e.to_string())?;

        let readme = format!(
            "Evident Ledger — Full Chain Evidence Export\n\
             ==========================================\n\
             Project: {}\n\
             Chain ID: {}\n\
             Events: {}\n\
             Status: {}\n\
             Exported: {}\n\n\
             Package contents:\n\
             - events/                 Original evidence files\n\
             - evidence_snapshot.pdf   Human-readable verification report\n\
             - chain_manifest.json     Machine-readable proof data\n\
             - README.txt              Package description\n\n\
             Verification:\n\
             Recalculate SHA-256 hashes of files in events/\n\
             and compare them with chain_manifest.json.\n",
            project_name,
            proof.chain_id,
            events.len(),
            if verify_valid {
                "VERIFIED"
            } else {
                "NOT VERIFIED"
            },
            chrono::Utc::now().to_rfc3339()
        );

        zip.start_file("README.txt", options)
            .map_err(|e| e.to_string())?;

        zip.write_all(readme.as_bytes())
            .map_err(|e| e.to_string())?;

        zip.finish().map_err(|e| e.to_string())?;

        Ok(zip_path)
    }

    fn build_evidence_snapshot(
        proof: &client::ProofFile,
        events: &[VerificationEvent],
        verify_valid: bool,
    ) -> (ProofData, VerificationContext) {
        let report_events: Vec<EventSummary> = proof
            .events
            .iter()
            .map(|e| EventSummary {
                event_id: e.event_id.clone(),
                file_hash: e.file_hash.clone(),
                sequence: Some(e.sequence),
            })
            .collect();

        let tsa_complete =
            proof
                .tsa
                .as_ref()
                .and_then(|t| match (t.timestamp, &t.serial, t.token_bytes) {
                    (Some(ts), Some(serial), Some(tb)) => Some(ReportTsaData {
                        timestamp: ts,
                        serial: serial.clone(),
                        token_bytes: tb as usize,
                    }),
                    _ => None,
                });

        let created_at = proof
            .tsa
            .as_ref()
            .and_then(|t| t.timestamp)
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single());

        let proof_data = ProofData {
            chain_id: proof.chain_id.clone(),
            head_event_id: proof.head_event_id.clone(),
            events: report_events,
            root: proof.proof.root.clone(),
            signature: proof.proof.signature.clone(),
            public_key: proof.proof.public_key.clone(),
            tsa: tsa_complete,
            created_at,
        };

        let first_failure = events.iter().find(|e| !e.valid);

        let files: Vec<FileStatus> = events
            .iter()
            .map(|e| FileStatus {
                file_name: e.file_name.clone(),
                chain_valid: e.valid,
                local_integrity_ok: e.local_integrity_ok,
            })
            .collect();

        let verification = VerificationContext {
            is_valid: verify_valid,
            verified_at: Utc::now(),
            first_failure_sequence: first_failure.map(|e| e.sequence),
            first_failure_error: first_failure.and_then(|e| e.error.clone()),
            files,
        };

        (proof_data, verification)
    }

    fn new() -> Self {
        let mut app = Self::default();
        if let Err(err) = app.ensure_projects_dir() {
            app.status = format!("⚠️ {err}");
        }
        app
    }

    fn projects_dir(&self) -> Result<PathBuf, String> {
        let home = std::env::var("HOME").map_err(|_| "HOME is required".to_string())?;
        Ok(PathBuf::from(home).join("Evident Projects"))
    }

    fn ensure_projects_dir(&self) -> Result<(), String> {
        let projects_dir = self.projects_dir()?;
        fs::create_dir_all(&projects_dir).map_err(|e| {
            format!(
                "{}: {e}",
                self.tr(
                    "Не удалось создать каталог проектов",
                    "Failed to create the projects directory"
                )
            )
        })?;
        Ok(())
    }

    fn load_projects(&mut self) {
        match self.projects_dir() {
            Ok(projects_dir) => self.projects = Project::list(&projects_dir),
            Err(err) => {
                self.projects.clear();
                self.status = format!("⚠️ {err}");
            }
        }
    }

    fn ensure_project_layout(&self, project_path: &Path) -> Result<(), String> {
        fs::create_dir_all(project_path.join("originals")).map_err(|e| {
            format!(
                "{} originals: {e}",
                self.tr("Не удалось создать", "Failed to create")
            )
        })?;
        fs::create_dir_all(project_path.join("proofs")).map_err(|e| {
            format!(
                "{} proofs: {e}",
                self.tr("Не удалось создать", "Failed to create")
            )
        })?;
        fs::create_dir_all(project_path.join("Аудит")).map_err(|e| {
            format!(
                "{} {}: {e}",
                self.tr("Не удалось создать", "Failed to create"),
                self.tr("Аудит", "Audit")
            )
        })?;
        Ok(())
    }

    fn persist_original(
        &self,
        project_path: &Path,
        source_path: &Path,
        sequence: i64,
    ) -> Result<String, String> {
        assert!(sequence > 0, "invalid sequence");

        let originals_dir = project_path.join("originals");
        fs::create_dir_all(&originals_dir).map_err(|e| {
            format!(
                "{} originals: {e}",
                self.tr("Не удалось создать", "Failed to create")
            )
        })?;

        let file_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document")
            .to_string();
        let stem = source_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("document")
            .to_string();
        let ext = source_path
            .extension()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();

        let mut candidate_sequence = sequence;
        loop {
            let candidate_name = if ext.is_empty() {
                format!("{:04}_{}", candidate_sequence, file_name)
            } else {
                format!("{:04}_{}.{}", candidate_sequence, stem, ext)
            };
            let candidate_path = originals_dir.join(&candidate_name);
            if !candidate_path.exists() {
                fs::copy(source_path, &candidate_path).map_err(|e| {
                    format!(
                        "{}: {e}",
                        self.tr(
                            "Не удалось сохранить оригинал",
                            "Failed to save the original file"
                        )
                    )
                })?;
                return Ok(candidate_name);
            }
            candidate_sequence += 1;
        }
    }

    fn append_audit_event(&self, project_path: &Path, event: AuditEvent) -> Result<(), String> {
        // The on-disk folder is intentionally kept as "Аудит" for backward
        // compatibility with existing projects.
        let audit_path = project_path.join("Аудит").join("audit.jsonl");
        let store = AuditStore::new(&audit_path);
        store.append(&event).map_err(|e| {
            format!(
                "{}: {e}",
                self.tr(
                    "Не удалось записать аудиторский журнал",
                    "Failed to write the audit log"
                )
            )
        })?;
        Ok(())
    }

    fn validate_chain(
        events: &[VerificationEvent],
        head_event_id: Option<&str>,
    ) -> Result<(), ChainValidationError> {
        if events.is_empty() {
            return Err(ChainValidationError::MissingHeadEventId);
        }

        for (index, event) in events.iter().enumerate() {
            if event.sequence <= 0 {
                return Err(ChainValidationError::InvalidSequence {
                    index,
                    sequence: event.sequence,
                });
            }

            let expected = (index as i64) + 1;
            if event.sequence != expected {
                return Err(ChainValidationError::SequenceBreak {
                    index,
                    expected,
                    actual: event.sequence,
                });
            }
        }

        if head_event_id.is_none() {
            return Err(ChainValidationError::MissingHeadEventId);
        }

        Ok(())
    }

    fn render_chain_error(&mut self, ui: &mut egui::Ui, error: &ChainValidationError) {
        ui.colored_label(
            COLOR_INVALID,
            self.tr("❌ Цепочка событий нарушена", "❌ Event chain is broken"),
        );
        ui.add_space(8.0);
        match error {
            ChainValidationError::InvalidSequence { index, sequence } => {
                ui.label(format!(
                    "{} #{index}: {sequence}",
                    self.tr("Неверный sequence в событии", "Invalid sequence in event")
                ));
            }
            ChainValidationError::SequenceBreak {
                index,
                expected,
                actual,
            } => {
                ui.label(format!(
                    "{} #{index}: {} {expected}, {} {actual}",
                    self.tr("Разрыв цепочки в событии", "Chain break in event"),
                    self.tr("ожидалось", "expected"),
                    self.tr("получено", "got"),
                ));
            }
            ChainValidationError::MissingHeadEventId => {
                ui.label(self.tr(
                    "Не получен head_event_id из backend",
                    "head_event_id was not received from the backend",
                ));
            }
        }
    }

    // ================================================================
    // PROJECT VERIFICATION
    // ================================================================
fn verify_project(&mut self, ctx: &egui::Context) {
        self.verification_events.clear();
        self.verification_complete = false;
        self.verification_report.clear();
        self.verification_project = self.selected_project.clone();

        if self.selected_project.is_empty() {
            self.status = self
                .tr("❌ Выберите проект", "❌ Select a project")
                .to_string();
            return;
        }

        let projects_dir = match self.projects_dir() {
            Ok(dir) => dir,
            Err(err) => {
                self.status = format!("⚠️ {err}");
                return;
            }
        };
        let project_path = projects_dir.join(&self.selected_project);

        let project_json = match fs::read_to_string(project_path.join("project.json")) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!(
                    "{}: {}",
                    self.tr(
                        "❌ Не удалось прочитать проект",
                        "❌ Failed to read the project"
                    ),
                    e
                );
                return;
            }
        };
        let project: Project = match serde_json::from_str(&project_json) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!(
                    "{}: {}",
                    self.tr(
                        "❌ Не удалось разобрать проект",
                        "❌ Failed to parse the project"
                    ),
                    e
                );
                return;
            }
        };
        let chain_id = match Uuid::parse_str(&project.chain_id) {
            Ok(id) => id,
            Err(_) => {
                self.status = self
                    .tr(
                        "❌ Неправильный chain_id в проекте",
                        "❌ Invalid chain_id in the project",
                    )
                    .to_string();
                return;
            }
        };

        let originals_dir = project_path.join("originals");
        let proofs_dir = project_path.join("proofs");

        // === LOCAL-FIRST: build the picture from disk, no network needed ===
        match Self::load_local_proof(&proofs_dir) {
            Some(local_proof) => {
                self.state.head_event_id = Some(local_proof.head_event_id.clone());
                self.verification_events = Self::build_local_events(&local_proof, &originals_dir);
                self.last_proof = Some(local_proof);

                let local_chain_ok = self.verification_events.iter().all(|e| e.valid);
                let local_tampered = self
                    .verification_events
                    .iter()
                    .any(|e| e.local_integrity_ok != Some(true));

                self.verify_status = if !local_chain_ok {
                    VerifyStatus::Invalid
                } else if local_tampered {
                    VerifyStatus::Partial
                } else {
                    VerifyStatus::Valid
                };

                self.status = self
                    .tr(
                        "✅ Локальные данные загружены (офлайн)",
                        "✅ Local data loaded (offline)",
                    )
                    .to_string();
               self.verification_report = self
                    .tr(
                        "Локальная проверка: цепочка событий, файлы на диске и криптографическая подпись.",
                        "Local check: event chain, files on disk, and cryptographic signature.",
                    )
                    .to_string();
            }
            None => {
                self.verify_status = VerifyStatus::None;
                self.status = self
                    .tr(
                        "⚠️ Локальные данные проекта не найдены",
                        "⚠️ No local project data found",
                    )
                    .to_string();
            }
        }

self.verification_complete = true;
        self.screen = Screen::VerifyResult;

        // === Local-only signature verification — no network involved ===
        if let Some(proof) = self.last_proof.as_ref() {
            let pinned_key_path = dirs::home_dir()
                .map(|home| home.join(".evident").join("server_identity.pub"));

            let trusted_public_key = pinned_key_path
                .as_ref()
                .and_then(|p| fs::read_to_string(p).ok())
                .map(|k| k.trim().to_string());

            match trusted_public_key {
                Some(key) => {
                    let sig_valid = evident_ledger::signing::verify_root(
                        &proof.chain_id,
                        &proof.proof.root,
                        &proof.proof.chain_head,
                        &proof.proof.signature,
                        &key,
                    );

                    if !sig_valid {
                        self.verify_status = VerifyStatus::Invalid;
                        self.status = self
                            .tr(
                                "❌ Подпись недействительна",
                                "❌ Signature is invalid",
                            )
                            .to_string();
                    }
                }
                None => {
                    self.status = self
                        .tr(
                            "⚠️ Не найден локальный ключ сервера для проверки подписи",
                            "⚠️ No local server key found to verify the signature",
                        )
                        .to_string();
                }
            }
        }
    }

    // ================================================================
    // COMMIT
    // ================================================================
    fn do_commit(&mut self, ctx: &egui::Context) {
        if self.file_path.is_empty() {
            self.status = self.tr("❌ Выберите файл", "❌ Select a file").to_string();
            return;
        }

        let project_name = if self.project_mode == ProjectMode::New {
            self.project_name.clone()
        } else {
            self.selected_project.clone()
        };

        if project_name.is_empty() {
            self.status = self
                .tr("❌ Укажите название проекта", "❌ Enter a project name")
                .to_string();
            return;
        }

        let projects_dir = match self.projects_dir() {
            Ok(dir) => dir,
            Err(err) => {
                self.status = format!("⚠️ {err}");
                return;
            }
        };
        let project_path = projects_dir.join(&project_name);
        let proofs_dir = project_path.join("proofs");
        let _ = fs::create_dir_all(&proofs_dir);
        if let Err(err) = self.ensure_project_layout(&project_path) {
            self.status = format!("❌ {err}");
            return;
        }

        let chain_id = if self.project_mode == ProjectMode::New {
            let project = Project {
                name: project_name.clone(),
                chain_id: Uuid::new_v4().to_string(),
                created_at: Utc::now().to_rfc3339(),
            };
            if let Err(e) = project.save(&projects_dir) {
                self.status = format!(
                    "{}: {}",
                    self.tr(
                        "❌ Ошибка создания проекта",
                        "❌ Error creating the project"
                    ),
                    e
                );
                return;
            }
            self.selected_project = project_name.clone();
            self.project_mode = ProjectMode::Existing;
            project.chain_id
        } else {
            let project_file = project_path.join("project.json");
            let project_json = match fs::read_to_string(&project_file) {
                Ok(contents) => contents,
                Err(e) => {
                    self.status = format!(
                        "{}: {}",
                        self.tr(
                            "❌ Не удалось прочитать проект",
                            "❌ Failed to read the project"
                        ),
                        e
                    );
                    return;
                }
            };
            let project: Project = match serde_json::from_str(&project_json) {
                Ok(project) => project,
                Err(e) => {
                    self.status = format!(
                        "{}: {}",
                        self.tr(
                            "❌ Не удалось разобрать проект",
                            "❌ Failed to parse the project"
                        ),
                        e
                    );
                    return;
                }
            };
            project.chain_id
        };

        self.step = Step::Committing;
        self.status = self
            .tr("⏳ Отправка на сервер...", "⏳ Sending to server...")
            .to_string();
        self.screen = Screen::CommitProgress;
        self.loading_commit = true;

        let file_bytes = match fs::read(&self.file_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.step = Step::Failed;
                self.status = format!(
                    "{}: {}",
                    self.tr("❌ Ошибка чтения файла", "❌ Error reading the file"),
                    e
                );
                self.loading_commit = false;
                return;
            }
        };

        let chain_uuid = match Uuid::parse_str(&chain_id) {
            Ok(id) => id,
            Err(_) => {
                self.step = Step::Failed;
                self.status = self
                    .tr("❌ Неправильный chain_id", "❌ Invalid chain_id")
                    .to_string();
                self.loading_commit = false;
                return;
            }
        };

        let tx = self.tx_resp.clone();
        let ctx = ctx.clone();
        let lang = self.lang;
        let project_path_clone = project_path.clone();
        let proofs_dir_clone = proofs_dir.clone();
        let source_file_path = self.file_path.clone();
        let file_name = self.file_name.clone();

        self.rt.spawn_blocking(move || {
            let client = EvidentClient::new("http://127.0.0.1:3000");
            match client.submit_event(chain_uuid, &file_bytes) {
                Ok((commit, proof_path, file_hash)) => {
                    let _ = tx.send(WorkerResponse::CommitDone(Ok(CommitSuccess {
                        commit,
                        proof_path,
                        file_hash,
                        project_path: project_path_clone,
                        proofs_dir: proofs_dir_clone,
                        chain_uuid,
                        source_file_path,
                        file_name,
                    })));
                }
                Err(e) => {
                    let file_hash = file_hash_from_bytes(&file_bytes);
                    let _ = tx.send(WorkerResponse::CommitDone(Err(CommitFailure {
                        error: friendly_error(&e, lang),
                        project_path: project_path_clone,
                        chain_uuid,
                        file_hash,
                    })));
                }
            }
            ctx.request_repaint();
        });
    }

    fn do_verify(&mut self, ctx: &egui::Context) {
        if self.selected_project.is_empty() {
            self.verify_status = VerifyStatus::Invalid;
            self.verify_details = self
                .tr("Проект не выбран", "No project selected")
                .to_string();
            return;
        }

        let projects_dir = match self.projects_dir() {
            Ok(dir) => dir,
            Err(err) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = format!("⚠️ {err}");
                return;
            }
        };
        let project_path = projects_dir.join(&self.selected_project);
        let project_file = project_path.join("project.json");
        let project_json = match fs::read_to_string(&project_file) {
            Ok(contents) => contents,
            Err(e) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = format!(
                    "⚠️ {}: {}",
                    self.tr("Не удалось прочитать проект", "Failed to read the project"),
                    e
                );
                return;
            }
        };
        let project: Project = match serde_json::from_str(&project_json) {
            Ok(project) => project,
            Err(e) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = format!(
                    "⚠️ {}: {}",
                    self.tr("Не удалось разобрать проект", "Failed to parse the project"),
                    e
                );
                return;
            }
        };
        let chain_id = match Uuid::parse_str(&project.chain_id) {
            Ok(id) => id,
            Err(_) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = self
                    .tr("⚠️ Неправильный chain_id", "⚠️ Invalid chain_id")
                    .to_string();
                return;
            }
        };

        self.loading_verify_chain = true;
        self.verify_details = self.tr("⏳ Проверка...", "⏳ Verifying...").to_string();

        let tx = self.tx_resp.clone();
        let ctx = ctx.clone();
        let lang = self.lang;
        self.rt.spawn_blocking(move || {
            let client = EvidentClient::new("http://127.0.0.1:3000");
            let result =
                client::verify_chain(&client, chain_id).map_err(|e| friendly_error(&e, lang));
            let _ = tx.send(WorkerResponse::VerifyChainDone(result));
            ctx.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Global button styling: rounded corners, comfortable padding,
        // consistent look across the whole app. Cosmetic only — no new
        // widgets or screens, just restyling what already exists.
{
            let style = ui.style_mut();
            style.spacing.button_padding = egui::vec2(16.0, 10.0);
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);

            let white_text = egui::Stroke::new(1.0, egui::Color32::WHITE);

            style.visuals.widgets.inactive.bg_fill = COLOR_ACCENT;
            style.visuals.widgets.inactive.weak_bg_fill = COLOR_ACCENT;
            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(0.0, COLOR_ACCENT);
            style.visuals.widgets.inactive.fg_stroke = white_text;

            style.visuals.widgets.hovered.bg_fill = COLOR_ACCENT_DARK;
            style.visuals.widgets.hovered.weak_bg_fill = COLOR_ACCENT_DARK;
            style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(0.0, COLOR_ACCENT_DARK);
            style.visuals.widgets.hovered.fg_stroke = white_text;

            style.visuals.widgets.active.bg_fill = COLOR_ACCENT_DARK;
            style.visuals.widgets.active.weak_bg_fill = COLOR_ACCENT_DARK;
            style.visuals.widgets.active.bg_stroke = egui::Stroke::new(0.0, COLOR_ACCENT_DARK);
            style.visuals.widgets.active.fg_stroke = white_text;

            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(15.0, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Body,
                egui::FontId::new(14.5, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Heading,
                egui::FontId::new(22.0, egui::FontFamily::Proportional),
            );
        }

        // --- worker response handling ---
        while let Ok(resp) = self.rx_resp.try_recv() {
            match resp {
                WorkerResponse::HashComputed(result) => match result {
                    Ok(hash) => {
                        self.selected_file_hash = hash.clone();

                        println!("SHA256 READY: {}", hash);
                        self.status = self
                            .tr("✅ SHA-256 рассчитан", "✅ SHA-256 calculated")
                            .to_string();
                    }
                    Err(e) => {
                        self.status = format!("❌ {}", e);
                    }
                },

            WorkerResponse::VerifyChainDone(res) => {
                    self.loading_verify_chain = false;
                    match res {
                        Ok(result) => {
                            if result.valid {
                                self.verify_status = VerifyStatus::Valid;
                                self.verify_details = self
                                    .tr("✅ Доказательство действительно", "✅ Proof is valid")
                                    .to_string();
                            } else {
                                self.verify_status = VerifyStatus::Invalid;
                                self.verify_details = result.errors.join("; ");
                            }
                        }
                        Err(e) => {
                            self.verify_status = VerifyStatus::Partial;
                            self.verify_details = format!(
                                "{}: {}",
                                self.tr("⚠️ Ошибка проверки", "⚠️ Verification error"),
                                e
                            );
                        }
                    }
                }
                WorkerResponse::CommitDone(res) => {
                    self.loading_commit = false;
                    match res {
                        Ok(success) => {
                            let CommitSuccess {
                                commit,
                                proof_path,
                                file_hash,
                                project_path,
                                proofs_dir,
                                chain_uuid,
                                source_file_path,
                                file_name,
                            } = success;

                            self.state.head_event_id = Some(commit.head_event_id.clone());

                            let original_name = match self.persist_original(
                                &project_path,
                                Path::new(&source_file_path),
                                commit.sequence,
                            ) {
                                Ok(name) => Some(name),
                                Err(err) => {
                                    self.step = Step::Failed;
                                    self.status = format!("❌ {err}");
                                    None
                                }
                            };

                            if let Some(original_name) = original_name {
                                self.event_id = commit.event_id.clone();
                                let proof_name = format!("{}.json", self.event_id);
                                let dest_proof = proofs_dir.join(&proof_name);
                                let _ = fs::copy(&proof_path, &dest_proof);
                                self.proof_path = dest_proof.display().to_string();

                                if let Ok(event_id) = Uuid::parse_str(&commit.event_id) {
                                    let audit_event = AuditEvent::submitted(
                                        event_id,
                                        chain_uuid,
                                        file_hash.clone(),
                                        None,
                                        format!("{}:{}", original_name, file_name),
                                    );
                                    let _ = self.append_audit_event(&project_path, audit_event);

                                    let parent_event_id = commit
                                        .events
                                        .iter()
                                        .find(|leaf| leaf.event_id == commit.event_id)
                                        .and_then(|leaf| {
                                            Uuid::parse_str(&leaf.parent_event_id).ok()
                                        });

                                    let proof = commit.tsa.as_ref().map(|_tsa| {
                                        ChainAnchorProof::new(
                                            commit.proof.root.clone(),
                                            commit.proof.signature.clone(),
                                            "evident-ledger".to_string(),
                                        )
                                    });

                                    if let Ok(server_event_id) = Uuid::parse_str(&commit.event_id) {
                                        let anchored_event = AuditEvent::anchored(
                                            Uuid::new_v4(),
                                            chain_uuid,
                                            file_hash,
                                            parent_event_id,
                                            commit.sequence,
                                            server_event_id,
                                            proof,
                                        );
                                        let _ =
                                            self.append_audit_event(&project_path, anchored_event);
                                    }
                                }

                                self.step = Step::Done;
                                self.status = self
                                    .tr("✅ Фиксация завершена", "✅ Commit complete")
                                    .to_string();
                                self.commit_success = true;
                                self.screen = Screen::Result;
                                self.load_projects();
                            }
                        }
                        Err(failure) => {
                            let _ = self.append_audit_event(
                                &failure.project_path,
                                AuditEvent::failed(
                                    Uuid::new_v4(),
                                    failure.chain_uuid,
                                    failure.file_hash,
                                    None,
                                    format!("submit failed: {}", failure.error),
                                ),
                            );
                            self.step = Step::Failed;
                            self.status = format!("❌ {}", failure.error);
                        }
                    }
                }
            }
        }
        // --- worker response handling end ---

        egui::CentralPanel::default().show(ui, |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, COLOR_BG);
            ui.add_space(8.0);

            // === Language toggle (visible on every screen) ===
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.lang == Lang::Ru, "RU").clicked() {
                        self.lang = Lang::Ru;
                    }
                    ui.add_space(4.0);
                    if ui.selectable_label(self.lang == Lang::En, "EN").clicked() {
                        self.lang = Lang::En;
                    }
                });
            });
            ui.add_space(4.0);

if self.screen == Screen::FileSelection {
                ui.heading("Evident Ledger");
                ui.add_space(8.0);

                let main_btn_width = 240.0;

                ui.horizontal(|ui| {
                    let select_clicked = ui
                        .add_sized(
                            [main_btn_width, 32.0],
                            egui::Button::new(self.tr("📄 Выбрать файл", "📄 Select File")),
                        )
                        .clicked();
                    if select_clicked {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.file_path = path.display().to_string();

                            self.file_name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into();

                            self.file_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

                            self.selected_file_hash.clear();
                            self.loading_hash = false;

                            self.screen = Screen::HashPreview;

                            self.status = self.tr("Файл выбран", "File selected").to_string();
                        }
                    }
                    if !self.file_path.is_empty() {
                        ui.label(format!("📁 {}", self.file_name));
                    }
                });

                ui.add_space(8.0);
                if ui
                    .add_sized(
                        [main_btn_width, 32.0],
                        egui::Button::new(self.tr("🔍 Проверить проект", "🔍 Verify Project")),
                    )
                    .clicked()
                {
                    self.screen = Screen::VerifyProject;
                    self.load_projects();
                }

                ui.add_space(8.0);
                if ui
                    .add_sized(
                        [main_btn_width, 32.0],
                        egui::Button::new(self.tr("🚪 Выход", "🚪 Exit")),
                    )
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }

                if !self.file_path.is_empty() {
                    let size_str = self.format_size(self.file_size);
                    ui.label(format!("📊 {}: {}", self.tr("Размер", "Size"), size_str));

                    if !self.selected_file_hash.is_empty() {
                        ui.add_space(6.0);

                        ui.horizontal(|ui| {
                            ui.label("SHA-256:");

                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.selected_file_hash).monospace(),
                                )
                                .selectable(true),
                            );

                            if ui.button("📋").clicked() {
                                ui.ctx().copy_text(self.selected_file_hash.clone());

                                self.status =
                                    self.tr("✅ Хэш скопирован", "✅ Hash copied").to_string();
                            }
                        });
                    } else if !self.file_path.is_empty() {
                        ui.label(self.tr("⏳ Вычисление SHA-256...", "⏳ Computing SHA-256..."));
                    }

                    if self.file_size == 0 {
                        ui.colored_label(
                            COLOR_INVALID,
                            self.tr("⚠️ Файл пустой!", "⚠️ The file is empty!"),
                        );
                    }
                }

                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(&self.status);
                }
                return;
            }

            // ================================
            // HASH PREVIEW
            // ================================
            if self.screen == Screen::HashPreview {
                ui.heading(self.tr("🔐 Отпечаток файла", "🔐 File Fingerprint"));

                ui.add_space(12.0);

                let size_str = self.format_size(self.file_size);

                ui.label(format!(
                    "📁 {}: {} ({})",
                    self.tr("Файл", "File"),
                    self.file_name,
                    size_str
                ));

                ui.add_space(12.0);

                if self.selected_file_hash.is_empty() {
                    if self.loading_hash {
                        ui.spinner();

                        ui.label(self.tr("⏳ Вычисление SHA-256...", "⏳ Computing SHA-256..."));
                    } else {
                        if ui
                            .button(self.tr("🔢 Посчитать хэш", "🔢 Compute Hash"))
                            .clicked()
                        {
                            self.loading_hash = true;

                            let tx = self.tx_resp.clone();
                            let ctx = ui.ctx().clone();

                            let path = PathBuf::from(&self.file_path);

                            self.rt.spawn_blocking(move || {
                                let result = fs::read(&path)
                                    .map(|bytes| file_hash_from_bytes(&bytes))
                                    .map_err(|e| e.to_string());

                                let _ = tx.send(WorkerResponse::HashComputed(result));

                                ctx.request_repaint();
                            });
                        }
                    }
                } else {
                    ui.label("SHA-256:");

                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.selected_file_hash).monospace())
                            .selectable(true),
                    );

                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button(self.tr("📋 Копировать", "📋 Copy")).clicked() {
                            ui.ctx().copy_text(self.selected_file_hash.clone());

                            self.status =
                                self.tr("✅ Хэш скопирован", "✅ Hash copied").to_string();
                        }

                        if ui
                            .button(self.tr("✅ Зафиксировать", "✅ Commit"))
                            .clicked()
                        {
                            self.screen = Screen::SelectProject;

                            self.load_projects();
                        }
                    });
                }

                ui.add_space(12.0);

                if ui.button(self.tr("⬅ Назад", "⬅ Back")).clicked() {
                    self.screen = Screen::FileSelection;

                    self.file_path.clear();
                    self.file_name.clear();

                    self.selected_file_hash.clear();

                    self.loading_hash = false;
                }

                if !self.status.is_empty() {
                    ui.add_space(8.0);

                    ui.label(&self.status);
                }

                return;
            }

            if self.screen == Screen::VerifyProject {
                ui.heading(self.tr("🔍 Проверка проекта", "🔍 Verify Project"));
                ui.add_space(12.0);
                ui.label(self.tr(
                    "Выберите проект для проверки:",
                    "Select a project to verify:",
                ));
                ui.add_space(8.0);

                if self.loading_verify_project {
                    ui.spinner();
                    ui.label(self.tr("Проверка...", "Verifying..."));
                } else if self.projects.is_empty() {
                    ui.label(self.tr("📭 Нет сохранённых проектов.", "📭 No saved projects."));
                } else {
                    let projects = self.projects.clone();
                    let half_width = ui.available_width() * 0.5;
                    for project in projects {
                        let resp = ui.add_sized(
                            [half_width, 32.0],
                            egui::Button::new(&project),
                        );
                        if resp.clicked() {
                            self.selected_project = project.clone();
                            self.verify_project(ui.ctx());
                        }
                    }
                }

                ui.add_space(12.0);
                if ui.button(self.tr("⬅ Назад", "⬅ Back")).clicked() {
                    self.screen = Screen::FileSelection;
                }
                return;
            }

            if self.screen == Screen::VerifyResult {
                ui.heading("Evident Ledger");
                ui.label(
                    egui::RichText::new(self.tr(
                        "Панель проверки доказательств",
                        "Evidence Verification Dashboard",
                    ))
                    .weak(),
                );
                ui.add_space(12.0);

                egui::Frame::group(ui.style())
                    .fill(COLOR_SURFACE)
                    .stroke(egui::Stroke::new(1.0, COLOR_BORDER))
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(self.tr("Проект", "Project")).weak());
                        });
                        ui.label(
                            egui::RichText::new(&self.verification_project)
                                .size(16.0)
                                .strong(),
                        );
                        ui.add_space(10.0);

                   let (status_color, status_text) = match self.verify_status {
                            VerifyStatus::Valid => {
                                (COLOR_VALID, self.tr("ПОДТВЕРЖДЕНО", "VERIFIED"))
                            }
                            VerifyStatus::Invalid => {
                                (COLOR_INVALID, self.tr("НЕДЕЙСТВИТЕЛЬНО", "INVALID"))
                            }
                            VerifyStatus::Partial => {
                                (COLOR_PARTIAL, self.tr("ЧАСТИЧНО", "PARTIAL"))
                            }
                            VerifyStatus::None => {
                                (COLOR_BORDER, self.tr("НЕ ПРОВЕРЕНО", "NOT VERIFIED"))
                            }
                        };
                        ui.colored_label(
                            status_color,
                            egui::RichText::new(status_text).size(20.0).strong(),
                        );

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);

                        ui.columns(2, |cols| {
                            let events_count = if let Some(proof) = self.last_proof.as_ref() {
                                proof.events.len()
                            } else {
                                self.verification_events.len()
                            };
                            cols[0].label(egui::RichText::new(self.tr("События", "Events")).weak());
                            cols[0].label(
                                egui::RichText::new(events_count.to_string())
                                    .size(16.0)
                                    .strong(),
                            );

                            let tsa_present = self
                                .last_proof
                                .as_ref()
                                .map(|p| p.tsa.is_some())
                                .unwrap_or(false);
                            let (tsa_color, tsa_text) = if tsa_present {
                                (COLOR_VALID, self.tr("Присутствует", "Present"))
                            } else {
                                (COLOR_PARTIAL, self.tr("Не закреплено", "Not Anchored"))
                            };
                            cols[1].label(
                                egui::RichText::new(
                                    self.tr(
                                        "Внешняя метка времени (TSA)",
                                        "External Timestamp (TSA)",
                                    ),
                                )
                                .weak(),
                            );
                            cols[1].colored_label(
                                tsa_color,
                                egui::RichText::new(tsa_text).size(16.0).strong(),
                            );
                        });
                    });

                ui.add_space(12.0);
                ui.label(&self.verification_report);
                ui.add_space(12.0);

                ui.label(self.tr("Цепочка событий:", "Event chain:"));
                ui.add_space(8.0);

                let projects_dir = match self.projects_dir() {
                    Ok(dir) => dir,
                    Err(err) => {
                        ui.colored_label(COLOR_INVALID, format!("⚠️ {err}"));
                        return;
                    }
                };

                {
                    match Self::validate_chain(
                        &self.verification_events,
                        self.state.head_event_id.as_deref(),
                    ) {
                        Ok(()) => {
                            for event in self.verification_events.clone() {
                                ui.horizontal(|ui| {
                                    let label = if event.valid {
                                        format!(
                                            "✅ EVENT {:03}    {}   {}",
                                            event.sequence,
                                            event.file_name,
                                            local_marker(event.local_integrity_ok, self.lang)
                                        )
                                    } else {
                                        format!(
                                            "❌ EVENT {:03}    {}   {}   ⚠️ {}",
                                            event.sequence,
                                            event.file_name,
                                            local_marker(event.local_integrity_ok, self.lang),
                                            event
                                                .error
                                                .as_deref()
                                                .unwrap_or(self.tr("ошибка", "error"))
                                        )
                                    };
                                    if event.valid {
                                        ui.label(label);
                                    } else {
                                        ui.colored_label(COLOR_INVALID, label);
                                    }

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                     ui.add_enabled(
                                                false,
                                                egui::Button::new(format!(
                                                    "📦 ZIP ({})",
                                                    self.tr("скоро", "soon")
                                                ))
                                                .min_size(egui::vec2(140.0, 32.0)),
                                            )
                                            .on_disabled_hover_text(self.tr(
                                                "Экспорт в ZIP появится в следующей версии",
                                                "ZIP export will be available in a future version",
                                            ));

                                            let pdf_clicked = ui
                                                .add_sized(
                                                    [110.0, 32.0],
                                                    egui::Button::new("📄 PDF"),
                                                )
                                                .clicked();

                                            if pdf_clicked {
                                                match self.last_proof.as_ref() {
                                                    Some(proof) => {
                                                        match Self::export_event_pdf(
                                                            &projects_dir,
                                                            &self.verification_project,
                                                            proof,
                                                            &event,
                                                        ) {
                                                            Ok(pdf_path) => {
                                                                let _ =
                                                                    std::process::Command::new(
                                                                        "open",
                                                                    )
                                                                    .arg(&pdf_path)
                                                                    .spawn();

                                                                self.status = format!(
                                                                    "{}: {}",
                                                                    self.tr(
                                                                        "✅ PDF создан",
                                                                        "✅ PDF created"
                                                                    ),
                                                                    pdf_path.display()
                                                                );
                                                            }
                                                            Err(e) => {
                                                                self.status = format!(
                                                                    "{}: {}",
                                                                    self.tr(
                                                                        "❌ Ошибка генерации PDF",
                                                                        "❌ PDF generation error"
                                                                    ),
                                                                    e
                                                                );
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        self.status = self
                                                            .tr(
                                                                "❌ Нет данных — сначала выполните проверку",
                                                                "❌ No data — run verification first",
                                                            )
                                                            .to_string();
                                                    }
                                                }
                                            }
                                        },
                                    );
                                });

                                if !event.valid && event.error_type != ErrorType::None {
                                    ui.label(format!(
                                        "   └─ {}: {:?}",
                                        self.tr("Тип", "Type"),
                                        event.error_type
                                    ));
                                }
                            }
                        }
                        Err(error) => {
                            self.render_chain_error(ui, &error);
                        }
                    }
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(self.tr("📄 Скачать заключение (PDF)", "📄 Download Report (PDF)"))
                        .clicked()
                    {
                        let fresh_proof = match &self.last_proof {
                            Some(cached_proof) => Uuid::parse_str(&cached_proof.chain_id)
                                .ok()
                                .and_then(|chain_uuid| {
                                    let client = EvidentClient::new("http://127.0.0.1:3000");
                                    client::fetch_proof(&client, chain_uuid).ok()
                                }),
                            None => None,
                        };

                        match fresh_proof.as_ref().or(self.last_proof.as_ref()) {
                            Some(proof) => {
                                let proofs_dir =
                                    projects_dir.join(&self.verification_project).join("proofs");
                                let _ = fs::create_dir_all(&proofs_dir);

                                let verify_valid = self.verify_status == VerifyStatus::Valid;
                                let (proof_data, verification) = Self::build_evidence_snapshot(
                                    proof,
                                    &self.verification_events,
                                    verify_valid,
                                );
                                let pdf_path = proofs_dir.join("evidence_snapshot.pdf");

                                match generate_report(
                                    &proof_data.chain_id.clone(),
                                    &proof_data,
                                    &verification,
                                    &pdf_path,
                                ) {
                                    Ok(()) => {
                                        let _ = std::process::Command::new("open")
                                            .arg(&pdf_path)
                                            .spawn();
                                        self.status = format!(
                                            "{}\n{}: {}",
                                            self.tr(
                                                "✅ Заключение сформировано и открыто",
                                                "✅ Report generated and opened"
                                            ),
                                            self.tr("Папка", "Folder"),
                                            pdf_path
                                                .parent()
                                                .map(|p| p.display().to_string())
                                                .unwrap_or_default()
                                        );
                                    }
                                    Err(e) => {
                                        self.status = format!(
                                            "{}: {}",
                                            self.tr(
                                                "❌ Ошибка генерации PDF",
                                                "❌ PDF generation error"
                                            ),
                                            e
                                        );
                                    }
                                }
                            }
                            None => {
                                self.status = self
                                    .tr(
                                        "❌ Нет данных для заключения — сначала выполните проверку",
                                        "❌ No data for a report — run verification first",
                                    )
                                    .to_string();
                            }
                        }
                    }
                    if ui
                        .button(self.tr("📦 Скачать проект (ZIP)", "📦 Download Project (ZIP)"))
                        .clicked()
                    {
                        match &self.last_proof {
                            Some(proof) => {
                                let verify_valid = self.verify_status == VerifyStatus::Valid;

                                match Self::export_chain_zip(
                                    &projects_dir,
                                    &self.verification_project,
                                    proof,
                                    &self.verification_events,
                                    verify_valid,
                                ) {
                                    Ok(path) => {
                                        let _ = Command::new("open")
                                            .arg(path.parent().unwrap())
                                            .spawn();

                                        self.status = format!("✅ ZIP created: {}", path.display());
                                    }

                                    Err(e) => {
                                        self.status = format!("❌ {}", e);
                                    }
                                }
                            }

                            None => {
                                self.status = "❌ Run verification first".to_string();
                            }
                        }
                    }
                });

                ui.add_space(8.0);
                if ui.button(self.tr("⬅ Назад", "⬅ Back")).clicked() {
                    self.screen = Screen::FileSelection;
                    self.verification_complete = false;
                }

                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(&self.status);
                }
                return;
            }

            if self.screen == Screen::SelectProject {
                ui.heading(self.tr(
                    "Куда сохранить доказательство?",
                    "Where should the proof be saved?",
                ));
                ui.add_space(8.0);

                let size_str = self.format_size(self.file_size);
                ui.label(format!(
                    "📁 {}: {} ({})",
                    self.tr("Файл", "File"),
                    self.file_name,
                    size_str
                ));
                if self.file_size == 0 {
                    ui.colored_label(
                        COLOR_INVALID,
                        format!(" {}", self.tr("⚠️ Файл пустой!", "⚠️ The file is empty!")),
                    );
                }

ui.add_space(12.0);
                ui.scope(|ui| {
                    let dark_text = egui::Stroke::new(1.0, egui::Color32::from_rgb(15, 23, 42));
                    let style = ui.style_mut();
                    style.visuals.widgets.inactive.fg_stroke = dark_text;
                    style.visuals.widgets.hovered.fg_stroke = dark_text;
                    style.visuals.widgets.active.fg_stroke = dark_text;

                    ui.horizontal(|ui| {
                        if ui
                            .radio(
                                self.project_mode == ProjectMode::New,
                                self.tr("🆕 Создать новый проект", "🆕 Create New Project"),
                            )
                            .clicked()
                        {
                            self.project_mode = ProjectMode::New;
                        }
                        if ui
                            .radio(
                                self.project_mode == ProjectMode::Existing,
                                self.tr("📂 Использовать существующий", "📂 Use Existing"),
                            )
                            .clicked()
                        {
                            self.project_mode = ProjectMode::Existing;
                            self.load_projects();
                        }
                    });
                });

                ui.add_space(12.0);

                if self.project_mode == ProjectMode::New {
                    ui.label(self.tr("Название проекта:", "Project name:"));
                    ui.text_edit_singleline(&mut self.project_name);
                    ui.add_space(8.0);
                    let projects_dir = match self.projects_dir() {
                        Ok(dir) => dir,
                        Err(err) => {
                            ui.colored_label(COLOR_INVALID, format!("⚠️ {err}"));
                            return;
                        }
                    };
                    ui.label(format!(
                        "📁 {}: {}/{}",
                        self.tr("Папка", "Folder"),
                        projects_dir.display(),
                        self.project_name
                    ));
                } else {
                    if self.projects.is_empty() {
                        ui.label(self.tr(
                            "📭 Нет сохранённых проектов. Создайте новый.",
                            "📭 No saved projects. Create a new one.",
                        ));
                 } else {
                        ui.label(self.tr("Выберите проект:", "Select a project:"));
                        let half_width = ui.available_width() * 0.5;
                        for project in &self.projects {
                            let is_selected = self.selected_project == *project;
                            let button_text = if is_selected {
                                format!("✅ {}", project)
                            } else {
                                project.clone()
                            };
                            let resp = ui.add_sized(
                                [half_width, 32.0],
                                egui::Button::new(button_text),
                            );
                            if resp.clicked() {
                                self.selected_project = project.clone();
                            }
                        }
                        if !self.selected_project.is_empty() {
                            ui.colored_label(
                                COLOR_VALID,
                                format!(
                                    "📂 {}: {}",
                                    self.tr("Выбран проект", "Selected project"),
                                    self.selected_project
                                ),
                            );
                        }
                    }
                }

                ui.add_space(12.0);
                ui.label(self.tr(
                    "📁 Копия файла будет сохранена в папку проекта.",
                    "📁 A copy of the file will be saved in the project folder.",
                ));
                ui.label(self.tr(
                    "📋 Аудит будет записываться в папку Аудит/",
                    "📋 An audit trail will be recorded in the Audit/ folder",
                ));
                ui.add_space(8.0);
                ui.label(self.tr(
                    "Оригинальный файл никогда не изменяется.",
                    "The original file is never modified.",
                ));

                ui.add_space(12.0);

                let can_commit = !self.file_path.is_empty()
                    && ((self.project_mode == ProjectMode::New && !self.project_name.is_empty())
                        || (self.project_mode == ProjectMode::Existing
                            && !self.selected_project.is_empty()));

                if self.loading_commit {
                    ui.spinner();
                    ui.label(self.tr("Отправка...", "Sending..."));
                } else if ui
                    .add_enabled(
                        can_commit,
                        egui::Button::new(self.tr("✅ Зафиксировать", "✅ Commit")),
                    )
                    .clicked()
                {
                    self.do_commit(ui.ctx());
                }

                if ui
                    .button(self.tr("⬅ Назад к файлу", "⬅ Back to File"))
                    .clicked()
                {
                    self.screen = Screen::FileSelection;
                    self.file_path.clear();
                    self.file_name.clear();
                    self.selected_file_hash.clear();
                }

                if !self.error_message.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(COLOR_INVALID, &self.error_message);
                }
                return;
            }

            if self.screen == Screen::CommitProgress {
                ui.heading(self.tr("Фиксация документа", "Committing Document"));
                ui.add_space(12.0);

                ui.label(format!(
                    "📁 {}: {}",
                    self.tr("Файл", "File"),
                    self.file_name
                ));
                ui.label(format!(
                    "📁 {}: {}",
                    self.tr("Проект", "Project"),
                    if self.project_mode == ProjectMode::New {
                        &self.project_name
                    } else {
                        &self.selected_project
                    }
                ));
                ui.add_space(12.0);

                match self.step {
                    Step::Hashing => {
                        let _ = ui
                            .label(self.tr("⏳ Вычисление SHA-256...", "⏳ Computing SHA-256..."));
                    }
                    Step::Committing => {
                        let _ = ui
                            .label(self.tr("⏳ Отправка на сервер...", "⏳ Sending to server..."));
                    }
                    Step::TsaWaiting => {
                        let _ = ui.label(self.tr("⏳ Получение TSA...", "⏳ Retrieving TSA..."));
                    }
                    Step::Done => {
                        let _ = ui.label(self.tr("✅ Фиксация завершена", "✅ Commit complete"));
                    }
                    Step::Failed => {
                        let _ = ui.label(self.tr("❌ Ошибка", "❌ Error"));
                    }
                    _ => {}
                }

                ui.add_space(8.0);
                ui.label(&self.status);

                if self.step == Step::Done {
                    ui.add_space(12.0);
                    if ui
                        .button(self.tr("📋 Посмотреть результат", "📋 View Result"))
                        .clicked()
                    {
                        self.screen = Screen::Result;
                    }
                }

                if self.step == Step::Failed {
                    ui.add_space(12.0);
                    if ui.button(self.tr("⬅ Назад", "⬅ Back")).clicked() {
                        self.screen = Screen::SelectProject;
                        self.step = Step::Idle;
                    }
                }
                return;
            }

            if self.screen == Screen::Result {
                ui.heading(self.tr(
                    "✅ Документ успешно зафиксирован",
                    "✅ Document successfully committed",
                ));
                ui.add_space(8.0);

                ui.label(format!(
                    "📁 {}: {}",
                    self.tr("Проект", "Project"),
                    if self.project_mode == ProjectMode::New {
                        &self.project_name
                    } else {
                        &self.selected_project
                    }
                ));
                ui.label(format!(
                    "📄 {}: {}",
                    self.tr("Событие", "Event"),
                    self.event_id
                ));
                ui.label(format!(
                    "📄 {}: {}",
                    self.tr("Доказательство", "Proof"),
                    self.proof_path
                ));

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if self.loading_verify_chain {
                        ui.spinner();
                        ui.label(self.tr("Проверка...", "Verifying..."));
                    } else if ui.button(self.tr("🔍 Проверить", "🔍 Verify")).clicked()
                    {
                        self.do_verify(ui.ctx());
                    }
                    if !self.proof_path.is_empty() {
                        if ui
                            .button(self.tr("📄 Открыть PDF", "📄 Open PDF"))
                            .clicked()
                        {
                            let projects_dir = match self.projects_dir() {
                                Ok(dir) => dir,
                                Err(err) => {
                                    self.status = format!("⚠️ {err}");
                                    return;
                                }
                            };
                            let project_name = if self.project_mode == ProjectMode::New {
                                &self.project_name
                            } else {
                                &self.selected_project
                            };
                            let pdf_path = projects_dir
                                .join(project_name)
                                .join("proofs")
                                .join("proof.pdf");
                            let _ = Command::new("open").arg(&pdf_path).output();
                        }
                    }
                    if ui
                        .button(self.tr("📋 Вся цепочка аудита", "📋 Full Audit Chain"))
                        .clicked()
                    {
                        self.screen = Screen::VerifyProject;
                        self.selected_project = if self.project_mode == ProjectMode::New {
                            self.project_name.clone()
                        } else {
                            self.selected_project.clone()
                        };
                        self.verify_project(ui.ctx());
                    }
                });

                match self.verify_status {
                    VerifyStatus::Valid => {
                        ui.colored_label(COLOR_VALID, self.tr("✅ ДЕЙСТВИТЕЛЬНО", "✅ VALID"));
                        ui.colored_label(COLOR_VALID, &self.verify_details);
                    }
                    VerifyStatus::Invalid => {
                        ui.colored_label(
                            COLOR_INVALID,
                            self.tr("❌ НЕДЕЙСТВИТЕЛЬНО", "❌ INVALID"),
                        );
                        ui.colored_label(COLOR_INVALID, &self.verify_details);
                    }
                    VerifyStatus::Partial => {
                        ui.colored_label(COLOR_PARTIAL, self.tr("⚠️ ЧАСТИЧНО", "⚠️ PARTIAL"));
                        ui.colored_label(COLOR_PARTIAL, &self.verify_details);
                    }
                    _ => {}
                }

                ui.add_space(12.0);
                if ui
                    .button(self.tr("📁 К выбору файла", "📁 Back to File Selection"))
                    .clicked()
                {
                    self.screen = Screen::FileSelection;
                    self.commit_success = false;
                    self.file_path.clear();
                    self.file_name.clear();
                    self.selected_file_hash.clear();
                    self.event_id.clear();
                    self.proof_path.clear();
                    self.verify_status = VerifyStatus::None;
                    self.step = Step::Idle;
                    self.project_name.clear();
                    self.selected_project.clear();
                }

                return;
            }
        });
    }
}

fn local_marker(status: Option<bool>, lang: Lang) -> &'static str {
    match (status, lang) {
        (Some(true), Lang::En) => "✅ Local",
        (Some(true), Lang::Ru) => "✅ Локально",
        (Some(false), Lang::En) => "❌ Local MODIFIED",
        (Some(false), Lang::Ru) => "❌ Локально ИЗМЕНЕНО",
        (None, Lang::En) => "⚠️ Local N/A",
        (None, Lang::Ru) => "⚠️ Локально Н/Д",
    }
}

fn friendly_error(e: &impl std::fmt::Display, lang: Lang) -> String {
    let msg = e.to_string();
    let unreachable = msg.contains("connection refused")
        || msg.contains("connect")
        || msg.contains("error sending request")
        || msg.contains("dns")
        || msg.contains("failed to lookup");
    let timed_out = msg.contains("timed out") || msg.contains("timeout");

    match lang {
        Lang::En => {
            if unreachable {
                "Server unavailable. Make sure Evident Ledger is running, then try again.".into()
            } else if timed_out {
                "The server is taking too long to respond. Please try again.".into()
            } else {
                format!("An error occurred: {msg}")
            }
        }
        Lang::Ru => {
            if unreachable {
                "Сервер недоступен. Проверьте, что программа Evident Ledger запущена, и повторите попытку.".into()
            } else if timed_out {
                "Сервер не отвечает слишком долго. Попробуйте ещё раз.".into()
            } else {
                format!("Произошла ошибка: {msg}")
            }
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let mut app = App::new();
    if let Err(err) = app.ensure_projects_dir() {
        app.status = format!("⚠️ {err}");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([750.0, 650.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("Evident Ledger", options, Box::new(|_cc| Ok(Box::new(app))))
}
