//! Public marketing landing page (`GET /`).

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::Html,
};

use crate::middleware::session_auth::optional_session_user;
use crate::state::AppState;

const LANDING_HTML: &str = include_str!("../../static/index.html");
const AUTH_NAV_MARKER: &str = "<!--AUTH_NAV-->";

const GUEST_AUTH_NAV: &str = concat!(
    r#"<a href="/login" data-ru="Войти" data-en="Log in">Log in</a>"#,
    r#"<a href="/register" data-ru="Регистрация" data-en="Sign up">Sign up</a>"#,
);

const AUTHENTICATED_AUTH_NAV: &str =
    r#"<a href="/dashboard/ui" data-ru="Dashboard" data-en="Dashboard">Dashboard</a>"#;

pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok());
    let logged_in = optional_session_user(&state, cookie_header).await.is_some();
    let auth_nav = if logged_in {
        AUTHENTICATED_AUTH_NAV
    } else {
        GUEST_AUTH_NAV
    };

    Html(LANDING_HTML.replace(AUTH_NAV_MARKER, auth_nav))
}
