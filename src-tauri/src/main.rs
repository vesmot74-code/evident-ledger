use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native("Evident Ledger", options, Box::new(|_cc| Box::new(MyApp::default())))
}

struct MyApp { output: String }
impl Default for MyApp { fn default() -> Self { Self { output: "Ledger Ready".to_owned() } } }
impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Evident Ledger");
            ui.label(&self.output);
        });
    }
}