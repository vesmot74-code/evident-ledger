pub mod account;
pub mod auth;
pub mod chain_verification;
pub mod errors;
pub mod event_access;
pub mod events;
pub mod file_verification;
pub mod idempotency;
pub mod identity_key_events;
pub mod identity_keys;
pub mod proof;
pub mod proof_material;
pub mod proof_state;
pub mod proof_status;
pub mod submit_event;
pub mod validation;
pub mod verify;

use axum::{middleware, Router};

use crate::middleware::subscription_enforcement::subscription_enforcement_middleware;
use crate::state::AppState;

use self::auth::v1_auth_middleware;
use self::errors::request_id_layer;

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/events", events::router(state.clone()))
        .nest("/proof", proof::router(state.clone()))
        .nest("/verify", verify::router(state.clone()))
        .nest("/account", account::router(state.clone()))
        .nest("/identity/keys", identity_keys::router(state.clone()))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            subscription_enforcement_middleware,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), v1_auth_middleware))
        .layer(middleware::from_fn(request_id_layer))
}
