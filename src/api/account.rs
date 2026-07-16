use crate::auth::AuthedAccount;
use crate::service::account::{
    change_dev_plan, get_key_status, get_usage, DevChangePlanError,
};
use crate::service::capabilities::get_account_capabilities;
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/usage", get(usage_handler))
        .route("/capabilities", get(capabilities_handler))
        .route("/key-status", get(key_status_handler))
        .route("/dev/change-plan", post(dev_change_plan_handler))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct DevChangePlanRequest {
    account_id: Uuid,
    plan: String,
}

#[derive(Debug)]
enum DevAccountApiError {
    NotAllowed,
    AccountMismatch,
    PlanNotFound,
    AccountNotFound,
    Database(String),
}

impl IntoResponse for DevAccountApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            DevAccountApiError::NotAllowed => (
                StatusCode::FORBIDDEN,
                "Dev tools are not available in this environment",
            ),
            DevAccountApiError::AccountMismatch => (
                StatusCode::FORBIDDEN,
                "account_id does not match authenticated account",
            ),
            DevAccountApiError::PlanNotFound => (StatusCode::BAD_REQUEST, "Unknown tariff plan"),
            DevAccountApiError::AccountNotFound => (StatusCode::NOT_FOUND, "Account not found"),
            DevAccountApiError::Database(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

async fn key_status_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, String> {
    let key_status = get_key_status(&state.db, &auth.key_hash)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Json(
        serde_json::to_value(key_status).map_err(|e| e.to_string())?,
    ))
}

async fn usage_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, String> {
    let usage = get_usage(&state.db, auth.account_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Json(
        serde_json::to_value(usage).map_err(|e| e.to_string())?,
    ))
}

async fn capabilities_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, String> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut value = serde_json::to_value(capabilities).map_err(|e| e.to_string())?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("account_id".into(), json!(auth.account_id));
        obj.insert("dev_tools_available".into(), json!(state.config.dev_mode));
    }
    Ok(Json(value))
}

async fn dev_change_plan_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(req): Json<DevChangePlanRequest>,
) -> Result<Json<serde_json::Value>, DevAccountApiError> {
    if !state.config.dev_mode {
        return Err(DevAccountApiError::NotAllowed);
    }

    if req.account_id != auth.account_id {
        return Err(DevAccountApiError::AccountMismatch);
    }

    let plan = req.plan.to_lowercase();
    let result = change_dev_plan(&state.db, req.account_id, &plan)
        .await
        .map_err(|e| match e {
            DevChangePlanError::PlanNotFound => DevAccountApiError::PlanNotFound,
            DevChangePlanError::AccountNotFound => DevAccountApiError::AccountNotFound,
            DevChangePlanError::Database(err) => DevAccountApiError::Database(err.to_string()),
        })?;

    Ok(Json(
        serde_json::to_value(result).map_err(|e| DevAccountApiError::Database(e.to_string()))?,
    ))
}
