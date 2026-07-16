use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use uuid::Uuid;

use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;
use super::event_access::verify_event_access;
use super::proof_material::build_proof_response;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/:event_id", get(handler))
        .with_state(state)
}

/// Returns proof for the commit-time snapshot of `event_id`.
///
/// Missing proof material → `200 OK` with `proof_status: "pending"`, not `404`.
async fn handler(
    State(state): State<AppState>,
    auth: V1Auth,
    Path(event_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let event = verify_event_access(&state.db, auth.0.account_id, event_id).await?;
    let response =
        build_proof_response(&state.db, state.signer.as_ref(), &event).await?;
    Ok(Json(response))
}
