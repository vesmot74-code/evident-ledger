use eframe::egui;
use std::process::Command;
use std::str;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
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
    file_path: String,
    file_name: String,
    file_size: u64,
    file_hash: String,
    projects: Vec<String>,
    project_name: String,
    selected_project: String,
    project_mode: ProjectMode,
    status: String,
    step: Step,
    commit_success: bool,
    event_id: String,
    proof_path: String,
    verify_status: VerifyStatus,
    verify_details: String,
    screen: Screen,
    error_message: String,
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
        let _ = fs::create_dir_all(&originals_dir);
        let _ = fs::create_dir_all(&proofs_dir);
        let _ = fs::create_dir_all(&audit_dir);

        // 3. Копируем файл
        let dest = originals_dir.join(&self.file_name);
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
                    let stdout = str::from_utf8(&out.stdout).unwrap_or("");

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
                            .and_then(|mut f| f.write_all(audit_line.as_bytes()));
                    }

self.status = "⏳ Генерация PDF...".to_string();
let _ = Command::new("./target/debug/evident")
    .args(&["report", "generate", &chain_id])
    .output();

// Копируем PDF в папку проекта
let source_pdf = format!("/Users/iuriiveselskii/.evident/proofs/{}/proof.pdf", chain_id);
let dest_pdf = proofs_dir.join("proof.pdf");
if std::path::Path::new(&source_pdf).exists() {
    let _ = fs::copy(&source_pdf, &dest_pdf);
    println!("DEBUG: PDF скопирован в {}", dest_pdf.display());
} else {
    println!("DEBUG: PDF не найден: {}", source_pdf);
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

        let output = Command::new("./target/debug/evident")
            .args(&["verify", &self.proof_path])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    self.verify_status = VerifyStatus::Valid;
                    self.verify_details = "✅ Доказательство действительно".to_string();
                } else {
                    self.verify_status = VerifyStatus::Invalid;
                    self.verify_details = "❌ Проверка не пройдена".to_string();
                }
            }
            Err(e) => {
                self.verify_status = VerifyStatus::Partial;
                self.verify_details = format!("⚠️ Частичная проверка: {}", e);
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
                    ui.label(format!("Размер: {} байт", self.file_size));
                    ui.label(format!("SHA-256: {}", &self.file_hash[..16]));
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
                ui.label(format!("📁 Файл: {}", self.file_name));
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
                            if ui.button(project).clicked() {
                                self.selected_project = project.clone();
                            }
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
                    if ui.button("🔍 Проверить").clicked() {
                        self.do_verify();
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
    println!("DEBUG: Открываем PDF: {}", pdf_path.display());
    if pdf_path.exists() {
        let _ = Command::new("open").arg(&pdf_path).output();
        self.status = format!("✅ PDF открыт: {}", pdf_path.display());
    } else {
        self.status = format!("❌ PDF не найден: {}", pdf_path.display());
    }
}
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
        });
    }
}

fn main() -> Result<(), eframe::Error> {
    // Создаём папку Evident Projects при запуске
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let projects_dir = PathBuf::from(home).join("Evident Projects");
    let _ = fs::create_dir_all(&projects_dir);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 600.0])
            .with_min_inner_size([500.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Evident Ledger",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
