//! Self-service account registration and API key management (Stage 8.1).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthedAccount;
use crate::middleware::public_rate_limit::{
    public_rate_limit_middleware, PublicRateLimitMiddlewareState,
};
use crate::service::accounts::{
    self, ApiKeyRecord, RegisterError, RevokeApiKeyError,
};
use crate::state::rate_limiter::PublicRateLimitState;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub company_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub account_id: Uuid,
    pub api_key: String,
    pub tariff_plan_id: Uuid,
    pub plan_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AccountMeResponse {
    pub account_id: Uuid,
    pub email: String,
    pub tariff_plan_id: Uuid,
    pub plan_name: String,
    pub subscription_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListItem {
    pub id: Uuid,
    pub key_prefix: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListResponse {
    pub api_keys: Vec<ApiKeyListItem>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CreateApiKeyRequest {
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: Uuid,
    pub api_key: String,
    pub key_prefix: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: String,
    message: String,
    request_id: String,
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            error: ErrorBody {
                code: code.to_string(),
                message: message.to_string(),
                request_id: Uuid::new_v4().to_string(),
            },
        }),
    )
        .into_response()
}

fn map_key_record(record: ApiKeyRecord) -> ApiKeyListItem {
    ApiKeyListItem {
        id: record.api_key_id,
        key_prefix: record.key_prefix,
        label: record.label,
        created_at: record.created_at,
        revoked_at: record.revoked_at,
    }
}

pub fn router(state: AppState, rate_limits: PublicRateLimitState) -> Router {
    let register = Router::new()
        .route("/register", post(register_handler))
        .layer(middleware::from_fn_with_state(
            PublicRateLimitMiddlewareState::register(&rate_limits),
            public_rate_limit_middleware,
        ))
        .with_state(state.clone());

    let protected = Router::new()
        .route("/me", get(me_handler))
        .route("/api-keys", get(list_keys_handler).post(create_key_handler))
        .route("/api-keys/:id", delete(revoke_key_handler))
        .with_state(state);

    register.merge(protected)
}

async fn register_handler(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Response {
    if !accounts::is_valid_email(&body.email) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid request",
        );
    }

    if let Some(company_name) = body.company_name.as_deref() {
        if company_name.trim().is_empty() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Invalid request",
            );
        }
    }

    match accounts::register_account(&state.db, &body.email).await {
        Ok(result) => (
            StatusCode::CREATED,
            Json(RegisterResponse {
                account_id: result.account_id,
                api_key: result.api_key,
                tariff_plan_id: result.tariff_plan_id,
                plan_name: result.plan_name,
                created_at: result.created_at,
            }),
        )
            .into_response(),
        Err(RegisterError::EmailAlreadyRegistered) => error_response(
            StatusCode::CONFLICT,
            "conflict",
            "Request conflict",
        ),
        Err(RegisterError::Database(_)) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

async fn me_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Response {
    match accounts::get_account_profile(&state.db, auth.account_id).await {
        Ok(Some(profile)) => (
            StatusCode::OK,
            Json(AccountMeResponse {
                account_id: profile.account_id,
                email: profile.email,
                tariff_plan_id: profile.tariff_plan_id,
                plan_name: profile.plan_name,
                subscription_status: profile.subscription_status,
                created_at: profile.created_at,
            }),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "Resource not found"),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

async fn list_keys_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Response {
    match accounts::list_api_keys(&state.db, auth.account_id).await {
        Ok(keys) => {
            let api_keys = keys.into_iter().map(map_key_record).collect();
            (
                StatusCode::OK,
                Json(ApiKeyListResponse { api_keys }),
            )
                .into_response()
        }
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

async fn create_key_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(body): Json<CreateApiKeyRequest>,
) -> Response {
    let label = body
        .label
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| "default".to_string());

    match accounts::create_api_key(&state.db, auth.account_id, &label).await {
        Ok((generated, record)) => (
            StatusCode::CREATED,
            Json(CreateApiKeyResponse {
                id: record.api_key_id,
                api_key: generated.full_key,
                key_prefix: record.key_prefix,
                label: record.label,
                created_at: record.created_at,
            }),
        )
            .into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

async fn revoke_key_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Path(api_key_id): Path<Uuid>,
) -> Response {
    match accounts::revoke_api_key(&state.db, auth.account_id, api_key_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(RevokeApiKeyError::LastActiveKey) => error_response(
            StatusCode::CONFLICT,
            "last_api_key",
            "Cannot delete the last active API key. Create a new key first.",
        ),
        Err(RevokeApiKeyError::NotFound) => {
            error_response(StatusCode::NOT_FOUND, "not_found", "Resource not found")
        }
        Err(RevokeApiKeyError::Database(_)) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_deserializes_optional_company_name() {
        let body: RegisterRequest =
            serde_json::from_str(r#"{"email":"a@b.com","company_name":"Acme"}"#).unwrap();
        assert_eq!(body.company_name.as_deref(), Some("Acme"));
    }
}
