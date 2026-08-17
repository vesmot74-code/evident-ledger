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

use crate::service::backup_snapshot::{BackupSnapshot, EventSnapshot};
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
            BackupError::NotFound => {
                (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
            }
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
pub struct BackupListItem {
    pub backup_id: Uuid,
    pub chain_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub event_count: i32,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BackupInfo {
    pub backup_id: Uuid,
    pub chain_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub event_count: i32,
    pub storage_path: String,
}

#[derive(Debug, sqlx::FromRow)]
struct BackupRow {
    backup_id: Uuid,
    chain_id: Uuid,
    created_at: DateTime<Utc>,
    event_count: i32,
    storage_path: String,
}

fn backup_root_dir() -> PathBuf {
    std::env::var("EVIDENT_BACKUP_DIR")
        .unwrap_or_else(|_| "./data/backups".into())
        .into()
}

pub(crate) fn ensure_server_backup_allowed(
    capabilities: &AccountCapabilities,
) -> Result<(), BackupError> {
    if !allowed(capabilities, Feature::ServerBackup) {
        return Err(BackupError::FeatureNotAvailable {
            plan: capabilities.plan_name.clone(),
        });
    }
    Ok(())
}

fn log_backup_op(op: &str, account_id: Uuid, backup_id: Option<Uuid>, detail: &str) {
    match backup_id {
        Some(id) => println!("backup {op} account_id={account_id} backup_id={id}{detail}"),
        None => println!("backup {op} account_id={account_id}{detail}"),
    }
}

/// Ownership-checked event fetch shared by Local Export and Server Backup.
async fn fetch_chain_events(
    pool: &PgPool,
    account_id: Uuid,
    chain_id: Uuid,
) -> Result<Vec<EventSnapshot>, BackupError> {
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

    Ok(events)
}

async fn fetch_owned_backup(
    pool: &PgPool,
    account_id: Uuid,
    backup_id: Uuid,
) -> Result<BackupRow, BackupError> {
    let row = sqlx::query_as!(
        BackupRow,
        r#"
        SELECT
            backup_id,
            chain_id,
            created_at,
            event_count,
            storage_path
        FROM backups
        WHERE backup_id = $1 AND account_id = $2
        "#,
        backup_id,
        account_id
    )
    .fetch_optional(pool)
    .await?;

    row.ok_or(BackupError::NotFound)
}

/// Local Export: return a chain snapshot without server-side persistence.
/// Available on all plans (no `Feature::ServerBackup` gate).
pub async fn export_local_snapshot(
    pool: &PgPool,
    account_id: Uuid,
    chain_id: Uuid,
) -> Result<BackupSnapshot, BackupError> {
    let events = fetch_chain_events(pool, account_id, chain_id).await?;
    Ok(BackupSnapshot {
        chain_id,
        events,
        exported_at: Utc::now(),
    })
}

pub async fn create_backup(
    pool: &PgPool,
    account_id: Uuid,
    chain_id: Uuid,
    capabilities: &AccountCapabilities,
) -> Result<BackupResult, BackupError> {
    // Ownership first (NotFound), then Server Backup entitlement.
    let events = fetch_chain_events(pool, account_id, chain_id).await?;
    ensure_server_backup_allowed(capabilities)?;

    let event_count = i32::try_from(events.len()).map_err(|_| {
        BackupError::Database(sqlx::Error::Protocol("event count exceeds i32::MAX".into()))
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

    log_backup_op(
        "create",
        account_id,
        Some(backup_id),
        &format!(" event_count={event_count}"),
    );

    Ok(BackupResult {
        backup_id,
        event_count,
    })
}

pub async fn list_backups(
    pool: &PgPool,
    account_id: Uuid,
    capabilities: &AccountCapabilities,
) -> Result<Vec<BackupListItem>, BackupError> {
    ensure_server_backup_allowed(capabilities)?;

    let backups = sqlx::query_as!(
        BackupListItem,
        r#"
        SELECT backup_id, chain_id, created_at, event_count
        FROM backups
        WHERE account_id = $1
        ORDER BY created_at DESC
        "#,
        account_id
    )
    .fetch_all(pool)
    .await?;

    log_backup_op(
        "list",
        account_id,
        None,
        &format!(" count={}", backups.len()),
    );

    Ok(backups)
}

pub async fn get_backup_info(
    pool: &PgPool,
    account_id: Uuid,
    backup_id: Uuid,
    capabilities: &AccountCapabilities,
) -> Result<BackupInfo, BackupError> {
    ensure_server_backup_allowed(capabilities)?;

    let row = fetch_owned_backup(pool, account_id, backup_id).await?;

    log_backup_op("info", account_id, Some(backup_id), "");

    Ok(BackupInfo {
        backup_id: row.backup_id,
        chain_id: row.chain_id,
        created_at: row.created_at,
        event_count: row.event_count,
        storage_path: row.storage_path,
    })
}

pub async fn read_backup_file(
    pool: &PgPool,
    account_id: Uuid,
    backup_id: Uuid,
    capabilities: &AccountCapabilities,
) -> Result<Vec<u8>, BackupError> {
    ensure_server_backup_allowed(capabilities)?;

    let row = fetch_owned_backup(pool, account_id, backup_id).await?;
    let bytes = fs::read(&row.storage_path).await?;

    log_backup_op(
        "download",
        account_id,
        Some(backup_id),
        &format!(" bytes={}", bytes.len()),
    );

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::backup_snapshot::{parse_snapshot, validate_structural_integrity};
    use crate::service::capabilities::{AccountCapabilities, TsaMode};
    use sqlx::postgres::PgPoolOptions;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn capabilities_with_server_backup(enabled: bool) -> AccountCapabilities {
        AccountCapabilities {
            plan_name: if enabled {
                "vault".into()
            } else {
                "free".into()
            },
            tsa_mode: TsaMode::Machine,
            server_backup: enabled,
            history_recovery: false,
            identity_enabled: false,
            monthly_commits_limit: Some(100),
            monthly_tsa_limit: Some(100),
        }
    }

    async fn test_pool() -> PgPool {
        dotenvy::dotenv().ok();
        let database_url = crate::db::require_test_database_url();
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("test db connection failed")
    }

    struct OwnershipFixture {
        account_a: Uuid,
        account_b: Uuid,
        backup_a: Uuid,
        backup_b: Uuid,
    }

    impl OwnershipFixture {
        async fn setup(pool: &PgPool) -> Self {
            let account_a = Uuid::new_v4();
            let account_b = Uuid::new_v4();
            let chain_a = Uuid::new_v4();
            let chain_b = Uuid::new_v4();
            let backup_a = Uuid::new_v4();
            let backup_b = Uuid::new_v4();
            let vault_plan_id: Uuid =
                sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = 'vault'")
                    .fetch_one(pool)
                    .await
                    .expect("vault tariff plan");

            sqlx::query(
                "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)",
            )
            .bind(account_a)
            .bind(format!("restore-test-a-{account_a}@test.local"))
            .bind(vault_plan_id)
            .execute(pool)
            .await
            .expect("insert account_a");

            sqlx::query(
                "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)",
            )
            .bind(account_b)
            .bind(format!("restore-test-b-{account_b}@test.local"))
            .bind(vault_plan_id)
            .execute(pool)
            .await
            .expect("insert account_b");

            sqlx::query("INSERT INTO chains (chain_id, account_id) VALUES ($1, $2)")
                .bind(chain_a)
                .bind(account_a)
                .execute(pool)
                .await
                .expect("insert chain_a");

            sqlx::query("INSERT INTO chains (chain_id, account_id) VALUES ($1, $2)")
                .bind(chain_b)
                .bind(account_b)
                .execute(pool)
                .await
                .expect("insert chain_b");

            sqlx::query(
                "INSERT INTO backups (backup_id, chain_id, account_id, storage_path, event_count) VALUES ($1, $2, $3, $4, 1)",
            )
            .bind(backup_a)
            .bind(chain_a)
            .bind(account_a)
            .bind(format!("/tmp/restore-test-{backup_a}.json"))
            .execute(pool)
            .await
            .expect("insert backup_a");

            sqlx::query(
                "INSERT INTO backups (backup_id, chain_id, account_id, storage_path, event_count) VALUES ($1, $2, $3, $4, 2)",
            )
            .bind(backup_b)
            .bind(chain_b)
            .bind(account_b)
            .bind(format!("/tmp/restore-test-{backup_b}.json"))
            .execute(pool)
            .await
            .expect("insert backup_b");

            Self {
                account_a,
                account_b,
                backup_a,
                backup_b,
            }
        }

        async fn teardown(self, pool: &PgPool) {
            for account_id in [self.account_a, self.account_b] {
                let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
                    .bind(account_id)
                    .execute(pool)
                    .await;
            }
        }
    }

    struct ExportFixture {
        account_id: Uuid,
        foreign_account_id: Uuid,
        chain_id: Uuid,
        foreign_chain_id: Uuid,
    }

    impl ExportFixture {
        async fn setup(pool: &PgPool, plan_name: &str) -> Self {
            let account_id = Uuid::new_v4();
            let foreign_account_id = Uuid::new_v4();
            let chain_id = Uuid::new_v4();
            let foreign_chain_id = Uuid::new_v4();
            let event_id = Uuid::new_v4();
            let plan_id: Uuid =
                sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = $1")
                    .bind(plan_name)
                    .fetch_one(pool)
                    .await
                    .expect("tariff plan");

            sqlx::query(
                "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)",
            )
            .bind(account_id)
            .bind(format!("export-test-{account_id}@test.local"))
            .bind(plan_id)
            .execute(pool)
            .await
            .expect("insert account");

            sqlx::query(
                "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)",
            )
            .bind(foreign_account_id)
            .bind(format!("export-foreign-{foreign_account_id}@test.local"))
            .bind(plan_id)
            .execute(pool)
            .await
            .expect("insert foreign account");

            sqlx::query("INSERT INTO chains (chain_id, account_id) VALUES ($1, $2)")
                .bind(chain_id)
                .bind(account_id)
                .execute(pool)
                .await
                .expect("insert chain");

            sqlx::query("INSERT INTO chains (chain_id, account_id) VALUES ($1, $2)")
                .bind(foreign_chain_id)
                .bind(foreign_account_id)
                .execute(pool)
                .await
                .expect("insert foreign chain");

            sqlx::query(
                r#"
                INSERT INTO events (
                    event_id, chain_id, parent_event_id, file_hash,
                    idempotency_key, signature, sequence
                ) VALUES ($1, $2, $3, $4, $5, $6, 1)
                "#,
            )
            .bind(event_id)
            .bind(chain_id)
            .bind(Uuid::nil())
            .bind("aa".repeat(32))
            .bind(format!("export-idem-{event_id}"))
            .bind("")
            .execute(pool)
            .await
            .expect("insert event");

            Self {
                account_id,
                foreign_account_id,
                chain_id,
                foreign_chain_id,
            }
        }

        async fn teardown(self, pool: &PgPool) {
            let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
                .bind(self.account_id)
                .execute(pool)
                .await;
            let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
                .bind(self.foreign_account_id)
                .execute(pool)
                .await;
        }
    }

    #[test]
    fn server_backup_disabled_returns_feature_not_available() {
        let err =
            ensure_server_backup_allowed(&capabilities_with_server_backup(false)).unwrap_err();
        assert!(matches!(
            err,
            BackupError::FeatureNotAvailable { plan } if plan == "free"
        ));
    }

    #[test]
    fn server_backup_enabled_passes_entitlement_check() {
        ensure_server_backup_allowed(&capabilities_with_server_backup(true)).unwrap();
    }

    #[test]
    fn read_backup_bytes_matches_file_on_disk() {
        let mut file = NamedTempFile::new().unwrap();
        let payload = br#"{"chain_id":"00000000-0000-0000-0000-000000000001","events":[],"exported_at":"2026-07-15T00:00:00Z"}"#;
        file.write_all(payload).unwrap();
        file.flush().unwrap();

        let read = std::fs::read(file.path()).unwrap();
        assert_eq!(read, payload);
    }

    #[tokio::test]
    async fn list_backups_returns_only_owned_backups() {
        let pool = test_pool().await;
        let fixture = OwnershipFixture::setup(&pool).await;
        let caps = capabilities_with_server_backup(true);

        let backups = list_backups(&pool, fixture.account_a, &caps)
            .await
            .expect("list backups for account_a");

        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].backup_id, fixture.backup_a);
        assert_ne!(backups[0].backup_id, fixture.backup_b);

        fixture.teardown(&pool).await;
    }

    #[tokio::test]
    async fn foreign_backup_returns_not_found_for_info_and_download() {
        let pool = test_pool().await;
        let fixture = OwnershipFixture::setup(&pool).await;
        let caps = capabilities_with_server_backup(true);

        let info_err = get_backup_info(&pool, fixture.account_b, fixture.backup_a, &caps)
            .await
            .unwrap_err();
        assert!(matches!(info_err, BackupError::NotFound));

        let download_err = read_backup_file(&pool, fixture.account_b, fixture.backup_a, &caps)
            .await
            .unwrap_err();
        assert!(matches!(download_err, BackupError::NotFound));

        fixture.teardown(&pool).await;
    }

    #[tokio::test]
    async fn free_account_can_export_local_snapshot() {
        let pool = test_pool().await;
        let fixture = ExportFixture::setup(&pool, "free").await;
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("EVIDENT_BACKUP_DIR", tmp.path());
        }

        let before_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM backups WHERE account_id = $1")
                .bind(fixture.account_id)
                .fetch_one(&pool)
                .await
                .expect("count");

        let snapshot = export_local_snapshot(&pool, fixture.account_id, fixture.chain_id)
            .await
            .expect("free export must succeed");

        assert_eq!(snapshot.chain_id, fixture.chain_id);
        assert_eq!(snapshot.events.len(), 1);

        let after_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM backups WHERE account_id = $1")
                .bind(fixture.account_id)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(after_rows, before_rows, "export must not INSERT into backups");

        let account_dir = tmp.path().join(fixture.account_id.to_string());
        assert!(
            !account_dir.exists()
                || std::fs::read_dir(&account_dir)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(true),
            "export must not write under backup_root_dir"
        );

        let bytes = serde_json::to_vec(&snapshot).expect("serialize");
        let parsed = parse_snapshot(&bytes).expect("parse");
        validate_structural_integrity(&parsed).expect("structural");

        fixture.teardown(&pool).await;
    }

    #[tokio::test]
    async fn export_foreign_chain_returns_not_found() {
        let pool = test_pool().await;
        let fixture = ExportFixture::setup(&pool, "free").await;

        let err = export_local_snapshot(&pool, fixture.account_id, fixture.foreign_chain_id)
            .await
            .unwrap_err();
        assert!(matches!(err, BackupError::NotFound));

        fixture.teardown(&pool).await;
    }

    #[tokio::test]
    async fn create_backup_still_requires_server_backup_feature() {
        let pool = test_pool().await;
        let fixture = ExportFixture::setup(&pool, "free").await;
        let caps = capabilities_with_server_backup(false);
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("EVIDENT_BACKUP_DIR", tmp.path());
        }

        let err = create_backup(&pool, fixture.account_id, fixture.chain_id, &caps)
            .await
            .unwrap_err();
        assert!(matches!(err, BackupError::FeatureNotAvailable { .. }));

        fixture.teardown(&pool).await;
    }

    #[tokio::test]
    async fn create_backup_foreign_chain_returns_not_found_regardless_of_plan() {
        let pool = test_pool().await;
        let fixture = ExportFixture::setup(&pool, "free").await;
        let caps = capabilities_with_server_backup(false);
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("EVIDENT_BACKUP_DIR", tmp.path());
        }

        let err = create_backup(&pool, fixture.account_id, fixture.foreign_chain_id, &caps)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackupError::NotFound),
            "Free + foreign chain must be NotFound, not FeatureNotAvailable: {err:?}"
        );

        fixture.teardown(&pool).await;
    }
}
