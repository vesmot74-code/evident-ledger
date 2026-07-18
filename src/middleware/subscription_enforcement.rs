//! Subscription enforcement middleware for `/v1/*` (Stage 8.2c).

use axum::{
    body::Body,
    extract::State,
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::api::v1::errors::ApiError;
use crate::auth::AuthedAccount;
use crate::service::subscription_enforcement::{
    apply_lazy_billing_transitions, is_read_method, load_billing_state, usage_limit_exceeded,
    write_blocked_by_subscription,
};
use crate::state::AppState;

pub async fn subscription_enforcement_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(auth) = request.extensions().get::<AuthedAccount>().cloned() else {
        return ApiError::Internal.into_response();
    };

    let is_write = !is_read_method(request.method());

    if let Err(_) = apply_lazy_billing_transitions(&state.db, auth.account_id).await {
        return ApiError::Internal.into_response();
    }

    let billing = match load_billing_state(&state.db, auth.account_id).await {
        Ok(state) => state,
        Err(_) => return ApiError::Internal.into_response(),
    };

    if is_write && write_blocked_by_subscription(&billing) {
        return ApiError::PaymentRequired.into_response();
    }

    if is_write {
        match usage_limit_exceeded(&state.db, auth.account_id).await {
            Ok(true) => return ApiError::UsageLimitExceeded.into_response(),
            Ok(false) => {}
            Err(_) => return ApiError::Internal.into_response(),
        }
    }

    next.run(request).await
}
