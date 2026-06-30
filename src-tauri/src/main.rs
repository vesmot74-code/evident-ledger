// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::{egui, epi};

fn main() {
  let options = eframe::NativeOptions::default();
  eframe::run_native(
    "Evident UI",
    options,
    Box::new(|_cc| Box::new(MyApp::default())),
  );
}

#[derive(Default)]
struct MyApp {
  output: String,
}

impl epi::App for MyApp {
  fn name(&self) -> &str {
    "Evident UI"
  }

  fn update(&mut self, ctx: &egui::CtxRef, _frame: &epi::Frame) {
    egui::CentralPanel::default().show(ctx, |ui| {
      ui.heading("Evident UI");

      if ui.button("Commit").clicked() {
        self.output = match commit() {
          Ok(msg) => msg,
          Err(err) => err,
        };
      }

      if ui.button("Verify").clicked() {
        self.output = match verify() {
          Ok(msg) => msg,
          Err(err) => err,
        };
      }

      if ui.button("Status").clicked() {
        self.output = match status() {
          Ok(msg) => msg,
          Err(err) => err,
        };
      }

      if ui.button("Generate PDF").clicked() {
        self.output = match report_generate() {
          Ok(msg) => msg,
          Err(err) => err,
        };
      }

      ui.separator();
      ui.label("Output:");
      ui.monospace(&self.output);
    });
  }
}
