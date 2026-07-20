//! Identity keys dashboard UI (Stage 9.5 read-only, Stage 9.7 revoke).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use uuid::Uuid;

use crate::api::v1::errors::ApiError;
use crate::middleware::session_auth::SessionUser;
use crate::service::identity_dashboard::{clamp_events_limit, IdentityDashboardService};
use crate::state::AppState;
use crate::web::templates::{
    format_optional_datetime, IdentityKeyEventRow, IdentityKeyEventsTemplate,
    IdentityKeyRevokedTemplate, IdentityKeyRow, IdentityKeysTemplate,
};

#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    cursor: Option<String>,
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/identity", get(list_handler))
        .route("/identity/:key_id", get(events_handler))
        .route(
            "/identity/:key_id/revoke",
            post(revoke_identity_key_handler),
        )
        .with_state(state)
}

async fn list_handler(State(state): State<AppState>, session: SessionUser) -> Response {
    let keys = match IdentityDashboardService::list_keys(&state.db, session.account_id).await {
        Ok(keys) => keys,
        Err(_) => return internal_error("Failed to load identity keys"),
    };

    let rows = keys
        .into_iter()
        .map(|key| IdentityKeyRow {
            key_id: key.key_id.to_string(),
            fingerprint: key.fingerprint,
            status: key.status,
            created_at: key.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
            verified_at: key.verified_at.format("%Y-%m-%d %H:%M UTC").to_string(),
            revoked_at: format_optional_datetime(key.revoked_at),
            events_count: key.events_count.to_string(),
        })
        .collect();

    render_template(IdentityKeysTemplate { keys: rows })
}

async fn revoke_identity_key_handler(
    State(state): State<AppState>,
    session: SessionUser,
    Path(key_id): Path<Uuid>,
) -> Response {
    match crate::api::v1::identity_key_revoke::revoke_identity_key(
        &state.db,
        session.account_id,
        key_id,
    )
    .await
    {
        Ok(_) => render_template(IdentityKeyRevokedTemplate),
        Err(ApiError::IdentityKeyNotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(ApiError::IdentityKeyAlreadyRevoked) => (
            StatusCode::CONFLICT,
            Html("<span class=\"error\">Identity key already revoked</span>"),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn events_handler(
    State(state): State<AppState>,
    session: SessionUser,
    Path(key_id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
) -> Response {
    let page = match IdentityDashboardService::list_key_events(
        &state.db,
        session.account_id,
        key_id,
        clamp_events_limit(None),
        query.cursor.as_deref(),
    )
    .await
    {
        Ok(page) => page,
        Err(crate::service::identity_dashboard::IdentityDashboardError::KeyNotFound) => {
            return axum::http::StatusCode::NOT_FOUND.into_response();
        }
        Err(_) => return internal_error("Failed to load identity key events"),
    };

    let events = page
        .events
        .into_iter()
        .map(|event| IdentityKeyEventRow {
            event_id: event.event_id.to_string(),
            chain_id: event.chain_id.to_string(),
            sequence: event.sequence.to_string(),
            signed_at: event.signed_at.format("%Y-%m-%d %H:%M UTC").to_string(),
            signature_valid: event.identity_signature_valid,
        })
        .collect();

    render_template(IdentityKeyEventsTemplate {
        key_id: page.key_id.to_string(),
        key_status: page.key_status,
        events,
        next_page_url: page
            .next_cursor
            .map(|cursor| format!("/dashboard/identity/{key_id}?cursor={cursor}")),
    })
}

fn render_template<T: askama::Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "dashboard identity template render failed");
            internal_error("Failed to render page")
        }
    }
}

fn internal_error(message: &str) -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Html(format!("<p class=\"error\">{message}</p>")),
    )
        .into_response()
}
