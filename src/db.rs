use sqlx::{PgPool, postgres::PgPoolOptions};
use std::env;
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub async fn create_pool() -> PgPool {
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("DB connection failed")
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub event_id: Uuid,
    pub parent_event_id: Uuid,
    pub file_hash: String,
    pub created_at: DateTime<Utc>,
    pub sequence: i64,
}
