use crate::models::event::SubmitEventRequest;
use crate::service::ledger::{submit_event, LedgerError};
use crate::state::AppState;
use axum::{
    extract::{Json, State},
    routing::post,
    Router,
};

pub fn router(state: AppState) -> Router {
    Router::new().route("/", post(handler)).with_state(state)
}

async fn handler(
    State(state): State<AppState>,
    Json(req): Json<SubmitEventRequest>,
) -> Result<Json<serde_json::Value>, LedgerError> {
    let res = submit_event(&state.db, state.signer.as_ref(), req).await?;
    Ok(Json(res))
}
