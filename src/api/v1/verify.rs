use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde_json::json;
use uuid::Uuid;

use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;
use super::event_access::verify_event_access;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/:event_id", get(handler))
        .with_state(state)
}

async fn handler(
    State(state): State<AppState>,
    auth: V1Auth,
    Path(event_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let event = verify_event_access(&state.db, auth.0.account_id, event_id).await?;
    Ok(Json(json!({ "event_id": event.event_id })))
}
