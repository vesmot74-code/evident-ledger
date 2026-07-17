pub mod account;
pub mod auth;
pub mod errors;
pub mod event_access;
pub mod events;
pub mod idempotency;
pub mod proof;
pub mod proof_material;
pub mod proof_state;
pub mod proof_status;
pub mod submit_event;
pub mod validation;
pub mod verify;

use axum::{middleware, Router};

use crate::state::AppState;

use self::errors::request_id_layer;

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/events", events::router(state.clone()))
        .nest("/proof", proof::router(state.clone()))
        .nest("/verify", verify::router(state.clone()))
        .nest("/account", account::router(state))
        .layer(middleware::from_fn(request_id_layer))
}
