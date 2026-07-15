use crate::auth::AuthedAccount;
use crate::service::account::get_usage;
use crate::service::capabilities::get_account_capabilities;
use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/usage", get(usage_handler))
        .route("/capabilities", get(capabilities_handler))
        .with_state(state)
}

async fn usage_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, String> {
    let usage = get_usage(&state.db, auth.account_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Json(serde_json::to_value(usage).map_err(|e| e.to_string())?))
}

async fn capabilities_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, String> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Json(
        serde_json::to_value(capabilities).map_err(|e| e.to_string())?,
    ))
}
