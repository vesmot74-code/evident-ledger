//! GET /v1/me — identity for desktop Bearer tokens and API keys (Stage 13.4).

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::service::accounts;
use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub account_id: Uuid,
    pub email: String,
    pub plan: String,
    pub plan_display: String,
    pub subscription_status: String,
    pub created_at: DateTime<Utc>,
}

pub async fn me_handler(
    State(state): State<AppState>,
    auth: V1Auth,
) -> Result<Json<MeResponse>, ApiError> {
    let profile = accounts::get_dashboard_profile(&state.db, auth.0.account_id)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(MeResponse {
        account_id: profile.account_id,
        email: profile.email,
        plan: profile.plan_name,
        plan_display: profile.plan_display_name,
        subscription_status: profile.subscription_status,
        created_at: profile.created_at,
    }))
}
