use crate::sac::SacDocument;
use crate::service::attestation::build_attestation;
use crate::service::verification::{export_proof, verify_chain};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use uuid::Uuid;

pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/:chain_id", get(handler_verify))
        .with_state(state.clone())
        .merge(
            Router::new()
                .route("/proof/:chain_id", get(handler_proof))
                .with_state(state.clone()),
        )
        .merge(
            Router::new()
                .route("/hash", post(handler_verify_hash))
                .with_state(state.clone()),
        )
        .merge(
            Router::new()
                .route("/:chain_id/attestation", get(handler_attestation))
                .with_state(state.clone()),
        )
        .merge(
            Router::new()
                .route("/:chain_id/attestation.pdf", get(handler_attestation_pdf))
                .with_state(state.clone()),
        )
        .merge(
            Router::new()
                .route(
                    "/hash/:hash/attestation.pdf",
                    get(handler_hash_attestation_pdf),
                )
                .with_state(state),
        )
}

async fn handler_verify(
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    verify_chain(&state.db, &state.signer, chain_id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn handler_attestation(
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<Json<SacDocument>, ApiError> {
    build_attestation(&state.db, &state.signer, chain_id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn handler_attestation_pdf(
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let doc = build_attestation(&state.db, &state.signer, chain_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let pdf_bytes = crate::sac_pdf::render_sac_pdf(&doc);

    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/pdf".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"sac-{}.pdf\"", chain_id),
            ),
        ],
        pdf_bytes,
    ))
}

async fn handler_proof(
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    export_proof(&state.db, &state.signer, chain_id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn handler_verify_hash(
    _state: State<AppState>,
    _payload: Json<serde_json::Value>,
) -> impl IntoResponse {
    deprecated_hash_lookup_response()
}

async fn handler_hash_attestation_pdf(
    _state: State<AppState>,
    _path: Path<String>,
) -> impl IntoResponse {
    deprecated_hash_lookup_response()
}

fn deprecated_hash_lookup_response() -> Response {
    let request_id = uuid::Uuid::new_v4();
    (
        StatusCode::GONE,
        Json(json!({
            "error": {
                "code": "endpoint_deprecated",
                "message": "This endpoint is no longer available. Use /public/verify for existence checks.",
                "request_id": request_id.to_string(),
            }
        })),
    )
        .into_response()
}
