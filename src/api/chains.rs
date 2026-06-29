use axum::{Router, routing::post, extract::State, Json};
use crate::state::AppState;
use crate::service::chains::create_chain;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", post(handler))
        .with_state(state)
}

async fn handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, String> {
    create_chain(&state.db)
        .await
        .map(Json)
        .map_err(|e| e.to_string())
}
