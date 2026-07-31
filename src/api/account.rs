use crate::api::v1::errors::ApiError;
use crate::auth::AuthedAccount;
use crate::service::account::{change_dev_plan, get_key_status, get_usage, DevChangePlanError};
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

fn map_internal(err: impl std::fmt::Display, context: &'static str) -> ApiError {
    tracing::error!(error = %err, "{context}");
    ApiError::Internal
}

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
) -> Result<Json<serde_json::Value>, ApiError> {
    let key_status = get_key_status(&state.db, &auth.key_hash)
        .await
        .map_err(|e| map_internal(e, "legacy account key_status failed"))?;
    Ok(Json(serde_json::to_value(key_status).map_err(|e| {
        map_internal(e, "legacy account key_status serialize failed")
    })?))
}

async fn usage_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, ApiError> {
    let usage = get_usage(&state.db, auth.account_id)
        .await
        .map_err(|e| map_internal(e, "legacy account usage failed"))?;
    Ok(Json(
        serde_json::to_value(usage).map_err(|e| map_internal(e, "legacy account usage serialize failed"))?,
    ))
}

async fn capabilities_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, ApiError> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id)
        .await
        .map_err(|e| map_internal(e, "legacy account capabilities failed"))?;
    let mut value = serde_json::to_value(capabilities)
        .map_err(|e| map_internal(e, "legacy account capabilities serialize failed"))?;
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

    Ok(Json(serde_json::to_value(result).map_err(|e| {
        DevAccountApiError::Database(e.to_string())
    })?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;

    #[tokio::test]
    async fn map_internal_response_hides_leaky_details() {
        let leaky = "sqlx::Error Postgres protocol /Users/test/src/foo.rs: panic in /home/ci";
        let response = map_internal(leaky, "test account leak").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&body);
        for needle in [
            "sqlx",
            "Postgres",
            "postgres",
            ".rs:",
            "panic",
            "/Users/",
            "/home/",
        ] {
            assert!(
                !text.contains(needle),
                "response must not contain {needle:?}: {text}"
            );
        }

        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["error"]["code"], "internal_error");
        assert_eq!(json["error"]["message"], "Internal server error");
    }
}
