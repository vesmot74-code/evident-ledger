//! Shared integration test environment setup for `AppConfig::from_env()`.

use std::sync::Arc;

use evident_ledger::config::AppConfig;
use evident_ledger::paddle::client::MockPaddleClient;
use evident_ledger::state::AppState;

pub const TEST_PADDLE_WEBHOOK_SECRET: &str = "test-paddle-webhook-secret";
pub const TEST_PADDLE_API_KEY: &str = "test-paddle-api-key";

/// Sets mandatory environment variables required by `AppConfig::from_env()`.
pub fn setup_test_env() {
    // Integration tests run in a separate crate; env must be set before each `from_env()` call.
    unsafe {
        std::env::set_var("PADDLE_WEBHOOK_SECRET", TEST_PADDLE_WEBHOOK_SECRET);
        std::env::set_var("PADDLE_API_KEY", TEST_PADDLE_API_KEY);
    }
}

pub fn test_app_state(pool: sqlx::PgPool) -> AppState {
    setup_test_env();
    AppState::with_paddle(
        pool,
        Arc::new(evident_ledger::signing::ServerSigner::load_or_create(
            "signing_key.bin",
        )),
        AppConfig::from_env(),
        MockPaddleClient::new(),
    )
}
