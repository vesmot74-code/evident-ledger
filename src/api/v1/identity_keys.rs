//! GET /v1/identity/keys — read-only identity key listing (Stage 9.5).

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;
use super::identity_key_events::list_key_events_handler;
use super::identity_key_revoke::revoke_key_handler;
use crate::service::identity_dashboard::{IdentityDashboardError, IdentityDashboardService};

#[derive(Debug, Serialize)]
pub struct IdentityKeysListResponse {
    pub keys: Vec<IdentityKeyListItem>,
}

#[derive(Debug, Serialize)]
pub struct IdentityKeyListItem {
    pub key_id: Uuid,
    pub fingerprint: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub verified_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub events_count: i64,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(list_keys_handler))
        .route("/:key_id/events", get(list_key_events_handler))
        .route("/:key_id/revoke", post(revoke_key_handler))
        .with_state(state)
}

async fn list_keys_handler(
    State(state): State<AppState>,
    auth: V1Auth,
) -> Result<Json<IdentityKeysListResponse>, ApiError> {
    let keys = IdentityDashboardService::list_keys(&state.db, auth.0.account_id)
        .await
        .map_err(map_dashboard_error)?;

    Ok(Json(IdentityKeysListResponse {
        keys: keys
            .into_iter()
            .map(|key| IdentityKeyListItem {
                key_id: key.key_id,
                fingerprint: key.fingerprint,
                status: key.status,
                created_at: key.created_at,
                verified_at: key.verified_at,
                revoked_at: key.revoked_at,
                events_count: key.events_count,
            })
            .collect(),
    }))
}

pub(crate) fn map_dashboard_error(err: IdentityDashboardError) -> ApiError {
    match err {
        IdentityDashboardError::KeyNotFound => ApiError::NotFound,
        IdentityDashboardError::InvalidCursor => ApiError::InvalidRequest,
        IdentityDashboardError::Database(_) | IdentityDashboardError::Verification(_) => {
            ApiError::Internal
        }
    }
}
