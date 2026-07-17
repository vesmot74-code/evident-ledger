use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::state::AppState;

use super::auth::V1Auth;
use super::chain_verification::verify_chain_prefix;
use super::errors::ApiError;
use super::event_access::verify_event_access;
use super::proof_material::{build_proof_snapshot_read, load_event_prefix};
use super::proof_state::resolve_proof_state;
use super::proof_status::ProofStatus;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/:event_id", get(handler))
        .with_state(state)
}

/// `GET /v1/verify/{event_id}` — ownership first, then proof status gating (Stage 5.2).
async fn handler(
    State(state): State<AppState>,
    auth: V1Auth,
    Path(event_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let event = verify_event_access(&state.db, auth.0.account_id, event_id).await?;
    let request_id = ApiError::request_id();

    let mut conn = state.db.acquire().await.map_err(|_| ApiError::Internal)?;

    let prefix = load_event_prefix(&mut *conn, event.chain_id, event.sequence)
        .await
        .map_err(|_| ApiError::Internal)?;

    let public_key = state.signer.public_key_hex();
    let snapshot = build_proof_snapshot_read(
        event.chain_id,
        event.event_id,
        &prefix,
        &event.signature,
        &public_key,
    );

    let resolved = resolve_proof_state(&state.db, event.chain_id, &event, &snapshot).await?;

    match resolved.status {
        ProofStatus::Pending => Err(ApiError::ProofNotReady),
        ProofStatus::Failed => Err(ApiError::ProofGenerationFailed),
        ProofStatus::Anchored => {
            let chain = verify_chain_prefix(
                event.chain_id,
                event.event_id,
                &event.signature,
                &public_key,
                &prefix,
                &resolved.resolved_root,
            );
            Ok(Json(json!({
                "event_id": event.event_id,
                "chain_id": event.chain_id,
                "sequence": event.sequence,
                "proof_status": ProofStatus::Anchored.as_str(),
                "chain": {
                    "valid": chain.valid,
                    "merkle_valid": chain.merkle_valid,
                    "signature_valid": chain.signature_valid,
                    "errors": chain.errors,
                },
                "request_id": request_id,
            })))
        }
    }
}
