use eframe::egui;
use std::process::Command;
use std::str;

#[derive(Default)]
struct App {
    file_path: String,
    status: String,
    result: String,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Evident Ledger");
        ui.separator();

        if ui.button("Select File").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                self.file_path = path.display().to_string();
                self.status = "File selected".to_string();
                self.result.clear();
            }
        }

        ui.label(format!("File: {}", self.file_path));

        if ui.button("Commit").clicked() {
            if self.file_path.is_empty() {
                self.status = "Error: No file selected".to_string();
            } else {
                self.status = "Committing...".to_string();
                self.result.clear();

                let chain_id = "11111111-1111-1111-1111-111111111111";
                let output = Command::new("./target/debug/evident")
                    .args(&["commit", &self.file_path, "--chain", chain_id])
                    .output();

                match output {
                    Ok(out) => {
                        if out.status.success() {
                            let stdout = str::from_utf8(&out.stdout).unwrap_or("");
                            self.status = "Commit successful!".to_string();
                            self.result = stdout.trim().to_string();
                        } else {
                            let stderr = str::from_utf8(&out.stderr).unwrap_or("");
                            self.status = "Commit failed".to_string();
                            self.result = format!("Error: {}", stderr.trim());
                        }
                    }
                    Err(e) => {
                        self.status = "Error running command".to_string();
                        self.result = format!("{}", e);
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
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([600.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Evident Ledger",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
