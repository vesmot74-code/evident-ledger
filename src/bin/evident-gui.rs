use eframe::egui;
use std::process::Command;
use std::str;

#[derive(Default)]
struct App {
    file_path: String,
    status: String,
    result: String,
    show_pdf_dialog: bool,
    commit_success: bool,
    event_id: String,
    chain_id: String,
    proof_path: String,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Evident Ledger");
        ui.separator();

        // Select File
        if ui.button("Select File").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                self.file_path = path.display().to_string();
                self.status = "File selected".to_string();
                self.result.clear();
                self.commit_success = false;
                self.show_pdf_dialog = false;
            }
        }

        ui.label(format!("File: {}", self.file_path));

        // Commit
        if ui.button("Commit").clicked() {
            if self.file_path.is_empty() {
                self.status = "Error: No file selected".to_string();
            } else {
                self.status = "Hashing...".to_string();
                self.result.clear();
                self.commit_success = false;
                self.show_pdf_dialog = false;

                let chain_id = "11111111-1111-1111-1111-111111111111";
                let output = Command::new("./target/debug/evident")
                    .args(&["commit", &self.file_path, "--chain", chain_id])
                    .output();

                match output {
                    Ok(out) => {
                        if out.status.success() {
                            let stdout = str::from_utf8(&out.stdout).unwrap_or("");
                            self.status = "✅ Commit successful!".to_string();
                            self.result = stdout.trim().to_string();
                            self.commit_success = true;
                            self.show_pdf_dialog = true;
                            self.chain_id = chain_id.to_string();

                            // Парсим event_id и proof_path из вывода
                            for line in stdout.lines() {
                                if line.starts_with("anchored    event=") {
                                    self.event_id = line.replace("anchored    event=", "").to_string();
                                }
                                if line.starts_with("proof       ") {
                                    self.proof_path = line.replace("proof       ", "").to_string();
                                }
                            }
                        } else {
                            let stderr = str::from_utf8(&out.stderr).unwrap_or("");
                            self.status = "❌ Commit failed".to_string();
                            self.result = format!("Error: {}", stderr.trim());
                            self.commit_success = false;
                            self.show_pdf_dialog = false;
                        }
                    }
                    Err(e) => {
                        self.status = "❌ Error running command".to_string();
                        self.result = format!("{}", e);
                        self.commit_success = false;
                        self.show_pdf_dialog = false;
                    }
                }
            }
        }

        ui.label(format!("Status: {}", self.status));

        if !self.result.is_empty() {
            ui.separator();
            ui.label("Result:");
            ui.label(&self.result);
        }

        // PDF Dialog
        if self.show_pdf_dialog && self.commit_success {
            ui.separator();
            ui.heading("📄 Фиксация подтверждена");
            ui.label(format!("   Event ID:    {}", self.event_id));
            ui.label(format!("   Chain ID:    {}", self.chain_id));
            ui.label(format!("   Proof:       {}", self.proof_path));

            ui.horizontal(|ui| {
                if ui.button("📄 Вывести полный PDF?").clicked() {
                    if !self.proof_path.is_empty() {
                        self.status = "Generating PDF...".to_string();
                        let pdf_path = "report.pdf";
                        let output = Command::new("./target/debug/evident")
                      .args(&["report", "generate", &self.chain_id])
                            .output();

                        match output {
                            Ok(out) => {
                                if out.status.success() {
                                    self.status = "✅ PDF generated!".to_string();
                                    self.result = format!("PDF saved to {}", pdf_path);
                                    // Открываем PDF
                                    let _ = Command::new("open").arg(pdf_path).output();
                                } else {
                                    let stderr = str::from_utf8(&out.stderr).unwrap_or("");
                                    self.status = "❌ PDF generation failed".to_string();
                                    self.result = format!("Error: {}", stderr.trim());
                                }
                            }
                            Err(e) => {
                                self.status = "❌ Error generating PDF".to_string();
                                self.result = format!("{}", e);
                            }
                        }
                        self.show_pdf_dialog = false;
                    }
                }

                if ui.button("❌ Нет, спасибо").clicked() {
                    self.show_pdf_dialog = false;
                }
            });
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([700.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Evident Ledger",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
