//! Web dashboard UI routes (Stage 8.3.1b).

use askama::Template;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use uuid::Uuid;

use crate::auth::api_key;
use crate::middleware::session_auth::{session_ui_auth_middleware, SessionUser};
use crate::service::accounts::{self, RevokeApiKeyError};
use crate::service::tariff;
use crate::state::AppState;
use crate::web::templates::{
    format_optional_datetime, format_optional_text, format_percentage, format_usage_summary,
    ApiKeyCreatedTemplate, ApiKeyRevokedTemplate, ApiKeyRow, ApiKeysTemplate,
    DashboardIndexTemplate, LoginTemplate, RegisterTemplate, SubscriptionTemplate, UsageTemplate,
};

const HX_REQUEST_HEADER: &str = "hx-request";

pub fn router(state: AppState) -> Router {
    let mutations = Router::new()
        .route("/ui/api-keys", post(create_api_key_handler))
        .route("/ui/api-keys/:key_id", delete(revoke_api_key_handler))
        .layer(middleware::from_fn(htmx_csrf_middleware));

    Router::new()
        .route("/ui", get(index_handler))
        .route("/ui/subscription", get(subscription_handler))
        .route("/ui/usage", get(usage_handler))
        .route("/ui/api-keys", get(api_keys_handler))
        .merge(crate::web::dashboard_identity::router(state.clone()))
        .merge(mutations)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_ui_auth_middleware,
        ))
        .with_state(state)
}

pub async fn login_page() -> impl IntoResponse {
    render_template(LoginTemplate)
}

pub async fn register_page() -> impl IntoResponse {
    render_template(RegisterTemplate)
}

async fn index_handler(State(state): State<AppState>, session: SessionUser) -> Response {
    let Ok(Some(profile)) = accounts::get_dashboard_profile(&state.db, session.account_id).await
    else {
        return internal_error("Failed to load account profile");
    };

    let Ok(Some(usage)) = accounts::get_monthly_usage_snapshot(&state.db, session.account_id).await
    else {
        return internal_error("Failed to load usage");
    };

    let percentage = usage
        .monthly_commits_limit
        .map(|limit| usage.server_commits.saturating_mul(100) / limit.max(1));

    let available_plans = tariff::list_upgradeable_plans(&state.db, session.account_id)
        .await
        .unwrap_or_default();

    render_template(DashboardIndexTemplate {
        email: profile.email,
        plan_display: profile.plan_display_name.to_uppercase(),
        usage_summary: format_usage_summary(usage.server_commits, usage.monthly_commits_limit),
        percentage: format_percentage(percentage),
        can_upgrade: !available_plans.is_empty(),
        available_plans,
        paddle_client_token: state.config.paddle_client_token.clone(),
        paddle_environment: state.config.paddle_environment().to_string(),
    })
}

async fn subscription_handler(State(state): State<AppState>, session: SessionUser) -> Response {
    let Ok(Some(snapshot)) =
        accounts::get_subscription_snapshot(&state.db, session.account_id).await
    else {
        return internal_error("Failed to load subscription");
    };

    let available_plans = tariff::list_upgradeable_plans(&state.db, session.account_id)
        .await
        .unwrap_or_default();

    render_template(SubscriptionTemplate {
        plan_display: snapshot.plan_display_name.to_uppercase(),
        plan: snapshot.plan_name,
        subscription_status: snapshot.subscription_status,
        current_period_end: format_optional_datetime(snapshot.current_period_end),
        pending_plan_display: format_optional_text(snapshot.pending_plan_display_name.as_deref()),
        can_upgrade: !available_plans.is_empty(),
        available_plans,
        paddle_client_token: state.config.paddle_client_token.clone(),
        paddle_environment: state.config.paddle_environment().to_string(),
    })
}

async fn usage_handler(State(state): State<AppState>, session: SessionUser) -> Response {
    let Ok(Some(usage)) = accounts::get_monthly_usage_snapshot(&state.db, session.account_id).await
    else {
        return internal_error("Failed to load usage");
    };

    let percentage = usage
        .monthly_commits_limit
        .map(|limit| usage.server_commits.saturating_mul(100) / limit.max(1));

    render_template(UsageTemplate {
        period: usage.period_start.format("%Y-%m").to_string(),
        usage_summary: format_usage_summary(usage.server_commits, usage.monthly_commits_limit),
        percentage: format_percentage(percentage),
    })
}

async fn api_keys_handler(State(state): State<AppState>, session: SessionUser) -> Response {
    let Ok(keys) = accounts::list_api_keys(&state.db, session.account_id).await else {
        return internal_error("Failed to load API keys");
    };

    let api_keys = keys
        .into_iter()
        .map(|record| ApiKeyRow {
            key_id: record.api_key_id.to_string(),
            prefix: api_key::key_prefix_for_listing(&record.key_prefix),
            created_at: record.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
            is_active: record.revoked_at.is_none(),
        })
        .collect();

    render_template(ApiKeysTemplate { api_keys })
}

async fn create_api_key_handler(State(state): State<AppState>, session: SessionUser) -> Response {
    let Ok((generated, _record)) =
        accounts::create_api_key(&state.db, session.account_id, "dashboard").await
    else {
        return internal_error("Failed to create API key");
    };

    render_template(ApiKeyCreatedTemplate {
        api_key: generated.full_key,
    })
}

async fn revoke_api_key_handler(
    State(state): State<AppState>,
    session: SessionUser,
    Path(key_id): Path<Uuid>,
) -> Response {
    match accounts::revoke_api_key(&state.db, session.account_id, key_id).await {
        Ok(()) => render_template(ApiKeyRevokedTemplate),
        Err(RevokeApiKeyError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RevokeApiKeyError::LastActiveKey) => (
            StatusCode::CONFLICT,
            Html("<span class=\"error\">Cannot revoke the last active key</span>"),
        )
            .into_response(),
        Err(RevokeApiKeyError::Database(_)) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn render_template<T: Template>(template: T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => internal_error(&err.to_string()),
    }
}

fn internal_error(message: &str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Html(message.to_string())).into_response()
}

pub async fn htmx_csrf_middleware(request: Request<Body>, next: Next) -> Response {
    if !is_valid_htmx_mutation(&request) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

fn is_valid_htmx_mutation(request: &Request<Body>) -> bool {
    let Some(hx_request) = request.headers().get(HX_REQUEST_HEADER) else {
        return false;
    };
    if hx_request.to_str().ok() != Some("true") {
        return false;
    }
    origin_matches_host(request.headers())
}

fn origin_allows_host(origin: &str, host: &str) -> bool {
    origin == format!("http://{host}") || origin == format!("https://{host}")
}

fn referer_host_matches(referer: &str, host: &str) -> bool {
    referer.starts_with(&format!("http://{host}/"))
        || referer.starts_with(&format!("https://{host}/"))
        || referer == format!("http://{host}")
        || referer == format!("https://{host}")
}

fn origin_matches_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };

    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        return origin_allows_host(origin, host);
    }

    if let Some(referer) = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
    {
        return referer_host_matches(referer, host);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_validation_accepts_matching_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:3000".parse().unwrap());
        headers.insert(header::ORIGIN, "http://localhost:3000".parse().unwrap());
        headers.insert(HX_REQUEST_HEADER, "true".parse().unwrap());
        assert!(origin_matches_host(&headers));
    }

    #[test]
    fn origin_validation_rejects_foreign_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:3000".parse().unwrap());
        headers.insert(header::ORIGIN, "http://evil.example".parse().unwrap());
        assert!(!origin_matches_host(&headers));
    }
}
