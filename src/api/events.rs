use crate::auth::{api_key_auth_middleware, AuthedAccount};
use crate::middleware::subscription_enforcement::subscription_enforcement_middleware;
use crate::models::event::SubmitEventRequest;
use crate::service::ledger::{submit_event, LedgerError};
use crate::state::AppState;
use axum::{
    extract::{Json, State},
    middleware,
    routing::post,
    Router,
};

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

async fn handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(req): Json<SubmitEventRequest>,
) -> Result<Json<serde_json::Value>, LedgerError> {
    reject_identity_fields_on_legacy(&req)?;
    Ok(Json(
        submit_event(&state.db, state.signer.as_ref(), auth.account_id, req).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
}
