//! Dashboard desktop connect API (Stage 13.4).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Form, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::v1::errors::{request_id_layer, ApiError};
use crate::middleware::session_auth::{
    session_auth_middleware, session_ui_auth_middleware, SessionUser,
};
use crate::service::desktop_tokens;
use crate::state::AppState;

pub fn api_router(state: AppState) -> Router {
    Router::new()
        .route("/desktop/connect", post(create_token_handler))
        .route(
            "/desktop/tokens/:token_id/revoke",
            post(revoke_token_handler),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_auth_middleware,
        ))
        .layer(middleware::from_fn(request_id_layer))
        .with_state(state)
}

/// Browser UI for desktop pairing (session cookie required).
pub fn ui_router(state: AppState) -> Router {
    Router::new()
        .route("/desktop/connect", get(connect_page))
        .route("/desktop/connect/confirm", post(connect_form))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            session_ui_auth_middleware,
        ))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct CreateDesktopTokenResponse {
    token: String,
    expires_at: DateTime<Utc>,
    token_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct ConnectQuery {
    redirect_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConnectForm {
    redirect_uri: Option<String>,
}

async fn create_token_handler(
    State(state): State<AppState>,
    session: SessionUser,
) -> Result<Json<CreateDesktopTokenResponse>, ApiError> {
    let created = desktop_tokens::create_desktop_token(&state.db, session.account_id)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(Json(CreateDesktopTokenResponse {
        token: created.plaintext,
        expires_at: created.expires_at,
        token_id: created.id,
    }))
}

async fn revoke_token_handler(
    State(state): State<AppState>,
    session: SessionUser,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let revoked = desktop_tokens::revoke_desktop_token(&state.db, session.account_id, token_id)
        .await
        .map_err(|_| ApiError::Internal)?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn is_allowed_redirect_uri(uri: &str) -> bool {
    let uri = uri.trim();
    let rest = if let Some(r) = uri.strip_prefix("http://127.0.0.1:") {
        r
    } else if let Some(r) = uri.strip_prefix("http://localhost:") {
        r
    } else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else {
        return false;
    };
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let path = path.split('?').next().unwrap_or(path);
    path == "callback" || path == "callback/"
}

fn append_query(base: &str, token: &str, expires_at: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!(
        "{base}{sep}token={}&expires_at={}",
        urlencoding_encode(token),
        urlencoding_encode(expires_at)
    )
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn connect_page(Query(query): Query<ConnectQuery>) -> impl IntoResponse {
    let redirect = query
        .redirect_uri
        .as_deref()
        .filter(|u| is_allowed_redirect_uri(u))
        .unwrap_or("");
    let redirect_attr = html_escape(redirect);
    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Connect Desktop — Evident Ledger</title>
  <style>
    body {{ font-family: 'Segoe UI', system-ui, sans-serif; background: #f4f6f9; color: #0f172a;
           min-height: 100vh; display: grid; place-items: center; margin: 0; padding: 24px; }}
    .card {{ max-width: 480px; width: 100%; background: #fff; border: 1px solid #e2e8f0;
             border-radius: 10px; padding: 28px; box-shadow: 0 1px 2px rgba(15,23,42,.04); }}
    h1 {{ font-size: 1.25rem; margin: 0 0 8px; }}
    p {{ color: #64748b; margin: 0 0 18px; line-height: 1.5; }}
    button {{ background: #3652f6; color: #fff; border: 0; border-radius: 6px; padding: 10px 16px;
              font-weight: 600; cursor: pointer; font-size: 0.95rem; }}
    button:hover {{ filter: brightness(0.96); }}
    .muted {{ font-size: 0.85rem; color: #94a3b8; margin-top: 16px; }}
    a {{ color: #3652f6; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Connect Desktop App</h1>
    <p>Authorize this computer to create proofs with your Evident Ledger account. A desktop token will be issued (not an API key).</p>
    <form method="post" action="/dashboard/desktop/connect/confirm">
      <input type="hidden" name="redirect_uri" value="{redirect_attr}">
      <button type="submit">Connect this computer</button>
    </form>
    <p class="muted">API keys remain for CLI and integrations. Desktop uses a separate Bearer token.</p>
    <p class="muted"><a href="/dashboard/ui">Back to Dashboard</a></p>
  </div>
</body>
</html>"#
    ))
}

async fn connect_form(
    State(state): State<AppState>,
    session: SessionUser,
    Form(form): Form<ConnectForm>,
) -> Result<axum::response::Response, ApiError> {
    let created = desktop_tokens::create_desktop_token(&state.db, session.account_id)
        .await
        .map_err(|_| ApiError::Internal)?;

    if let Some(redirect) = form
        .redirect_uri
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        if !is_allowed_redirect_uri(redirect) {
            return Err(ApiError::InvalidRequest);
        }
        let target = append_query(
            redirect,
            &created.plaintext,
            &created.expires_at.to_rfc3339(),
        );
        return Ok(Redirect::to(&target).into_response());
    }

    Ok(Html(format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8"><title>Connected</title>
<style>body{{font-family:system-ui;padding:40px;}} code{{background:#f1f5f9;padding:2px 6px;border-radius:4px;word-break:break-all;}}</style>
</head><body>
<h1>Desktop connected</h1>
<p>Token issued (shown once). Prefer the Desktop app Connect flow so the token is saved to Keychain automatically.</p>
<p><code>{}</code></p>
<p>Expires: {}</p>
<p><a href="/dashboard/ui">Dashboard</a></p>
</body></html>"#,
        html_escape(&created.plaintext),
        created.expires_at.to_rfc3339()
    ))
    .into_response())
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_localhost_callback_only() {
        assert!(is_allowed_redirect_uri("http://127.0.0.1:54321/callback"));
        assert!(is_allowed_redirect_uri("http://localhost:9/callback"));
        assert!(!is_allowed_redirect_uri("https://evil.example/callback"));
        assert!(!is_allowed_redirect_uri("http://127.0.0.1:9/other"));
        assert!(!is_allowed_redirect_uri("http://192.168.1.1/callback"));
    }
}
