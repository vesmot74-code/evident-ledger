//! Web authentication handlers and router (Stage 8.3.0).

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::api::v1::errors::{request_id_layer, ApiError};
use crate::auth::password::{self};
use crate::auth::resolve_authed_account;
use crate::auth::session_store::{
    clear_session_cookie, create_session, delete_session_by_token, parse_session_cookie,
    session_cookie_value,
};
use crate::middleware::login_rate_limit::login_rate_limit_middleware;
use crate::service::accounts::{self, SetPasswordError, WebRegisterError};
use crate::state::rate_limiter::LoginRateLimitState;
use crate::state::AppState;

use super::web_me::get_me;

#[derive(Debug, Deserialize)]
pub struct WebRegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct WebRegisterResponse {
    pub account_id: Uuid,
    pub email: String,
    pub plan: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct WebLoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct WebLoginResponse {
    pub account_id: Uuid,
    pub email: String,
    pub plan: String,
    pub subscription_status: String,
}

#[derive(Debug, Deserialize)]
pub struct SetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SetPasswordResponse {
    pub account_id: Uuid,
    pub message: String,
}

pub fn router(state: AppState, login_limits: LoginRateLimitState) -> Router {
    let login =
        Router::new()
            .route("/login", post(login_handler))
            .layer(middleware::from_fn_with_state(
                login_limits.clone(),
                login_rate_limit_middleware,
            ));

    Router::new()
        .route("/register", post(register_handler))
        .merge(login)
        .route("/logout", post(logout_handler))
        .route("/me", get(get_me))
        .route("/set-password", post(set_password_handler))
        .layer(middleware::from_fn(request_id_layer))
        .with_state(state)
}

pub async fn require_web_session(headers: &HeaderMap, pool: &PgPool) -> Result<Uuid, ApiError> {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let token = parse_session_cookie(cookie_header).ok_or(ApiError::Unauthorized)?;
    crate::auth::session_store::resolve_session_account_id(pool, &token)
        .await
        .map_err(|_| ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)
}

async fn register_handler(
    State(state): State<AppState>,
    Json(body): Json<WebRegisterRequest>,
) -> Result<(StatusCode, Json<WebRegisterResponse>), ApiError> {
    if !accounts::is_valid_email(&body.email) || !password::validate_password(&body.password) {
        return Err(ApiError::InvalidRequest);
    }

    let password_hash = password::hash_password(&body.password).map_err(|_| ApiError::Internal)?;

    let result = accounts::register_web_account(&state.db, &body.email, &password_hash)
        .await
        .map_err(|e| match e {
            WebRegisterError::EmailAlreadyRegistered => ApiError::EmailAlreadyRegistered,
            WebRegisterError::Database(_) => ApiError::Internal,
        })?;

    Ok((
        StatusCode::CREATED,
        Json(WebRegisterResponse {
            account_id: result.account_id,
            email: result.email,
            plan: result.plan_name,
            created_at: result.created_at,
        }),
    ))
}

async fn login_handler(
    State(state): State<AppState>,
    Json(body): Json<WebLoginRequest>,
) -> Result<Response, ApiError> {
    if !accounts::is_valid_email(&body.email) {
        return Err(ApiError::InvalidCredentials);
    }

    let email = body.email.trim().to_lowercase();
    let row = sqlx::query(
        r#"
        SELECT
            a.account_id,
            a.password_hash,
            a.email,
            tp.name AS plan_name,
            a.subscription_status
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.email = $1
        "#,
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| ApiError::Internal)?;

    let Some(row) = row else {
        return Err(ApiError::InvalidCredentials);
    };

    let password_hash: Option<String> = row
        .try_get("password_hash")
        .map_err(|_| ApiError::Internal)?;
    let Some(stored_hash) = password_hash else {
        return Err(ApiError::InvalidCredentials);
    };

    let valid =
        password::verify_password(&body.password, &stored_hash).map_err(|_| ApiError::Internal)?;
    if !valid {
        return Err(ApiError::InvalidCredentials);
    }

    let account_id: Uuid = row.try_get("account_id").map_err(|_| ApiError::Internal)?;
    let token = create_session(&state.db, account_id)
        .await
        .map_err(|_| ApiError::Internal)?;

    let secure_cookie = !state.config.dev_mode;
    let body = WebLoginResponse {
        account_id,
        email: row.try_get("email").map_err(|_| ApiError::Internal)?,
        plan: row.try_get("plan_name").map_err(|_| ApiError::Internal)?,
        subscription_status: row
            .try_get("subscription_status")
            .map_err(|_| ApiError::Internal)?,
    };

    Ok((
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            session_cookie_value(&token, secure_cookie),
        )],
        Json(body),
    )
        .into_response())
}

async fn logout_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let token = parse_session_cookie(cookie_header).ok_or(ApiError::Unauthorized)?;
    require_web_session(&headers, &state.db).await?;
    delete_session_by_token(&state.db, &token)
        .await
        .map_err(|_| ApiError::Internal)?;

    let secure_cookie = !state.config.dev_mode;
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/login".to_string()),
            (header::SET_COOKIE, clear_session_cookie(secure_cookie)),
        ],
    )
        .into_response())
}

async fn set_password_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SetPasswordRequest>,
) -> Result<Json<SetPasswordResponse>, ApiError> {
    let auth = resolve_authed_account(&headers, &state)
        .await
        .map_err(|_| ApiError::Unauthorized)?;

    if !password::validate_password(&body.password) {
        return Err(ApiError::InvalidRequest);
    }

    let password_hash = password::hash_password(&body.password).map_err(|_| ApiError::Internal)?;

    accounts::set_account_password(&state.db, auth.account_id, &password_hash)
        .await
        .map_err(|e| match e {
            SetPasswordError::PasswordAlreadySet => ApiError::PasswordAlreadySet,
            SetPasswordError::NotFound => ApiError::NotFound,
            SetPasswordError::Database(_) => ApiError::Internal,
        })?;

    Ok(Json(SetPasswordResponse {
        account_id: auth.account_id,
        message: "Password set successfully".to_string(),
    }))
}
