//! POST /paddle/webhook — Paddle Billing webhook handler (Stage 8.2b).

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::json;

use crate::paddle::{
    models::PaddleWebhookEvent, process_paddle_webhook, verify_paddle_signature,
    processor::{WebhookError, WebhookOutcome},
};
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/webhook", post(webhook_handler))
        .with_state(state)
}

async fn webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signature = headers
        .get("Paddle-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_paddle_signature(&body, signature, &state.config.paddle_webhook_secret) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "invalid_signature" }),
        );
    }

    let event: PaddleWebhookEvent = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "invalid_payload" }),
            );
        }
    };

    match process_paddle_webhook(&state.db, &event, &body).await {
        Ok(WebhookOutcome::Processed) => (
            StatusCode::OK,
            Json(json!({ "status": "processed" })),
        )
            .into_response(),
        Ok(WebhookOutcome::Idempotent) => (
            StatusCode::OK,
            Json(json!({ "status": "idempotent" })),
        )
            .into_response(),
        Ok(WebhookOutcome::WaitingForAccountLink) => (
            StatusCode::OK,
            Json(json!({ "status": "waiting_for_account_link" })),
        )
            .into_response(),
        Err(WebhookError::PayloadHashConflict) => error_response(
            StatusCode::CONFLICT,
            json!({ "error": "conflict" }),
        ),
        Err(WebhookError::InvalidStatusTransition) | Err(WebhookError::PlanNotFound) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "internal_error" }))
        }
        Err(WebhookError::MissingField(_)) | Err(WebhookError::Database(_)) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "internal_error" }),
        ),
    }
}

fn error_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}
