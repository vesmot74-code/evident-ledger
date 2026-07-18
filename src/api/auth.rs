//! Web authentication routes (Stage 8.3.0).

use axum::Router;

use crate::auth::web_auth;
use crate::state::rate_limiter::LoginRateLimitState;
use crate::state::AppState;

pub fn router(state: AppState, login_limits: LoginRateLimitState) -> Router {
    web_auth::router(state, login_limits)
}
