use crate::hash_attestation::build_hash_attestation;
use crate::hash_attestation_pdf::render_hash_attestation_pdf;
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
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Deserialize)]
struct HashLookupRequest {
    hash: String,
}

#[derive(Debug, Serialize)]
struct HashMatch {
    chain_id: Uuid,
    event_id: Uuid,
    sequence: i64,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct HashLookupResponse {
    found: bool,
    matches: Vec<HashMatch>,
}

async fn handler_verify_hash(
    State(state): State<AppState>,
    Json(payload): Json<HashLookupRequest>,
) -> Result<Json<HashLookupResponse>, ApiError> {
    let hash = payload.hash.trim().to_lowercase();

    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "invalid sha256 hash format".to_string(),
        ));
    }

    let rows = sqlx::query!(
        r#"
        SELECT chain_id, event_id, sequence, created_at
        FROM events
        WHERE file_hash = $1
        ORDER BY created_at ASC
        "#,
        hash
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let matches: Vec<HashMatch> = rows
        .into_iter()
        .map(|r| HashMatch {
            chain_id: r.chain_id,
            event_id: r.event_id,
            sequence: r.sequence,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(HashLookupResponse {
        found: !matches.is_empty(),
        matches,
    }))
}

async fn handler_hash_attestation_pdf(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let hash = hash.trim().to_lowercase();
    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(
            "invalid sha256 hash format".to_string(),
        ));
    }

    let doc = build_hash_attestation(&state.db, &state.signer, &hash)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let pdf_bytes = render_hash_attestation_pdf(&doc);

    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/pdf".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"hash-attestation-{}.pdf\"",
                    &hash[..16]
                ),
            ),
        ],
        pdf_bytes,
    ))
}
