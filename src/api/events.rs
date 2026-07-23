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

async fn handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(req): Json<SubmitEventRequest>,
) -> Result<Json<serde_json::Value>, LedgerError> {
    Ok(Json(
        submit_event(&state.db, state.signer.as_ref(), auth.account_id, req).await?,
    ))
}
