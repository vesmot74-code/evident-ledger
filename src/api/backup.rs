use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::AuthedAccount;
use crate::service::backup::{create_backup, BackupError};
use crate::service::capabilities::get_account_capabilities;
use crate::state::AppState;

#[derive(Deserialize)]
struct CreateBackupRequest {
    chain_id: Uuid,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/create", post(handler))
        .with_state(state)
}

async fn handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(req): Json<CreateBackupRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), BackupError> {
    let capabilities = get_account_capabilities(&state.db, auth.account_id).await?;

    let result = create_backup(
        &state.db,
        auth.account_id,
        req.chain_id,
        &capabilities,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "backup_id": result.backup_id,
            "status": "created",
            "event_count": result.event_count,
        })),
    ))
}
