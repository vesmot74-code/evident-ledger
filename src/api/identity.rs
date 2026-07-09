use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};

async fn get_identity(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "public_key": state.signer.public_key_hex(),
        "algorithm": "ed25519",
    }))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(get_identity))
        .with_state(state)
}
