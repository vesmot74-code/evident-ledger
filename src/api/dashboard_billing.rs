//! POST /dashboard/upgrade — billing checkout initiation (Stage 8.3.2, 10.1).

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::api::v1::errors::{request_id_layer, ApiError};
use crate::middleware::session_auth::{session_auth_middleware, SessionUser};
use crate::service::accounts;
use crate::service::billing::{self, BillingError};
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/upgrade", post(upgrade_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_auth_middleware,
        ))
        .layer(middleware::from_fn(request_id_layer))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct UpgradeRequest {
    plan: String,
}

#[derive(Debug, Serialize)]
struct UpgradeResponse {
    checkout_url: String,
}

#[derive(Debug, Serialize)]
struct AlreadyActiveResponse {
    status: &'static str,
    message: &'static str,
}

async fn upgrade_handler(
    State(state): State<AppState>,
    session: SessionUser,
    Json(body): Json<UpgradeRequest>,
) -> Response {
    let profile = match accounts::get_dashboard_profile(&state.db, session.account_id).await {
        Ok(Some(profile)) => profile,
        Ok(None) => return ApiError::NotFound.into_response(),
        Err(_) => return ApiError::Internal.into_response(),
    };

    let plan_name = body.plan.trim();
    if plan_name.is_empty() {
        return invalid_plan_response();
    }

    match billing::initiate_upgrade(
        &state.db,
        state.paddle.as_ref(),
        session.account_id,
        &profile.email,
        plan_name,
    )
    .await
    {
        Ok(checkout_url) => (
            StatusCode::OK,
            Json(UpgradeResponse { checkout_url }),
        )
            .into_response(),
        Err(BillingError::AlreadyActive) => (
            StatusCode::CONFLICT,
            Json(AlreadyActiveResponse {
                status: "already_active",
                message: "Subscription already active",
            }),
        )
            .into_response(),
        Err(BillingError::InvalidPlan) => invalid_plan_response(),
        Err(BillingError::PlanNotPurchasable) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "plan_not_purchasable" })),
        )
            .into_response(),
        Err(BillingError::PaddleUnavailable) => {
            paddle_unavailable_response(ApiError::request_id())
        }
        Err(BillingError::AccountNotFound) => ApiError::NotFound.into_response(),
        Err(BillingError::CustomerCreationFailed)
        | Err(BillingError::CheckoutCreationFailed)
        | Err(BillingError::Internal) => ApiError::Internal.into_response(),
    }
}

fn invalid_plan_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "invalid_plan" })),
    )
        .into_response()
}

fn paddle_unavailable_response(request_id: uuid::Uuid) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({
            "error": {
                "code": "paddle_unavailable",
                "message": "Payment provider temporarily unavailable",
                "request_id": request_id.to_string(),
            }
        })),
    )
        .into_response()
}
