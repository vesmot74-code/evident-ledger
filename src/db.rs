use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::env;
use uuid::Uuid;

pub async fn create_pool() -> PgPool {
    let database_url = match env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("STARTUP_ERROR database: DATABASE_URL must be set");
            std::process::exit(3);
        }
    };

    PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .unwrap_or_else(|err| {
            eprintln!("STARTUP_ERROR database: connection failed: {}", err);
            std::process::exit(3);
        })
}

/// Resolve `TEST_DATABASE_URL` and refuse connecting to the non-test `ledger` database.
pub fn require_test_database_url() -> String {
    let db_url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set for tests");
    refuse_non_test_database(&db_url);
    db_url
}

/// Panic if `db_url` points at the shared development database (`…/ledger`).
pub fn refuse_non_test_database(db_url: &str) {
    if db_url.contains("/ledger") && !db_url.contains("/ledger_test") {
        panic!("Refusing to run tests against non-test database: {db_url}. Use ledger_test.");
    }
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub event_id: Uuid,
    pub parent_event_id: Uuid,
    pub file_hash: String,
    pub created_at: DateTime<Utc>,
    pub sequence: i64,
}
