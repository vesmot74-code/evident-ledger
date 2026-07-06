use eframe::egui;
use std::process::Command;
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use chrono::Utc;
use evident_ledger::audit::{AuditEvent, AuditStore, TsaAttestation};
use sha2::{Digest, Sha256};
use evident_ledger::client::{self, EvidentClient};
use uuid::Uuid;
use notary_pdf::{generate_certificate_pdf, CertificateInput, CertificateStatus};

// ============================================================================
// МОДЕЛЬ ПРОЕКТА
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
// МОДЕЛЬ АУДИТА ДЛЯ ВЕРИФИКАЦИИ
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
// ЦВЕТА
// ============================================================================
const COLOR_NAVY: egui::Color32 = egui::Color32::from_rgb(11, 31, 58);
const COLOR_VALID: egui::Color32 = egui::Color32::from_rgb(21, 128, 61);
const COLOR_INVALID: egui::Color32 = egui::Color32::from_rgb(185, 28, 28);
const COLOR_PARTIAL: egui::Color32 = egui::Color32::from_rgb(180, 83, 9);
const COLOR_SURFACE: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
const COLOR_BG: egui::Color32 = egui::Color32::from_rgb(241, 245, 249);
const COLOR_BORDER: egui::Color32 = egui::Color32::from_rgb(203, 213, 225);

// ============================================================================
// ПРИЛОЖЕНИЕ
// ============================================================================
fn file_hash_from_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq)]
enum ChainValidationError {
    InvalidSequence { index: usize, sequence: i64 },
    SequenceBreak { index: usize, expected: i64, actual: i64 },
    MissingHeadEventId,
}

#[derive(Default)]
struct AppState {
    head_event_id: Option<String>,
}

struct App {
    // === Файл ===
    file_path: String,
    file_name: String,
    file_size: u64,

    // === Проект ===
    projects: Vec<String>,
    project_name: String,
    selected_project: String,
    project_mode: ProjectMode,

    // === Статус ===
    status: String,
    step: Step,
    commit_success: bool,
    event_id: String,
    proof_path: String,
    verify_status: VerifyStatus,
    verify_details: String,

    // === Верификация проекта ===
    verification_events: Vec<VerificationEvent>,
    verification_report: String,
    verification_complete: bool,
    verification_project: String,

    // === UI состояние ===
    screen: Screen,
    error_message: String,
    state: AppState,

    // === Async ===
    _runtime: tokio::runtime::Runtime,
    rt: tokio::runtime::Handle,
    tx_resp: tokio::sync::mpsc::UnboundedSender<WorkerResponse>,
    rx_resp: tokio::sync::mpsc::UnboundedReceiver<WorkerResponse>,
    loading_verify_project: bool,
    loading_verify_chain: bool,
    network_error: bool,
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
            _runtime: rt,
            rt: rt_handle,
            tx_resp: tx,
            rx_resp: rx,
            loading_verify_project: Default::default(),
            loading_verify_chain: false,
            network_error: false,
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
    SelectProject,
    CommitProgress,
    Result,
    VerifyProject,  // новый экран
    VerifyResult,   // новый экран
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
    VerifyProjectDone(Result<(evident_ledger::client::VerifyResponse, evident_ledger::client::ProofFile, PathBuf), String>),
    VerifyChainDone(Result<evident_ledger::client::VerifyResponse, String>),
    CommitDone(Result<CommitSuccess, CommitFailure>),
}

impl App {

    fn check_local_integrity(originals_dir: &Path, sequence: i64, expected_hash: &str) -> Option<bool> {
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
    fs::read_dir(originals_dir).ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.file_name().to_string_lossy().into_owned())
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

    let sha256 = proof.events.iter()
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
        None => "не подтверждено".to_string(),
    };
    let tsa_token = proof.tsa.as_ref().and_then(|t| t.token_bytes).unwrap_or(0).to_string();

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
        fs::create_dir_all(&projects_dir)
            .map_err(|e| format!("Не удалось создать каталог проектов: {e}"))?;
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
        fs::create_dir_all(project_path.join("originals"))
            .map_err(|e| format!("Не удалось создать originals: {e}"))?;
        fs::create_dir_all(project_path.join("proofs"))
            .map_err(|e| format!("Не удалось создать proofs: {e}"))?;
        fs::create_dir_all(project_path.join("Аудит"))
            .map_err(|e| format!("Не удалось создать Аудит: {e}"))?;
        Ok(())
    }

    fn persist_original(&self, project_path: &Path, source_path: &Path, sequence: i64) -> Result<String, String> {
        assert!(sequence > 0, "invalid sequence");

        let originals_dir = project_path.join("originals");
        fs::create_dir_all(&originals_dir)
            .map_err(|e| format!("Не удалось создать originals: {e}"))?;

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
                fs::copy(source_path, &candidate_path)
                    .map_err(|e| format!("Не удалось сохранить оригинал: {e}"))?;
                return Ok(candidate_name);
            }
            candidate_sequence += 1;
        }
    }

    fn append_audit_event(&self, project_path: &Path, event: AuditEvent) -> Result<(), String> {
        let audit_path = project_path.join("Аудит").join("audit.jsonl");
        let store = AuditStore::new(&audit_path);
        store.append(&event).map_err(|e| format!("Не удалось записать аудиторский журнал: {e}"))?;
        Ok(())
    }

    fn validate_chain(events: &[VerificationEvent], head_event_id: Option<&str>) -> Result<(), ChainValidationError> {
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
        ui.colored_label(COLOR_INVALID, "❌ Цепочка событий нарушена");
        ui.add_space(8.0);
        match error {
            ChainValidationError::InvalidSequence { index, sequence } => {
                ui.label(format!("Неверный sequence в событии #{index}: {sequence}"));
            }
            ChainValidationError::SequenceBreak { index, expected, actual } => {
                ui.label(format!("Разрыв цепочки в событии #{index}: ожидалось {expected}, получено {actual}"));
            }
            ChainValidationError::MissingHeadEventId => {
                ui.label("Не получен head_event_id из backend");
            }
        }
    }

    // ================================================================
    // ВЕРИФИКАЦИЯ ПРОЕКТА
    // ================================================================
    fn verify_project(&mut self, ctx: &egui::Context) {
        self.verification_events.clear();
        self.verification_complete = false;
        self.verification_report.clear();
        self.verification_project = self.selected_project.clone();

        if self.selected_project.is_empty() {
            self.status = "❌ Выберите проект".to_string();
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
            Ok(contents) => contents,
            Err(e) => {
                self.status = format!("❌ Не удалось прочитать проект: {}", e);
                return;
            }
        };
        let project: Project = match serde_json::from_str(&project_json) {
            Ok(project) => project,
            Err(e) => {
                self.status = format!("❌ Не удалось разобрать проект: {}", e);
                return;
            }
        };
        let chain_id = match Uuid::parse_str(&project.chain_id) {
            Ok(id) => id,
            Err(_) => {
                self.status = "❌ Неправильный chain_id в проекте".to_string();
                return;
            }
        };

        let originals_dir = project_path.join("originals");

        // --- теперь уходит в фон ---
        self.loading_verify_project = true;
        self.status = "⏳ Проверка...".to_string();

        let tx = self.tx_resp.clone();
        let ctx = ctx.clone();
        self.rt.spawn_blocking(move || {
            let client = EvidentClient::new("http://127.0.0.1:3000");

            let result: Result<(evident_ledger::client::VerifyResponse, evident_ledger::client::ProofFile, PathBuf), String> = (|| {
                let verify_result = client::verify_chain(&client, chain_id)
                    .map_err(|e| friendly_error(&e))?;
                let proof = client::fetch_proof(&client, chain_id)
                    .map_err(|e| friendly_error(&e))?;
                Ok((verify_result, proof, originals_dir))
            })();

            let _ = tx.send(WorkerResponse::VerifyProjectDone(result));
            ctx.request_repaint();
        });
    }

    // ================================================================
    // ФИКСАЦИЯ
    // ================================================================
    fn do_commit(&mut self, ctx: &egui::Context) {
        if self.file_path.is_empty() {
            self.status = "❌ Выберите файл".to_string();
            return;
        }

        let project_name = if self.project_mode == ProjectMode::New {
            self.project_name.clone()
        } else {
            self.selected_project.clone()
        };

        if project_name.is_empty() {
            self.status = "❌ Укажите название проекта".to_string();
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
                self.status = format!("❌ Ошибка создания проекта: {}", e);
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
                    self.status = format!("❌ Не удалось прочитать проект: {}", e);
                    return;
                }
            };
            let project: Project = match serde_json::from_str(&project_json) {
                Ok(project) => project,
                Err(e) => {
                    self.status = format!("❌ Не удалось разобрать проект: {}", e);
                    return;
                }
            };
            project.chain_id
        };

        self.step = Step::Committing;
        self.status = "⏳ Отправка на сервер...".to_string();
        self.screen = Screen::CommitProgress;
        self.loading_commit = true;

        let file_bytes = match fs::read(&self.file_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.step = Step::Failed;
                self.status = format!("❌ Ошибка чтения файла: {}", e);
                self.loading_commit = false;
                return;
            }
        };

        let chain_uuid = match Uuid::parse_str(&chain_id) {
            Ok(id) => id,
            Err(_) => {
                self.step = Step::Failed;
                self.status = "❌ Неправильный chain_id".to_string();
                self.loading_commit = false;
                return;
            }
        };

        let tx = self.tx_resp.clone();
        let ctx = ctx.clone();
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
                        error: friendly_error(&e),
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
            self.verify_details = "Проект не выбран".to_string();
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
                self.verify_details = format!("⚠️ Не удалось прочитать проект: {}", e);
                return;
            }
        };
        let project: Project = match serde_json::from_str(&project_json) {
            Ok(project) => project,
            Err(e) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = format!("⚠️ Не удалось разобрать проект: {}", e);
                return;
            }
        };
        let chain_id = match Uuid::parse_str(&project.chain_id) {
            Ok(id) => id,
            Err(_) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = "⚠️ Неправильный chain_id".to_string();
                return;
            }
        };

        self.loading_verify_chain = true;
        self.verify_details = "⏳ Проверка...".to_string();

        let tx = self.tx_resp.clone();
        let ctx = ctx.clone();
        self.rt.spawn_blocking(move || {
            let client = EvidentClient::new("http://127.0.0.1:3000");
            let result = client::verify_chain(&client, chain_id).map_err(|e| friendly_error(&e));
            let _ = tx.send(WorkerResponse::VerifyChainDone(result));
            ctx.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // --- worker response handling ---
        while let Ok(resp) = self.rx_resp.try_recv() {
            match resp {
                WorkerResponse::VerifyProjectDone(res) => {
                    self.loading_verify_project = false;
                    match res {
                        Ok((verify_result, proof, originals_dir)) => {
                            self.network_error = false;
                            self.last_proof = Some(proof.clone());
                            self.state.head_event_id = Some(verify_result.head_event_id.clone());

                            for event in proof.events.iter() {
                                let local_integrity_ok = Self::check_local_integrity(
                                    &originals_dir, event.sequence, &event.file_hash,
                                );
                                self.verification_events.push(VerificationEvent {
                                    sequence: event.sequence,
                                    event_id: event.event_id.clone(),
                                    file_name: Self::find_original_name(&originals_dir, event.sequence)
                                        .unwrap_or_else(|| "Файл недоступен".to_string()),
                                    timestamp: "".to_string(),
                                    valid: verify_result.valid,
                                    error: if verify_result.valid {
                                        None
                                    } else {
                                        Some(verify_result.errors.join("; "))
                                    },
                                    error_type: if verify_result.valid {
                                        ErrorType::None
                                    } else {
                                        ErrorType::ChainBreak
                                    },
                                    local_integrity_ok,
                                });
                            }

                            self.verification_project = self.selected_project.clone();
                            self.verification_complete = true;

                            let local_tampered = self.verification_events.iter()
                                .any(|e| e.local_integrity_ok == Some(false));

                            self.verify_status = if !verify_result.valid {
                                VerifyStatus::Invalid
                            } else if local_tampered {
                                VerifyStatus::Partial
                            } else {
                                VerifyStatus::Valid
                            };

                            self.status = if !verify_result.valid {
                                "❌ Обнаружены нарушения".to_string()
                            } else if local_tampered {
                                "⚠️ Backend OK, но локальные файлы изменены".to_string()
                            } else {
                                "✅ Проект успешно проверен".to_string()
                            };

                            self.verification_report = if !verify_result.valid {
                                verify_result.errors.join("; ")
                            } else if local_tampered {
                                "Локальные файлы originals/ изменены или отсутствуют".to_string()
                            } else {
                                "Все события валидны. Локальная копия совпадает.".to_string()
                            };

                            self.screen = Screen::VerifyResult;
                        }
                        Err(e) => {
                            self.network_error = true;
                            self.status = format!("❌ {}", e);
                            self.verify_status = VerifyStatus::Partial;
                            self.verification_report = e;
                            self.screen = Screen::VerifyResult;
                        }
                    }
                }
                WorkerResponse::VerifyChainDone(res) => {
                    self.loading_verify_chain = false;
                    match res {
                        Ok(result) => {
                            if result.valid {
                                self.verify_status = VerifyStatus::Valid;
                                self.verify_details = "✅ Доказательство действительно".to_string();
                            } else {
                                self.verify_status = VerifyStatus::Invalid;
                                self.verify_details = result.errors.join("; ");
                            }
                        }
                        Err(e) => {
                            self.verify_status = VerifyStatus::Partial;
                            self.verify_details = format!("⚠️ Ошибка проверки: {}", e);
                        }
                    }
                }
                WorkerResponse::CommitDone(res) => {
                    self.loading_commit = false;
                    match res {
                        Ok(success) => {
                            let CommitSuccess {
                                commit, proof_path, file_hash,
                                project_path, proofs_dir, chain_uuid,
                                source_file_path, file_name,
                            } = success;

                            self.state.head_event_id = Some(commit.head_event_id.clone());

                            let original_name = match self.persist_original(
                                &project_path, Path::new(&source_file_path), commit.sequence,
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

                                    let parent_event_id = commit.events.iter()
                                        .find(|leaf| leaf.event_id == commit.event_id)
                                        .and_then(|leaf| Uuid::parse_str(&leaf.parent_event_id).ok());

                                    let proof = commit.tsa.as_ref().map(|_tsa| {
                                        TsaAttestation::new(
                                            commit.proof.root.clone(),
                                            commit.proof.signature.clone(),
                                            "".to_string()
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
                                        let _ = self.append_audit_event(&project_path, anchored_event);
                                    }
                                }

                                self.step = Step::Done;
                                self.status = "✅ Фиксация завершена".to_string();
                                self.commit_success = true;
                                self.screen = Screen::Result;
                                self.load_projects();
                            }
                        }
                        Err(failure) => {
                            let _ = self.append_audit_event(&failure.project_path, AuditEvent::failed(
                                Uuid::new_v4(),
                                failure.chain_uuid,
                                failure.file_hash,
                                None,
                                format!("submit failed: {}", failure.error),
                            ));
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
            ui.add_space(12.0);

            if self.screen == Screen::FileSelection {
                ui.heading("Evident Ledger");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("📄 Выбрать файл").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.file_path = path.display().to_string();
                            self.file_name = path.file_name().unwrap_or_default().to_string_lossy().into();
                            self.file_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            self.screen = Screen::SelectProject;
                            self.load_projects();
                            self.status = "Файл выбран".to_string();
                        }
                    }
                    if !self.file_path.is_empty() {
                        ui.label(format!("📁 {}", self.file_name));
                    }
                });

                ui.add_space(8.0);
                if ui.button("🔍 Проверить проект").clicked() {
                    self.screen = Screen::VerifyProject;
                    self.load_projects();
                }

                if !self.file_path.is_empty() {
                    let size_kb = self.file_size / 1024;
                    let size_mb = self.file_size / (1024 * 1024);
                    let size_str = if size_mb > 0 {
                        format!("{:.2} МБ", self.file_size as f64 / (1024.0 * 1024.0))
                    } else if size_kb > 0 {
                        format!("{} КБ", size_kb)
                    } else {
                        format!("{} байт", self.file_size)
                    };
                    ui.label(format!("📊 Размер: {}", size_str));
                    if self.file_size == 0 {
                        ui.colored_label(COLOR_INVALID, "⚠️ Файл пустой!");
                    }
                }

                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(&self.status);
                }
                return;
            }

            if self.screen == Screen::VerifyProject {
                ui.heading("🔍 Проверка проекта");
                ui.add_space(12.0);
                ui.label("Выберите проект для проверки:");
                ui.add_space(8.0);

                if self.loading_verify_project {
                    ui.spinner();
                    ui.label("Проверка...");
                } else if self.projects.is_empty() {
                    ui.label("📭 Нет сохранённых проектов.");
                } else {
                    let projects = self.projects.clone();
                    for project in projects {
                        if ui.button(&project).clicked() {
                            self.selected_project = project.clone();
                    self.verify_project(ui.ctx());
                        }
                    }
                }

                ui.add_space(12.0);
                if ui.button("⬅ Назад").clicked() {
                    self.screen = Screen::FileSelection;
                }
                return;
            }

            if self.screen == Screen::VerifyResult {
                ui.heading(format!("🔍 Проверка проекта: {}", self.verification_project));
                ui.add_space(12.0);

                match self.verify_status {
                    VerifyStatus::Valid => {
                        ui.colored_label(COLOR_VALID, "✅ ВСЕ СОБЫТИЯ ВАЛИДНЫ");
                    }
                    VerifyStatus::Invalid => {
                        ui.colored_label(COLOR_INVALID, "❌ ОБНАРУЖЕНЫ НАРУШЕНИЯ");
                    }
                    _ => {}
                }

                ui.add_space(8.0);
                ui.label(&self.verification_report);
                ui.add_space(12.0);

                ui.label("Цепочка событий:");
                ui.add_space(8.0);

                let projects_dir = match self.projects_dir() {
                    Ok(dir) => dir,
                    Err(err) => {
                        ui.colored_label(COLOR_INVALID, format!("⚠️ {err}"));
                        return;
                    }
                };

                if !self.network_error {
                    match Self::validate_chain(&self.verification_events, self.state.head_event_id.as_deref()) {
                        Ok(()) => {
                            for event in &self.verification_events {
                                ui.horizontal(|ui| {
                                    let label = if event.valid {
format!(
    "✅ EVENT {:03}    {}   {}",
    event.sequence,
    event.file_name,
    local_marker(event.local_integrity_ok)
)
                                    } else {
                         format!("❌ EVENT {:03}    {}   {}   ⚠️ {}", event.sequence, event.file_name, local_marker(event.local_integrity_ok), event.error.as_deref().unwrap_or("ошибка"))
                                    };
                                    if event.valid {
                                        ui.label(label);
                                    } else {
                                        ui.colored_label(COLOR_INVALID, label);
                                    }

                                    ui.add_enabled(false, egui::Button::new("📄 PDF"))
                                        .on_disabled_hover_text("Доступно в event-level экспорте (скоро)");

ui.add_enabled(false, egui::Button::new("📦 ZIP (скоро)"))
    .on_disabled_hover_text("Экспорт в ZIP появится в следующей версии");
                                });

                                if !event.valid && event.error_type != ErrorType::None {
                                    ui.label(format!("   └─ Тип: {:?}", event.error_type));
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
                    if ui.button("📄 Скачать заключение (PDF)").clicked() {
                        match &self.last_proof {
                            Some(proof) => {
                                let originals_dir = projects_dir.join(&self.verification_project).join("originals");
                                let proofs_dir = projects_dir.join(&self.verification_project).join("proofs");
                                let _ = fs::create_dir_all(&proofs_dir);

                                let verify_valid = self.verify_status == VerifyStatus::Valid;
                                let input = Self::build_certificate_input(proof, &self.verification_events, &originals_dir, verify_valid);

                                match generate_certificate_pdf(&input) {
                                    Ok(pdf_bytes) => {
                                        let pdf_path = proofs_dir.join("certificate.pdf");
                                        match fs::write(&pdf_path, pdf_bytes) {
                                            Ok(()) => {
                                                self.status = format!("✅ Заключение сохранено: {}", pdf_path.display());
                                            }
                                            Err(e) => {
                                                self.status = format!("❌ Не удалось сохранить PDF: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        self.status = format!("❌ Ошибка генерации PDF: {}", e);
                                    }
                                }
                            }
                            None => {
                                self.status = "❌ Нет данных для заключения — сначала выполните проверку".to_string();
                            }
                        }
                    }
                    if ui.button("📦 Скачать проект (ZIP)").clicked() {
                        self.status = "⏳ Упаковка проекта...".to_string();
                    }
                });

                ui.add_space(8.0);
                if ui.button("⬅ Назад").clicked() {
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
                ui.heading("Куда сохранить доказательство?");
                ui.add_space(8.0);

                let size_kb = self.file_size / 1024;
                let size_mb = self.file_size / (1024 * 1024);
                let size_str = if size_mb > 0 {
                    format!("{:.2} МБ", self.file_size as f64 / (1024.0 * 1024.0))
                } else if size_kb > 0 {
                    format!("{} КБ", size_kb)
                } else {
                    format!("{} байт", self.file_size)
                };
                ui.label(format!("📁 Файл: {} ({})", self.file_name, size_str));
                if self.file_size == 0 {
                    ui.colored_label(COLOR_INVALID, " ⚠️ Файл пустой!");
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.radio(self.project_mode == ProjectMode::New, "🆕 Создать новый проект").clicked() {
                        self.project_mode = ProjectMode::New;
                    }
                    if ui.radio(self.project_mode == ProjectMode::Existing, "📂 Использовать существующий").clicked() {
                        self.project_mode = ProjectMode::Existing;
                        self.load_projects();
                    }
                });

                ui.add_space(12.0);

                if self.project_mode == ProjectMode::New {
                    ui.label("Название проекта:");
                    ui.text_edit_singleline(&mut self.project_name);
                    ui.add_space(8.0);
                    let projects_dir = match self.projects_dir() {
                        Ok(dir) => dir,
                        Err(err) => {
                            ui.colored_label(COLOR_INVALID, format!("⚠️ {err}"));
                            return;
                        }
                    };
                    ui.label(format!("📁 Папка: {}/{}", projects_dir.display(), self.project_name));
                } else {
                    if self.projects.is_empty() {
                        ui.label("📭 Нет сохранённых проектов. Создайте новый.");
                    } else {
                        ui.label("Выберите проект:");
                        for project in &self.projects {
                            let is_selected = self.selected_project == *project;
                            let button_text = if is_selected {
                                format!("✅ {}", project)
                            } else {
                                project.clone()
                            };
                            if ui.button(button_text).clicked() {
                                self.selected_project = project.clone();
                            }
                        }
                        if !self.selected_project.is_empty() {
                            ui.colored_label(COLOR_VALID, format!("📂 Выбран проект: {}", self.selected_project));
                        }
                    }
                }

                ui.add_space(12.0);
                ui.label("📁 Копия файла будет сохранена в папку проекта.");
                ui.label("📋 Аудит будет записываться в папку Аудит/");
                ui.add_space(8.0);
                ui.label("Оригинальный файл никогда не изменяется.");

                ui.add_space(12.0);

                let can_commit = !self.file_path.is_empty()
                    && ((self.project_mode == ProjectMode::New && !self.project_name.is_empty())
                        || (self.project_mode == ProjectMode::Existing && !self.selected_project.is_empty()));

                if self.loading_commit {
                    ui.spinner();
                    ui.label("Отправка...");
                } else if ui.add_enabled(can_commit, egui::Button::new("✅ Зафиксировать")).clicked() {
                    self.do_commit(ui.ctx());
                }

                if ui.button("⬅ Назад к файлу").clicked() {
                    self.screen = Screen::FileSelection;
                    self.file_path.clear();
                    self.file_name.clear();
                }

                if !self.error_message.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(COLOR_INVALID, &self.error_message);
                }
                return;
            }

            if self.screen == Screen::CommitProgress {
                ui.heading("Фиксация документа");
                ui.add_space(12.0);

                ui.label(format!("📁 Файл: {}", self.file_name));
                ui.label(format!("📁 Проект: {}", if self.project_mode == ProjectMode::New { &self.project_name } else { &self.selected_project }));
                ui.add_space(12.0);

                match self.step {
                    Step::Hashing => {
                        let _ = ui.label("⏳ Вычисление SHA-256...");
                    }
                    Step::Committing => {
                        let _ = ui.label("⏳ Отправка на сервер...");
                    }
                    Step::TsaWaiting => {
                        let _ = ui.label("⏳ Получение TSA...");
                    }
                    Step::Done => {
                        let _ = ui.label("✅ Фиксация завершена");
                    }
                    Step::Failed => {
                        let _ = ui.label("❌ Ошибка");
                    }
                    _ => {}
                }

                ui.add_space(8.0);
                ui.label(&self.status);

                if self.step == Step::Done {
                    ui.add_space(12.0);
                    if ui.button("📋 Посмотреть результат").clicked() {
                        self.screen = Screen::Result;
                    }
                }

                if self.step == Step::Failed {
                    ui.add_space(12.0);
                    if ui.button("⬅ Назад").clicked() {
                        self.screen = Screen::SelectProject;
                        self.step = Step::Idle;
                    }
                }
                return;
            }

            if self.screen == Screen::Result {
                ui.heading("✅ Документ успешно зафиксирован");
                ui.add_space(8.0);

                ui.label(format!("📁 Проект: {}", if self.project_mode == ProjectMode::New { &self.project_name } else { &self.selected_project }));
                ui.label(format!("📄 Событие: {}", self.event_id));
                ui.label(format!("📄 Доказательство: {}", self.proof_path));

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if self.loading_verify_chain {
                        ui.spinner();
                        ui.label("Проверка...");
                    } else if ui.button("🔍 Проверить").clicked() {
                        self.do_verify(ui.ctx());
                    }
                    if !self.proof_path.is_empty() {
                        if ui.button("📄 Открыть PDF").clicked() {
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
                    if ui.button("📋 Вся цепочка аудита").clicked() {
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
                        ui.colored_label(COLOR_VALID, "✅ ДЕЙСТВИТЕЛЬНО");
                        ui.colored_label(COLOR_VALID, &self.verify_details);
                    }
                    VerifyStatus::Invalid => {
                        ui.colored_label(COLOR_INVALID, "❌ НЕДЕЙСТВИТЕЛЬНО");
                        ui.colored_label(COLOR_INVALID, &self.verify_details);
                    }
                    VerifyStatus::Partial => {
                        ui.colored_label(COLOR_PARTIAL, "⚠️ ЧАСТИЧНО");
                        ui.colored_label(COLOR_PARTIAL, &self.verify_details);
                    }
                    _ => {}
                }

                ui.add_space(12.0);
                if ui.button("📁 К выбору файла").clicked() {
                    self.screen = Screen::FileSelection;
                    self.commit_success = false;
                    self.file_path.clear();
                    self.file_name.clear();
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

fn local_marker(status: Option<bool>) -> &'static str {
    match status {
        Some(true) => "✅ Local",
        Some(false) => "❌ Local MODIFIED",
        None => "⚠️ Local N/A",
    }
}

fn friendly_error(e: &impl std::fmt::Display) -> String {
    let msg = e.to_string();
    if msg.contains("connection refused") 
        || msg.contains("connect") 
        || msg.contains("error sending request") 
        || msg.contains("dns") 
        || msg.contains("failed to lookup") {
        "Сервер недоступен. Проверьте, что программа Evident Ledger запущена, и повторите попытку.".into()
    } else if msg.contains("timed out") || msg.contains("timeout") {
        "Сервер не отвечает слишком долго. Попробуйте ещё раз.".into()
    } else {
        format!("Произошла ошибка: {msg}")
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
    eframe::run_native(
        "Evident Ledger",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    )
}

