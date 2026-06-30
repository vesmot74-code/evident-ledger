use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Evident Ledger",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

struct App {
    file_path: String,
    status: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            file_path: String::new(),
            status: "Ready".to_string(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Evident Ledger");
        ui.separator();

        if ui.button("Select File").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                self.file_path = path.display().to_string();
                self.status = "File selected".to_string();
            }
        }

        ui.label(format!("File: {}", self.file_path));

        if ui.button("Commit").clicked() {
            if !self.file_path.is_empty() {
                self.status = "Committing...".to_string();
            }
        }

        ui.label(format!("Status: {}", self.status));
    }
}
