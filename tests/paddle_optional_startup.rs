//! Startup config when Paddle billing is intentionally disabled.

use std::sync::{Mutex, MutexGuard};

use evident_ledger::config::AppConfig;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn clear_paddle_secrets() {
    unsafe {
        std::env::remove_var("PADDLE_WEBHOOK_SECRET");
        std::env::remove_var("PADDLE_API_KEY");
        std::env::remove_var("PADDLE_CLIENT_TOKEN");
    }
}

#[test]
fn startup_succeeds_without_paddle_vars_when_disabled() {
    let _guard = lock_env();

    unsafe {
        std::env::set_var("PADDLE_ENABLED", "false");
        std::env::set_var("ENVIRONMENT", "development");
        // Ensure production signing guard does not fire.
        std::env::remove_var("DEV_MODE");
    }
    clear_paddle_secrets();

    let config = AppConfig::from_env();

    assert!(!config.paddle_enabled);
    assert!(config.paddle_webhook_secret.is_empty());
    assert!(config.paddle_api_key.is_empty());
    assert!(config.paddle_client_token.is_empty());

    unsafe {
        std::env::remove_var("PADDLE_ENABLED");
    }
}

#[test]
fn paddle_enabled_defaults_to_true() {
    let _guard = lock_env();

    unsafe {
        std::env::remove_var("PADDLE_ENABLED");
        std::env::set_var("ENVIRONMENT", "development");
        std::env::set_var("PADDLE_WEBHOOK_SECRET", "test-paddle-webhook-secret");
        std::env::set_var("PADDLE_API_KEY", "test-paddle-api-key");
        std::env::set_var("PADDLE_CLIENT_TOKEN", "test_paddle_client_token");
    }

    let config = AppConfig::from_env();
    assert!(config.paddle_enabled);
    assert_eq!(config.paddle_webhook_secret, "test-paddle-webhook-secret");
    assert_eq!(config.paddle_api_key, "test-paddle-api-key");
    assert_eq!(config.paddle_client_token, "test_paddle_client_token");
}
