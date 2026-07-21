//! Shared integration test environment setup.

use std::sync::Arc;

use evident_ledger::config::AppConfig;
use evident_ledger::paddle::client::MockPaddleClient;
use evident_ledger::state::AppState;
use sqlx::postgres::PgPoolOptions;

pub const TEST_PADDLE_WEBHOOK_SECRET: &str = "test-paddle-webhook-secret";
pub const TEST_PADDLE_API_KEY: &str = "test-paddle-api-key";
pub const TEST_PADDLE_CLIENT_TOKEN: &str = "test_paddle_client_token";

/// Sets mandatory environment variables required by `AppConfig::from_env()`.
pub fn setup_test_env() {
    // Integration tests run in a separate crate; env must be set before each `from_env()` call.
    unsafe {
        std::env::set_var("PADDLE_WEBHOOK_SECRET", TEST_PADDLE_WEBHOOK_SECRET);
        std::env::set_var("PADDLE_API_KEY", TEST_PADDLE_API_KEY);
        std::env::set_var("PADDLE_CLIENT_TOKEN", TEST_PADDLE_CLIENT_TOKEN);
    }
}

/// Load `.env` then return a validated `TEST_DATABASE_URL`.
pub fn test_database_url() -> String {
    dotenvy::dotenv().ok();
    evident_ledger::db::require_test_database_url()
}

/// DB URL shared with a running `evident-ledger` process (live-server tests only).
///
/// Must match the server's `DATABASE_URL`. Do not use for tests that mutate
/// shared catalog rows such as `tariff_plans.paddle_price_id`.
pub fn live_server_database_url() -> String {
    dotenvy::dotenv().ok();
    std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for live-server tests")
}

/// Connect to `ledger_test` and apply migrations.
pub async fn test_pool() -> sqlx::PgPool {
    let database_url = test_database_url();
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("test db");
    sqlx::migrate!().run(&pool).await.expect("migrate");
    pool
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
