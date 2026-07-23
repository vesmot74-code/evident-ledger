use crate::auth::{api_key_auth_middleware, AuthedAccount};
use crate::middleware::subscription_enforcement::subscription_enforcement_middleware;
use crate::service::chains::create_chain;
use crate::state::AppState;
use axum::{
    extract::State,
    middleware,
    routing::post,
    Json, Router,
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
) -> Result<Json<serde_json::Value>, String> {
    create_chain(&state.db, auth.account_id)
        .await
        .map(Json)
        .map_err(|e| e.to_string())
}
