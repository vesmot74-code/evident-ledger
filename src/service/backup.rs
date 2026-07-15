use std::path::PathBuf;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use tokio::fs;
use uuid::Uuid;

use crate::service::capabilities::AccountCapabilities;
use crate::service::entitlements::{allowed, Feature};

#[derive(Debug)]
pub enum BackupError {
    NotFound,
    FeatureNotAvailable { plan: String },
    Io(std::io::Error),
    Database(sqlx::Error),
    Serialize(serde_json::Error),
}

impl From<sqlx::Error> for BackupError {
    fn from(err: sqlx::Error) -> Self {
        BackupError::Database(err)
    }
}

impl From<std::io::Error> for BackupError {
    fn from(err: std::io::Error) -> Self {
        BackupError::Io(err)
    }
}

impl From<serde_json::Error> for BackupError {
    fn from(err: serde_json::Error) -> Self {
        BackupError::Serialize(err)
    }
}

impl IntoResponse for BackupError {
    fn into_response(self) -> Response {
        match self {
            BackupError::NotFound => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "not_found" })),
            )
                .into_response(),
            BackupError::FeatureNotAvailable { plan } => (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "feature_not_available",
                    "feature": "server_backup",
                    "plan": plan.to_uppercase(),
                })),
            )
                .into_response(),
            BackupError::Io(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("I/O error: {err}") })),
            )
                .into_response(),
            BackupError::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "database error" })),
            )
                .into_response(),
            BackupError::Serialize(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("serialization error: {err}") })),
            )
                .into_response(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BackupResult {
    pub backup_id: Uuid,
    pub event_count: i32,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct EventSnapshot {
    event_id: Uuid,
    chain_id: Uuid,
    parent_event_id: Uuid,
    file_hash: String,
    idempotency_key: String,
    signature: String,
    created_at: DateTime<Utc>,
    sequence: i64,
}

#[derive(Debug, Serialize)]
struct BackupSnapshot {
    chain_id: Uuid,
    events: Vec<EventSnapshot>,
    exported_at: DateTime<Utc>,
}

fn backup_root_dir() -> PathBuf {
    std::env::var("EVIDENT_BACKUP_DIR")
        .unwrap_or_else(|_| "./data/backups".into())
        .into()
}

pub async fn create_backup(
    pool: &PgPool,
    account_id: Uuid,
    chain_id: Uuid,
    capabilities: &AccountCapabilities,
) -> Result<BackupResult, BackupError> {
    let chain = sqlx::query!(
        r#"
        SELECT chain_id
        FROM chains
        WHERE chain_id = $1 AND account_id = $2
        "#,
        chain_id,
        account_id
    )
    .fetch_optional(pool)
    .await?;

    if chain.is_none() {
        return Err(BackupError::NotFound);
    }

    if !allowed(capabilities, Feature::ServerBackup) {
        return Err(BackupError::FeatureNotAvailable {
            plan: capabilities.plan_name.clone(),
        });
    }

    let events = sqlx::query_as!(
        EventSnapshot,
        r#"
        SELECT
            event_id,
            chain_id,
            parent_event_id,
            file_hash,
            idempotency_key,
            signature,
            created_at,
            sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await?;

    let event_count = i32::try_from(events.len()).map_err(|_| {
        BackupError::Database(sqlx::Error::Protocol(
            "event count exceeds i32::MAX".into(),
        ))
    })?;

    let backup_id = Uuid::new_v4();
    let snapshot = BackupSnapshot {
        chain_id,
        events,
        exported_at: Utc::now(),
    };

    let storage_path = backup_root_dir()
        .join(account_id.to_string())
        .join(format!("{backup_id}.json"));

    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let json_bytes = serde_json::to_vec_pretty(&snapshot)?;
    fs::write(&storage_path, json_bytes).await?;

    let storage_path_str = storage_path.to_string_lossy().into_owned();

    sqlx::query!(
        r#"
        INSERT INTO backups (backup_id, chain_id, account_id, storage_path, event_count)
        VALUES ($1, $2, $3, $4, $5)
        "#,
        backup_id,
        chain_id,
        account_id,
        storage_path_str,
        event_count
    )
    .execute(pool)
    .await?;

    Ok(BackupResult {
        backup_id,
        event_count,
    })
}
