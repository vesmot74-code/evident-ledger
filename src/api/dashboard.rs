//! Dashboard API contract (Stage 8.3.1a).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::api::v1::errors::{request_id_layer, ApiError};
use crate::auth::api_key;
use crate::middleware::session_auth::{session_auth_middleware, SessionUser};
use crate::service::accounts::{self, RevokeApiKeyError};
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/me", get(me_handler))
        .route("/subscription", get(subscription_handler))
        .route("/usage", get(usage_handler))
        .route(
            "/api-keys",
            get(list_api_keys_handler).post(create_api_key_handler),
        )
        .route("/api-keys/:key_id", delete(revoke_api_key_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_auth_middleware,
        ))
        .layer(middleware::from_fn(request_id_layer))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct DashboardMeResponse {
    account_id: Uuid,
    email: String,
    plan: String,
    plan_display: String,
    subscription_status: String,
    created_at: DateTime<Utc>,
    email_verified: bool,
}

#[derive(Debug, Serialize)]
struct DashboardSubscriptionResponse {
    plan: String,
    plan_display: String,
    subscription_status: String,
    current_period_end: Option<DateTime<Utc>>,
    pending_plan: Option<String>,
    pending_plan_display: Option<String>,
}

#[derive(Debug, Serialize)]
struct DashboardUsageResponse {
    period: String,
    server_commits: i32,
    monthly_limit: Option<i32>,
    percentage: Option<i32>,
}

#[derive(Debug, Serialize)]
struct DashboardApiKeyItem {
    key_id: Uuid,
    prefix: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    is_active: bool,
}

#[derive(Debug, Serialize)]
struct DashboardApiKeyListResponse {
    api_keys: Vec<DashboardApiKeyItem>,
}

#[derive(Debug, Serialize)]
struct DashboardCreateApiKeyResponse {
    api_key: String,
    key_id: Uuid,
    created_at: DateTime<Utc>,
}

async fn me_handler(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<DashboardMeResponse>, ApiError> {
    let profile = accounts::get_dashboard_profile(&state.db, session.account_id)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(DashboardMeResponse {
        account_id: profile.account_id,
        email: profile.email,
        plan: profile.plan_name,
        plan_display: profile.plan_display_name,
        subscription_status: profile.subscription_status,
        created_at: profile.created_at,
        email_verified: profile.email_verified_at.is_some(),
    }))
}

async fn subscription_handler(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<DashboardSubscriptionResponse>, ApiError> {
    let snapshot = accounts::get_subscription_snapshot(&state.db, session.account_id)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(DashboardSubscriptionResponse {
        plan: snapshot.plan_name,
        plan_display: snapshot.plan_display_name,
        subscription_status: snapshot.subscription_status,
        current_period_end: snapshot.current_period_end,
        pending_plan: snapshot.pending_plan_name,
        pending_plan_display: snapshot.pending_plan_display_name,
    }))
}

async fn usage_handler(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<DashboardUsageResponse>, ApiError> {
    let usage = accounts::get_monthly_usage_snapshot(&state.db, session.account_id)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    let period = usage.period_start.format("%Y-%m").to_string();
    let percentage = usage
        .monthly_commits_limit
        .map(|limit| usage.server_commits.saturating_mul(100) / limit.max(1));

    Ok(Json(DashboardUsageResponse {
        period,
        server_commits: usage.server_commits,
        monthly_limit: usage.monthly_commits_limit,
        percentage,
    }))
}

async fn list_api_keys_handler(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<DashboardApiKeyListResponse>, ApiError> {
    let keys = accounts::list_api_keys(&state.db, session.account_id)
        .await
        .map_err(|_| ApiError::Internal)?;

    let api_keys = keys
        .into_iter()
        .map(|record| DashboardApiKeyItem {
            key_id: record.api_key_id,
            prefix: api_key::key_prefix_for_listing(&record.key_prefix),
            created_at: record.created_at,
            last_used_at: None,
            revoked_at: record.revoked_at,
            is_active: record.revoked_at.is_none(),
        })
        .collect();

    Ok(Json(DashboardApiKeyListResponse { api_keys }))
}

async fn create_api_key_handler(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<DashboardCreateApiKeyResponse>, ApiError> {
    let (generated, record) = accounts::create_api_key(&state.db, session.account_id, "dashboard")
        .await
        .map_err(|_| ApiError::Internal)?;

    Ok(Json(DashboardCreateApiKeyResponse {
        api_key: generated.full_key,
        key_id: record.api_key_id,
        created_at: record.created_at,
    }))
}

async fn revoke_api_key_handler(
    State(state): State<AppState>,
    session: SessionUser,
    Path(key_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    match accounts::revoke_api_key(&state.db, session.account_id, key_id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(RevokeApiKeyError::NotFound) => Err(ApiError::NotFound),
        Err(RevokeApiKeyError::LastActiveKey) => Err(ApiError::Conflict),
        Err(RevokeApiKeyError::Database(_)) => Err(ApiError::Internal),
    }
}
