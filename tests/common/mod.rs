//! Shared integration test environment setup for `AppConfig::from_env()`.

pub const TEST_PADDLE_WEBHOOK_SECRET: &str = "test-paddle-webhook-secret";

/// Sets mandatory environment variables required by `AppConfig::from_env()`.
pub fn setup_test_env() {
    // Integration tests run in a separate crate; env must be set before each `from_env()` call.
    unsafe {
        std::env::set_var("PADDLE_WEBHOOK_SECRET", TEST_PADDLE_WEBHOOK_SECRET);
    }
}
