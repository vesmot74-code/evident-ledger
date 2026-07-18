//! GET /v1/identity/keys/{key_id}/events — read-only signed event history (Stage 9.5).

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::identity_dashboard::{clamp_events_limit, IdentityDashboardService};
use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;
use super::identity_keys::map_dashboard_error;

#[derive(Debug, Deserialize)]
pub struct ListKeyEventsQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IdentityKeyEventsResponse {
    pub key_id: Uuid,
    pub key_status: String,
    pub events: Vec<IdentityKeyEventItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IdentityKeyEventItem {
    pub event_id: Uuid,
    pub chain_id: Uuid,
    pub sequence: i64,
    pub signed_at: DateTime<Utc>,
    pub identity_signature_valid: bool,
}

pub async fn list_key_events_handler(
    State(state): State<AppState>,
    auth: V1Auth,
    Path(key_id): Path<Uuid>,
    Query(query): Query<ListKeyEventsQuery>,
) -> Result<Json<IdentityKeyEventsResponse>, ApiError> {
    let limit = clamp_events_limit(query.limit);
    let page = IdentityDashboardService::list_key_events(
        &state.db,
        auth.0.account_id,
        key_id,
        limit,
        query.cursor.as_deref(),
    )
    .await
    .map_err(map_dashboard_error)?;

    Ok(Json(IdentityKeyEventsResponse {
        key_id: page.key_id,
        key_status: page.key_status,
        events: page
            .events
            .into_iter()
            .map(|event| IdentityKeyEventItem {
                event_id: event.event_id,
                chain_id: event.chain_id,
                sequence: event.sequence,
                signed_at: event.signed_at,
                identity_signature_valid: event.identity_signature_valid,
            })
            .collect(),
        next_cursor: page.next_cursor,
    }))
}
