//! GET /auth/me — session-authenticated profile (Stage 8.3.0).

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use sqlx::Row;

use crate::api::v1::errors::ApiError;
use crate::state::AppState;

use super::web_auth::require_web_session;

#[derive(Debug, Serialize)]
pub struct WebMeResponse {
    pub account_id: Uuid,
    pub email: String,
    pub plan: String,
    pub subscription_status: String,
    pub created_at: DateTime<Utc>,
}

pub async fn get_me(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<WebMeResponse>, ApiError> {
    let account_id = require_web_session(&headers, &state.db).await?;
    let row = sqlx::query(
        r#"
        SELECT
            a.email,
            tp.name AS plan_name,
            a.subscription_status,
            a.created_at
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| ApiError::Internal)?
    .ok_or(ApiError::Unauthorized)?;

    Ok(Json(WebMeResponse {
        account_id,
        email: row.try_get("email").map_err(|_| ApiError::Internal)?,
        plan: row.try_get("plan_name").map_err(|_| ApiError::Internal)?,
        subscription_status: row
            .try_get("subscription_status")
            .map_err(|_| ApiError::Internal)?,
        created_at: row.try_get("created_at").map_err(|_| ApiError::Internal)?,
    }))
}
