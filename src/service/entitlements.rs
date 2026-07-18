use sqlx::PgPool;
use uuid::Uuid;

use crate::service::capabilities::{get_account_capabilities, AccountCapabilities};

pub enum Feature {
    Tsa,
    ServerBackup,
    Identity,
    HistoryRecovery,
}

#[derive(Debug)]
pub enum EntitlementError {
    Missing,
    Database(sqlx::Error),
}

pub fn allowed(capabilities: &AccountCapabilities, feature: Feature) -> bool {
    match feature {
        Feature::Tsa => capabilities.tsa_available(),
        Feature::ServerBackup => capabilities.server_backup,
        Feature::Identity => capabilities.identity_enabled,
        Feature::HistoryRecovery => capabilities.history_recovery,
    }
}

pub async fn require_feature(
    pool: &PgPool,
    account_id: Uuid,
    feature: Feature,
) -> Result<(), EntitlementError> {
    let caps = get_account_capabilities(pool, account_id)
        .await
        .map_err(EntitlementError::Database)?;
    if allowed(&caps, feature) {
        Ok(())
    } else {
        Err(EntitlementError::Missing)
    }
}
