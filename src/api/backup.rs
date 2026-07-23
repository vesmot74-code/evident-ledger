use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::{api_key_auth_middleware, AuthedAccount};
use crate::middleware::subscription_enforcement::subscription_enforcement_middleware;
use crate::service::backup::{
    create_backup, get_backup_info, list_backups, read_backup_file, BackupError,
};
use crate::service::capabilities::get_account_capabilities;
use crate::state::AppState;

#[derive(Deserialize)]
struct CreateBackupRequest {
    chain_id: Uuid,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/create", post(create_handler))
        .route("/list", get(list_handler))
        .route("/:backup_id/download", get(download_handler))
        .route("/:backup_id", get(info_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            subscription_enforcement_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_key_auth_middleware,
        ))
        .with_state(state)
}

async fn create_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(req): Json<CreateBackupRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), BackupError> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id).await?;

    let result = create_backup(&state.db, auth.account_id, req.chain_id, &capabilities).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "backup_id": result.backup_id,
            "status": "created",
            "event_count": result.event_count,
        })),
    ))
}

async fn list_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, BackupError> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id).await?;
    let backups = list_backups(&state.db, auth.account_id, &capabilities).await?;
    Ok(Json(
        serde_json::to_value(backups).map_err(BackupError::Serialize)?,
    ))
}

async fn info_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Path(backup_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, BackupError> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id).await?;
    let info = get_backup_info(&state.db, auth.account_id, backup_id, &capabilities).await?;
    Ok(Json(
        serde_json::to_value(info).map_err(BackupError::Serialize)?,
    ))
}

async fn download_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Path(backup_id): Path<Uuid>,
) -> Result<Response, BackupError> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id).await?;
    let bytes = read_backup_file(&state.db, auth.account_id, backup_id, &capabilities).await?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    Ok((StatusCode::OK, headers, bytes).into_response())
}
