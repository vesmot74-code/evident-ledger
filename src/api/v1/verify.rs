use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::merkle::MerkleTree;
use crate::service::identity_verification::IdentityVerificationService;
use crate::state::AppState;

use super::auth::V1Auth;
use super::chain_verification::verify_chain_prefix;
use super::errors::ApiError;
use super::event_access::verify_event_access;
use super::file_verification::{normalize_query_file_hash, verify_file_hash};
use super::proof_material::{build_proof_snapshot_read, load_event_prefix};
use super::proof_state::resolve_proof_state;
use super::proof_status::ProofStatus;

#[derive(Debug, Deserialize)]
struct VerifyQuery {
    file_hash: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/:event_id", get(handler))
        .with_state(state)
}

/// `GET /v1/verify/{event_id}` — ownership, query validation, proof gating, chain + file.
async fn handler(
    State(state): State<AppState>,
    auth: V1Auth,
    Path(event_id): Path<Uuid>,
    Query(query): Query<VerifyQuery>,
) -> Result<Json<Value>, ApiError> {
    // 1. X-API-KEY via V1Auth extractor (above)
    // 2. Ownership before any query validation (anti-leak).
    let event = verify_event_access(&state.db, auth.0.account_id, event_id).await?;
    let request_id = ApiError::request_id();

    // 3. Validate optional file_hash query before proof_status gating.
    let provided_file_hash =
        normalize_query_file_hash(query.file_hash).map_err(|_| ApiError::InvalidVerifyFileHash)?;

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

    // 4. Proof status gating.
    let resolved = resolve_proof_state(&state.db, event.chain_id, &event, &snapshot).await?;

    match resolved.status {
        ProofStatus::Pending => Err(ApiError::ProofNotReady),
        ProofStatus::Failed => Err(ApiError::ProofGenerationFailed),
        ProofStatus::Anchored => {
            // 5. Chain verification (Stage 5.3).
            let chain = verify_chain_prefix(
                event.chain_id,
                event.event_id,
                &event.signature,
                &public_key,
                &prefix,
                &resolved.resolved_root,
            );

            // 6. File hash claim verification (Stage 5.4). stored hash never exposed.
            let file = verify_file_hash(provided_file_hash, &event.file_hash);

            let identity_event = crate::models::event::Event {
                event_id: event.event_id,
                chain_id: event.chain_id,
                parent_event_id: event.parent_event_id,
                file_hash: event.file_hash.clone(),
                sequence: event.sequence,
                identity_key_id: event.identity_key_id,
                identity_signature: event.identity_signature.clone(),
                identity_fingerprint: event.identity_fingerprint.clone(),
            };
            let canonical_event_hash_hex = MerkleTree::build_leaf(
                event.sequence,
                &event.event_id,
                &event.parent_event_id,
                &event.file_hash,
            );
            let identity_verification = IdentityVerificationService::verify(
                &state.db,
                &identity_event,
                &canonical_event_hash_hex,
            )
            .await
            .map_err(|_| ApiError::Internal)?;

            let identity_signature = if identity_verification.present {
                Some(json!({
                    "present": true,
                    "valid": identity_verification.valid,
                    "reason": identity_verification.reason,
                    "fingerprint": identity_verification.fingerprint,
                    "key_id": identity_verification.key_id,
                }))
            } else {
                None
            };

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
                "file": {
                    "provided": file.provided,
                    "provided_hash": file.provided_hash,
                    "is_valid_file_hash": file.is_valid_file_hash,
                },
                "identity_signature": identity_signature,
                "request_id": request_id,
            })))
        }
    }
}
