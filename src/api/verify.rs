use axum::{Router, routing::get, extract::{State, Path}, Json};
use uuid::Uuid;
use crate::state::AppState;
use crate::service::verification::{verify_chain, export_proof};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/:chain_id", get(handler_verify))
        .with_state(state.clone())
        .merge(
            Router::new()
                .route("/proof/:chain_id", get(handler_proof))
                .with_state(state)
        )
}

async fn handler_verify(
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, String> {
    verify_chain(&state.db, &state.signer, chain_id)
        .await.map(Json).map_err(|e| e.to_string())
}

async fn handler_proof(
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, String> {
    export_proof(&state.db, &state.signer, chain_id)
        .await.map(Json).map_err(|e| e.to_string())
}
