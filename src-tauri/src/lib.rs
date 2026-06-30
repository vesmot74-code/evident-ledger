#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[tauri::command]
fn commit() -> Result<String, String> {
  use std::process::Command;

  let output = Command::new("evident")
      .arg("commit")
      .output()
      .expect("failed to execute process");

  if output.status.success() {
      Ok("Commit successful".into())
  } else {
      Err("Commit failed".into())
  }
}

#[tauri::command]
fn verify() -> Result<String, String> {
  use std::process::Command;

  let output = Command::new("evident")
      .arg("verify")
      .output()
      .expect("failed to execute process");

  if output.status.success() {
      Ok("VALID".into())
  } else {
      Err("INVALID".into())
  }
}

#[tauri::command]
fn status() -> Result<String, String> {
  use std::process::Command;

  let output = Command::new("evident")
      .arg("status")
      .output()
      .expect("failed to execute process");

  if output.status.success() {
      Ok("Status retrieved".into())
  } else {
      Err("Failed to retrieve status".into())
  }
}

#[tauri::command]
fn report_generate() -> Result<String, String> {
  use std::process::Command;

  let output = Command::new("evident")
      .arg("report")
      .arg("generate")
      .output()
      .expect("failed to execute process");

  if output.status.success() {
      Ok("PDF created".into())
  } else {
      Err("Failed to create PDF".into())
  }
}

pub fn run() {
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![commit, verify, status, report_generate])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
