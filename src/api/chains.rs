use crate::auth::AuthedAccount;
use crate::service::chains::create_chain;
use crate::state::AppState;
use axum::{extract::State, routing::post, Json, Router};

pub fn router(state: AppState) -> Router {
    Router::new().route("/", post(handler)).with_state(state)
}

async fn handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, String> {
    create_chain(&state.db, auth.account_id)
        .await
        .map(Json)
        .map_err(|e| e.to_string())
}
