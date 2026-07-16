use axum::{
    routing::get,
    Router,
};

use crate::state::AppState;

use super::auth::V1Auth;
use super::errors::ApiError;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/capabilities", get(not_implemented))
        .with_state(state)
}

async fn not_implemented(_auth: V1Auth) -> Result<(), ApiError> {
    Err(ApiError::NotImplemented)
}
