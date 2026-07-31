use crate::auth::{api_key_auth_middleware, AuthedAccount};
use crate::middleware::subscription_enforcement::subscription_enforcement_middleware;
use crate::models::event::SubmitEventRequest;
use crate::service::ledger::{submit_event, LedgerError};
use crate::state::AppState;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use serde_json::json;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", post(handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            subscription_enforcement_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_key_auth_middleware,
        ))
        .with_state(state)
}

/// Identity event signatures are only supported on `POST /v1/events`.
/// Reject any identity-bearing payload on the legacy path (do not silently drop).
fn reject_identity_fields_on_legacy(req: &SubmitEventRequest) -> Result<(), LedgerError> {
    if req.identity_key_id.is_some()
        || req.identity_signature.is_some()
        || req.identity_fingerprint.is_some()
    {
        return Err(LedgerError::IdentityNotSupportedOnLegacyPath);
    }
    Ok(())
}

/// Legacy `/events` HTTP mapping (SEC-003).
///
/// `ChainAccessDenied` and `ChainNotFound` must be indistinguishable to clients.
/// Shared `LedgerError::IntoResponse` is left unchanged for other surfaces.
pub fn map_legacy_events_error(err: LedgerError) -> Response {
    match err {
        LedgerError::ChainAccessDenied | LedgerError::ChainNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "not_found"
            })),
        )
            .into_response(),
        other => other.into_response(),
    }
}

async fn handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(req): Json<SubmitEventRequest>,
) -> Response {
    if let Err(err) = reject_identity_fields_on_legacy(&req) {
        return map_legacy_events_error(err);
    }

    match submit_event(&state.db, state.signer.as_ref(), auth.account_id, req).await {
        Ok(value) => Json(value).into_response(),
        Err(err) => map_legacy_events_error(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;
    use uuid::Uuid;

    async fn response_parts(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body: Value = serde_json::from_slice(&bytes).expect("json");
        (status, body)
    }

    #[test]
    fn rejects_any_identity_field() {
        let base = || SubmitEventRequest {
            chain_id: Uuid::nil(),
            file_hash: "a".repeat(64),
            idempotency_key: "k".into(),
            parent_event_id: None,
            event_id: None,
            identity_key_id: None,
            identity_signature: None,
            identity_fingerprint: None,
        };

        assert!(reject_identity_fields_on_legacy(&base()).is_ok());

        let mut with_key = base();
        with_key.identity_key_id = Some(Uuid::nil());
        assert!(matches!(
            reject_identity_fields_on_legacy(&with_key),
            Err(LedgerError::IdentityNotSupportedOnLegacyPath)
        ));

        let mut with_sig = base();
        with_sig.identity_signature = Some("ab".into());
        assert!(matches!(
            reject_identity_fields_on_legacy(&with_sig),
            Err(LedgerError::IdentityNotSupportedOnLegacyPath)
        ));

        let mut with_fp = base();
        with_fp.identity_fingerprint = Some("cd".into());
        assert!(matches!(
            reject_identity_fields_on_legacy(&with_fp),
            Err(LedgerError::IdentityNotSupportedOnLegacyPath)
        ));
    }

    /// ChainNotFound is not normally reachable through a fresh chain_id on
    /// POST /events because the endpoint auto-claims unseen chains.
    /// This test verifies the defensive error mapping only:
    /// if ChainNotFound is returned, it must be indistinguishable from
    /// ChainAccessDenied.
    #[tokio::test]
    async fn chain_not_found_and_access_denied_have_same_legacy_error_response() {
        let (chain_not_found_status, chain_not_found_body) =
            response_parts(map_legacy_events_error(LedgerError::ChainNotFound)).await;
        let (chain_access_denied_status, chain_access_denied_body) =
            response_parts(map_legacy_events_error(LedgerError::ChainAccessDenied)).await;

        assert_eq!(chain_not_found_status, StatusCode::NOT_FOUND);
        assert_eq!(chain_access_denied_status, StatusCode::NOT_FOUND);
        assert_eq!(chain_not_found_status, chain_access_denied_status);
        assert_eq!(chain_not_found_body, chain_access_denied_body);
        assert_eq!(chain_not_found_body, json!({ "error": "not_found" }));
    }
}
