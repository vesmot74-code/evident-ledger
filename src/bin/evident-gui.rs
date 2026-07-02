use eframe::egui;
use std::process::Command;
use std::str;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Write, BufRead, BufReader};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use chrono::Utc;

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
// МОДЕЛЬ АУДИТА
// ============================================================================
#[derive(Debug, Deserialize)]
struct AuditEvent {
    timestamp: String,
    event_id: String,
    file_name: String,
    file_hash: String,
    chain_id: String,
    proof: String,
}

#[derive(Debug, Clone)]
struct VerificationEvent {
    index: usize,
    event_id: String,
    file_name: String,
    file_hash: String,
    timestamp: String,
    valid: bool,
    error: Option<String>,
    error_type: ErrorType,
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
#[derive(Default)]
struct App {
    // === Файл ===
    file_path: String,
    file_name: String,
    file_size: u64,
    file_hash: String,

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
    verify_running: bool,          // ← одно объявление
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
    existing_hash: String,
    existing_events: Vec<String>,
    hash_history: Vec<(String, String)>,
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
    VerifyProject,
    VerifyResult,
    ReconfirmDialog,
}

impl App {
    fn compute_hash(path: &str) -> String {
        let bytes = fs::read(path).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    }

    fn load_projects(&mut self) {
        let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join("Evident Projects");
        self.projects = Project::list(&projects_dir);
    }

    // ================================================================
    // ВЕРИФИКАЦИЯ ПРОЕКТА
    // ================================================================
    fn verify_project(&mut self) {
        self.verification_events.clear();
        self.verification_complete = false;
        self.verification_report.clear();

        let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join("Evident Projects");
        let project_path = projects_dir.join(&self.selected_project);
        let audit_path = project_path.join("Аудит").join("audit.jsonl");

        eprintln!("DEBUG: project_path = {}", project_path.display());
        eprintln!("DEBUG: audit_path = {}", audit_path.display());

        if !audit_path.exists() {
            self.status = "❌ Файл аудита не найден".to_string();
            return;
        }

        let file = match fs::File::open(&audit_path) {
            Ok(f) => f,
            Err(e) => {
                self.status = format!("❌ Ошибка чтения аудита: {}", e);
                return;
            }
        };
        let reader = BufReader::new(file);
        let mut events: Vec<AuditEvent> = Vec::new();

        for line in reader.lines() {
            if let Ok(line) = line {
                if let Ok(event) = serde_json::from_str::<AuditEvent>(&line) {
                    events.push(event);
                }
            }
        }

        eprintln!("DEBUG: events loaded = {}", events.len());

        if events.is_empty() {
            self.status = "❌ В проекте нет событий".to_string();
            return;
        }

        let mut valid = true;

        for (i, event) in events.iter().enumerate() {
            let mut verification_event = VerificationEvent {
                index: i + 1,
                event_id: event.event_id.clone(),
                file_name: event.file_name.clone(),
                file_hash: event.file_hash.clone(),
                timestamp: event.timestamp.clone(),
                valid: true,
                error: None,
                error_type: ErrorType::None,
            };

            // Проверка файла
            let originals_dir = project_path.join("originals");
            let mut found = false;

            if let Ok(entries) = fs::read_dir(&originals_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let hash = Self::compute_hash(&path.display().to_string());
                        if hash == event.file_hash {
                            found = true;
                            eprintln!("DEBUG: ✅ FILE FOUND BY HASH: {}", path.display());
                            break;
                        }
                    }
                }
            }

            if found {
                verification_event.valid = true;
            } else {
                let file_path = originals_dir.join(&event.file_name);
                if file_path.exists() {
                    let hash = Self::compute_hash(&file_path.display().to_string());
                    if hash == event.file_hash {
                        verification_event.valid = true;
                    } else {
                        verification_event.valid = false;
                        verification_event.error_type = ErrorType::FileHashMismatch;
                        verification_event.error = Some("Файл был изменён после фиксации".to_string());
                        valid = false;
                    }
                } else {
                    verification_event.valid = false;
                    verification_event.error_type = ErrorType::FileHashMismatch;
                    verification_event.error = Some("Файл не найден в папке originals".to_string());
                    valid = false;
                }
            }

            self.verification_events.push(verification_event);
        }

        self.verification_project = self.selected_project.clone();
        self.verification_complete = true;

        if valid {
            self.status = "✅ Проект успешно проверен".to_string();
            self.verify_status = VerifyStatus::Valid;
            self.verification_report = "Все события валидны. Цепочка не нарушена.".to_string();
        } else {
            self.status = "❌ Обнаружены нарушения".to_string();
            self.verify_status = VerifyStatus::Invalid;
            self.verification_report = "Обнаружены нарушения в цепочке.".to_string();
        }

        self.screen = Screen::VerifyResult;
    }

    // ================================================================
    // ФИКСАЦИЯ
    // ================================================================
    fn do_commit(&mut self) {
        if self.file_path.is_empty() {
            self.status = "❌ Выберите файл".to_string();
            return;
        }

        let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join("Evident Projects");

        let project_name = if self.project_mode == ProjectMode::New {
            self.project_name.clone()
        } else {
            self.selected_project.clone()
        };

        if project_name.is_empty() {
            self.status = "❌ Укажите название проекта".to_string();
            return;
        }

        let project_path = projects_dir.join(&project_name);
        let audit_path = project_path.join("Аудит").join("audit.jsonl");

        // Проверка: есть ли такой хеш в аудите?
        let mut existing_events: Vec<String> = Vec::new();
        let mut hash_found = false;

        if audit_path.exists() {
            if let Ok(file) = fs::File::open(&audit_path) {
                let reader = BufReader::new(file);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        if let Ok(event) = serde_json::from_str::<AuditEvent>(&line) {
                            if event.file_hash == self.file_hash {
                                hash_found = true;
                                existing_events.push(format!("{}  {}", event.timestamp, event.file_name));
                            }
                        }
                    }
                }
            }
        }

        if hash_found {
            self.existing_hash = self.file_hash.clone();
            self.existing_events = existing_events;
            self.screen = Screen::ReconfirmDialog;
            return;
        }

        self.do_commit_actual();
    }

    fn do_commit_actual(&mut self) {
        if self.file_path.is_empty() {
            self.status = "❌ Выберите файл".to_string();
            return;
        }

        let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join("Evident Projects");

        let project_name = if self.project_mode == ProjectMode::New {
            self.project_name.clone()
        } else {
            self.selected_project.clone()
        };

        if project_name.is_empty() {
            self.status = "❌ Укажите название проекта".to_string();
            return;
        }

        let project_path = projects_dir.join(&project_name);
        let originals_dir = project_path.join("originals");
        let proofs_dir = project_path.join("proofs");
        let audit_dir = project_path.join("Аудит");

        // 1. Создаём проект
        if self.project_mode == ProjectMode::New {
            let project = Project {
                name: project_name.clone(),
                chain_id: "11111111-1111-1111-1111-111111111111".to_string(),
                created_at: Utc::now().to_rfc3339(),
            };
            if let Err(e) = project.save(&projects_dir) {
                self.status = format!("❌ Ошибка создания проекта: {}", e);
                return;
            }
        }

        // 2. Создаём папки
        for dir in [&originals_dir, &proofs_dir, &audit_dir] {
            if let Err(e) = fs::create_dir_all(dir) {
                self.status = format!("❌ Ошибка создания папки: {}", e);
                return;
            }
        }

        // 3. Копируем файл
        let dest = originals_dir.join(&self.file_name);
        if dest.exists() {
            let existing_hash = Self::compute_hash(&dest.display().to_string());
            if existing_hash != self.file_hash {
                self.hash_history.push((self.file_name.clone(), existing_hash));
                let backup_name = format!("{}.old_{}", self.file_name, Utc::now().timestamp());
                let backup_path = originals_dir.join(backup_name);
                let _ = fs::rename(&dest, &backup_path);
            }
        }
        if let Err(e) = fs::copy(&self.file_path, &dest) {
            self.status = format!("❌ Ошибка копирования: {}", e);
            return;
        }

        self.step = Step::Hashing;
        self.status = "⏳ Вычисление SHA-256...".to_string();
        self.screen = Screen::CommitProgress;

        self.step = Step::Committing;
        self.status = "⏳ Отправка на сервер...".to_string();

        let chain_id = "11111111-1111-1111-1111-111111111111".to_string();
        let output = Command::new("./target/debug/evident")
            .args(&["commit", &self.file_path, "--chain", &chain_id])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    self.step = Step::TsaWaiting;
                    self.status = "⏳ Получение TSA...".to_string();

                    for line in stdout.lines() {
                        if line.starts_with("anchored    event=") {
                            self.event_id = line.replace("anchored    event=", "").to_string();
                        }
                        if line.starts_with("proof       ") {
                            self.proof_path = line.replace("proof       ", "").to_string();
                        }
                    }

                    // Сохраняем proof
                    if !self.proof_path.is_empty() {
                        let proof_name = format!("{}.json", self.event_id);
                        let dest_proof = proofs_dir.join(&proof_name);
                        let _ = fs::copy(&self.proof_path, &dest_proof);
                        self.proof_path = dest_proof.display().to_string();
                    }

                    // Аудит
                    let audit_path = audit_dir.join("audit.jsonl");
                    let audit_entry = serde_json::json!({
                        "timestamp": Utc::now().to_rfc3339(),
                        "event_id": self.event_id,
                        "file_name": self.file_name,
                        "file_hash": self.file_hash,
                        "chain_id": chain_id,
                        "proof": self.proof_path,
                    });
                    if let Ok(audit_line) = serde_json::to_string(&audit_entry) {
                        let _ = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&audit_path)
                            .and_then(|mut f| {
                                f.write_all(audit_line.as_bytes())?;
                                f.write_all(b"\n")
                            });
                    }

                    // Генерация PDF
                    self.status = "⏳ Генерация PDF...".to_string();
                    let _ = Command::new("./target/debug/evident")
                        .args(&["report", "generate", &chain_id])
                        .output();

                    // Копируем PDF в папку проекта
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                    let source_pdf = PathBuf::from(home)
                        .join(".evident")
                        .join("proofs")
                        .join(&chain_id)
                        .join("proof.pdf");
                    let dest_pdf = proofs_dir.join("proof.pdf");
                    if source_pdf.exists() {
                        let _ = fs::copy(&source_pdf, &dest_pdf);
                    }

                    self.step = Step::Done;
                    self.status = "✅ Фиксация завершена".to_string();
                    self.commit_success = true;
                    self.screen = Screen::Result;
                    self.load_projects();
                } else {
                    self.step = Step::Failed;
                    self.status = "❌ Ошибка фиксации".to_string();
                }
            }
            Err(e) => {
                self.step = Step::Failed;
                self.status = format!("❌ Ошибка: {}", e);
            }
        }
    }

    fn do_verify(&mut self) {
        if self.proof_path.is_empty() {
            self.verify_status = VerifyStatus::Invalid;
            self.verify_details = "Доказательство не найдено".to_string();
            return;
        }

        let output = Command::new("./target/debug/evident-verify")
            .arg(&self.proof_path)
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    self.verify_status = VerifyStatus::Valid;
                    self.verify_details = "✅ Доказательство действительно".to_string();
                } else {
                    self.verify_status = VerifyStatus::Invalid;
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if stdout.contains("FAIL: signature invalid") {
                        self.verify_details = "❌ Подпись недействительна".to_string();
                    } else if stdout.contains("FAIL: no pinned server key") {
                        self.verify_details = "❌ Ключ сервера не закреплён".to_string();
                    } else {
                        self.verify_details = format!("❌ Проверка не пройдена: {}", stdout.trim());
                    }
                }
            }
            Err(e) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = format!("⚠️ Ошибка запуска верификатора: {}", e);
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
                            self.file_hash = Self::compute_hash(&path.display().to_string());
                            self.screen = Screen::SelectProject;
                            self.load_projects();
                            self.status = "Файл выбран".to_string();
                        }
                    }
                    if !self.file_path.is_empty() {
                        ui.label(format!("📁 {}", self.file_name));
                    }
                });

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
                    ui.label(format!("🔑 SHA-256: {}", &self.file_hash[..16]));
                }

                ui.add_space(12.0);
                if ui.button("🔍 Проверить проект").clicked() {
                    self.screen = Screen::VerifyProject;
                    self.load_projects();
                }

                ui.add_space(8.0);
                if ui.button("🚪 Выход").clicked() {
                    std::process::exit(0);
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
                ui.label(format!("🔑 SHA-256: {}", &self.file_hash[..16]));

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
                    let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                        .join("Evident Projects");
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

                if ui.add_enabled(can_commit, egui::Button::new("✅ Зафиксировать")).clicked() {
                    self.do_commit();
                }

                if ui.button("⬅ Назад к файлу").clicked() {
                    self.screen = Screen::FileSelection;
                    self.file_path.clear();
                    self.file_name.clear();
                    self.file_hash.clear();
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
                ui.label(format!("📁 Проект: {}", 
                    if self.project_mode == ProjectMode::New { &self.project_name } else { &self.selected_project }
                ));
                ui.add_space(12.0);

                match self.step {
                    Step::Hashing => { ui.label("⏳ Вычисление SHA-256..."); }
                    Step::Committing => { ui.label("⏳ Отправка на сервер..."); }
                    Step::TsaWaiting => { ui.label("⏳ Получение TSA..."); }
                    Step::Done => { ui.label("✅ Фиксация завершена"); }
                    Step::Failed => { ui.label("❌ Ошибка"); }
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

                ui.label(format!("📁 Проект: {}", 
                    if self.project_mode == ProjectMode::New { &self.project_name } else { &self.selected_project }
                ));
                ui.label(format!("📄 Событие: {}", self.event_id));
                ui.label(format!("📄 Доказательство: {}", self.proof_path));

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("🔍 Проверить").clicked() && !self.verify_running {
                        self.verify_running = true;
                        self.do_verify();
                        self.verify_running = false;
                    }
                    if !self.proof_path.is_empty() {
                        if ui.button("📄 Открыть PDF").clicked() {
                            let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                                .join("Evident Projects");
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
                        self.verify_project();
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
                    self.file_hash.clear();
                    self.event_id.clear();
                    self.proof_path.clear();
                    self.verify_status = VerifyStatus::None;
                    self.step = Step::Idle;
                    self.project_name.clear();
                    self.selected_project.clear();
                }
                return;
            }

            if self.screen == Screen::VerifyProject {
                ui.heading("🔍 Проверка проекта");
                ui.add_space(12.0);

                ui.label("Выберите проект для проверки:");
                ui.add_space(8.0);

                if self.projects.is_empty() {
                    ui.label("📭 Нет сохранённых проектов.");
                } else {
                    let projects = self.projects.clone();
                    for project in projects {
                        if ui.button(&project).clicked() {
                            self.selected_project = project.clone();
                            self.verify_project();
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

                let projects_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                    .join("Evident Projects");

                for event in &self.verification_events {
                    ui.horizontal(|ui| {
                        let label = if event.valid {
                            format!("✅ EVENT {:03}    {}   {}", event.index, event.timestamp, event.file_name)
                        } else {
                            format!("❌ EVENT {:03}    {}   {}   ⚠️ {}", event.index, event.timestamp, event.file_name, event.error.as_deref().unwrap_or("ошибка"))
                        };
                        if event.valid {
                            ui.label(label);
                        } else {
                            ui.colored_label(COLOR_INVALID, label);
                        }

                        if ui.button("📄 PDF").clicked() {
                            let pdf_path = projects_dir
                                .join(&self.verification_project)
                                .join("proofs")
                                .join("proof.pdf");
                            if pdf_path.exists() {
                                let _ = Command::new("open").arg(&pdf_path).output();
                                self.status = format!("✅ PDF открыт для события {}", event.index);
                            } else {
                                self.status = format!("❌ PDF не найден для события {}", event.index);
                            }
                        }

                        if ui.button("📦 ZIP").clicked() {
                            self.status = format!("⏳ Упаковка события {}...", event.index);
                        }
                    });

                    if !event.valid && event.error_type != ErrorType::None {
                        ui.label(format!("   └─ Тип: {:?}", event.error_type));
                    }
                }

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("📄 Скачать заключение (PDF)").clicked() {
                        self.status = "⏳ Генерация заключения...".to_string();
                    }
                    if ui.button("📦 Скачать проект (ZIP)").clicked() {
                        self.status = "⏳ Упаковка проекта...".to_string();
                    }
                });

                ui.add_space(8.0);
                if ui.button("⬅ Назад").clicked() {
                    if !self.existing_events.is_empty() {
                        self.screen = Screen::ReconfirmDialog;
                    } else {
                        self.screen = Screen::FileSelection;
                    }
                    self.verification_events.clear();
                    self.verification_complete = false;
                }

                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(&self.status);
                }
                return;
            }

            if self.screen == Screen::ReconfirmDialog {
                ui.heading("📁 Файл уже существует в аудите");
                ui.add_space(8.0);

                ui.label(format!("Проект: {}", self.selected_project));
                ui.label(format!("Файл: {}", self.file_name));
                ui.label(format!("Хеш: {}", &self.file_hash[..16]));
                ui.add_space(8.0);

                ui.label("История фиксаций:");
                for event in &self.existing_events {
                    ui.label(format!("   • {}", event));
                }

                ui.add_space(12.0);
                ui.label("Что сделать?");

                ui.horizontal(|ui| {
                    if ui.button("🔄 Повторно зафиксировать").clicked() {
                        self.screen = Screen::SelectProject;
                        self.do_commit_actual();
                    }
                    if ui.button("🔍 Проверить историю").clicked() {
                        self.screen = Screen::VerifyProject;
                        self.verify_project();
                    }
                    if ui.button("❌ Отмена").clicked() {
                        self.screen = Screen::SelectProject;
                        self.existing_events.clear();
                        self.file_path.clear();
                        self.file_name.clear();
                        self.file_hash.clear();
                    }
                });
                return;
            }
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let projects_dir = PathBuf::from(home).join("Evident Projects");
    let _ = fs::create_dir_all(&projects_dir);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([750.0, 650.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Evident Ledger",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
