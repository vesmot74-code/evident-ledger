use crate::auth::AuthedAccount;
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
    auth: AuthedAccount,
    Json(req): Json<SubmitEventRequest>,
) -> Result<Json<serde_json::Value>, LedgerError> {
    Ok(Json(
        submit_event(&state.db, state.signer.as_ref(), auth.account_id, req).await?,
    ))
}
