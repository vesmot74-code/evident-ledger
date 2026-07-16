use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};

use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;
use super::submit_event::{submit_v1_event, V1SubmitEventRequest};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", post(handler))
        .with_state(state)
}

async fn handler(
    State(state): State<AppState>,
    auth: V1Auth,
    headers: HeaderMap,
    Json(body): Json<V1SubmitEventRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let idempotency_key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::InvalidRequest)?;

    let (status, response) = submit_v1_event(
        &state.db,
        state.signer.as_ref(),
        auth.0.account_id,
        idempotency_key,
        body,
    )
    .await?;

    Ok((status, Json(response)))
}
