//! POST /paddle/webhook — Paddle Billing webhook handler (Stage 8.2b / 11.4).

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
    models::PaddleWebhookEvent,
    process_paddle_webhook,
    processor::{WebhookError, WebhookOutcome},
    verify_paddle_signature,
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

    // Permanent: invalid signature (do not retry).
    if !verify_paddle_signature(&body, signature, &state.config.paddle_webhook_secret) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "invalid_signature" }),
        );
    }

    // Permanent: malformed JSON (do not retry). Keep existing 400 contract.
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
        Ok(WebhookOutcome::Processed) => {
            (StatusCode::OK, Json(json!({ "status": "processed" }))).into_response()
        }
        Ok(WebhookOutcome::Idempotent) => {
            (StatusCode::OK, Json(json!({ "status": "idempotent" }))).into_response()
        }
        Ok(WebhookOutcome::WaitingForAccountLink) => (
            StatusCode::OK,
            Json(json!({ "status": "waiting_for_account_link" })),
        )
            .into_response(),
        Ok(WebhookOutcome::Ignored) => {
            (StatusCode::OK, Json(json!({ "status": "ignored" }))).into_response()
        }
        Err(err) => map_processing_error(&event, err),
    }
}

fn map_processing_error(event: &PaddleWebhookEvent, err: WebhookError) -> Response {
    if err.is_temporary() {
        tracing::error!(
            event_id = %event.event_id,
            event_type = %event.event_type,
            error_type = err.error_type_name(),
            error = ?err,
            "paddle webhook temporary processing failure; returning 5xx for Paddle retry"
        );
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "temporary_failure" }),
        );
    }

    // Permanent payload/structure issues — do not invite Paddle retry.
    match err {
        WebhookError::PayloadHashConflict => {
            error_response(StatusCode::CONFLICT, json!({ "error": "conflict" }))
        }
        WebhookError::MissingField(field) => error_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "invalid_payload", "field": field }),
        ),
        other => {
            // Defensive: unclassified permanent variants.
            tracing::warn!(
                event_id = %event.event_id,
                event_type = %event.event_type,
                error_type = other.error_type_name(),
                "paddle webhook permanent processing failure"
            );
            error_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "invalid_payload" }),
            )
        }
    }
}

fn error_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}
