use axum::{extract::State, Json, routing::get, Router};
use serde_json::{json, Value};
use crate::state::AppState;

async fn get_identity(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "public_key": state.signer.public_key_hex(),
        "algorithm": "ed25519",
    }))
}

pub fn router(state: AppState) -> Router {
Router::new().route("/", get(get_identity)).with_state(state)
}
