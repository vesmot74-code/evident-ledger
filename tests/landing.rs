//! Landing page auth navigation (`GET /`).

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use evident_ledger::auth::session_store::{create_session, SESSION_COOKIE_NAME};
use evident_ledger::service::accounts;
use evident_ledger::state::AppState;
use evident_ledger::web::landing;
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    common::test_pool().await
}

fn landing_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/", axum::routing::get(landing::index))
        .with_state(state)
}

async fn call_html(app: axum::Router, cookie: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().method("GET").uri("/");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let req = builder.body(Body::empty()).expect("request");
    let response = app.into_service().oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn cleanup_email(pool: &sqlx::PgPool, email: &str) {
    let _ = sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE account_id IN (SELECT account_id FROM accounts WHERE email = $1)
        "#,
    )
    .bind(email)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM api_keys WHERE account_id IN (SELECT account_id FROM accounts WHERE email = $1)",
    )
    .bind(email)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await;
}

#[tokio::test]
async fn landing_guest_nav_includes_login_and_register_hrefs() {
    let pool = test_pool().await;
    let app = landing_app(common::test_app_state(pool));

    let (status, html) = call_html(app, None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"href="/login""#),
        "guest landing must include login href"
    );
    assert!(
        html.contains(r#"href="/register""#),
        "guest landing must include register href"
    );
    assert!(
        !html.contains(r#"href="/dashboard/ui""#),
        "guest landing must not include dashboard href"
    );
}

#[tokio::test]
async fn landing_authenticated_nav_includes_dashboard_href() {
    let pool = test_pool().await;
    let email = format!("landing-auth-{}@example.com", Uuid::new_v4());
    cleanup_email(&pool, &email).await;

    let account = accounts::register_account(&pool, &email)
        .await
        .expect("register");
    let token = create_session(&pool, account.account_id)
        .await
        .expect("session");
    let cookie = format!("{SESSION_COOKIE_NAME}={token}");

    let app = landing_app(common::test_app_state(pool.clone()));
    let (status, html) = call_html(app, Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"href="/dashboard/ui""#),
        "authenticated landing must include dashboard href"
    );
    assert!(
        !html.contains(r#"href="/register""#),
        "authenticated landing must not include register href"
    );
    assert!(
        !html.contains(r#"href="/login""#),
        "authenticated landing must not include login href"
    );

    cleanup_email(&pool, &email).await;
}

#[tokio::test]
async fn landing_primary_download_points_to_cli_not_gui() {
    let pool = test_pool().await;
    let app = landing_app(common::test_app_state(pool));

    let (status, html) = call_html(app, None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("Download CLI"),
        "landing must label the primary CTA as Download CLI"
    );

    let main_href = extract_href_for_id(&html, "el-download-main");
    let bottom_href = extract_href_for_id(&html, "el-download-main-bottom");
    assert!(
        main_href.contains("/download/evident-") && !main_href.contains("evident-gui"),
        "primary download must target CLI artifact, got {main_href}"
    );
    assert!(
        bottom_href.contains("/download/evident-") && !bottom_href.contains("evident-gui"),
        "bottom primary download must target CLI artifact, got {bottom_href}"
    );

    // OS-detection map for the primary CTA must use CLI assets.
    assert!(html.contains(
        "releases/latest/download/evident-aarch64-apple-darwin'"
    ));
    assert!(html.contains(
        "releases/latest/download/evident-windows-x64.exe'"
    ));
    // GUI remains available only as a labeled preview option.
    assert!(html.contains("GUI Preview"));
    assert!(html.contains("data-download-kind=\"gui\""));
    assert!(html.contains("data-download-kind=\"cli\""));
}

/// Pull `href="…"` for an anchor that includes `id="<id>"` in its opening tag.
fn extract_href_for_id(html: &str, id: &str) -> String {
    let marker = format!(r#"id="{id}""#);
    let Some(id_pos) = html.find(&marker) else {
        return String::new();
    };
    let tag_start = html[..id_pos].rfind('<').unwrap_or(0);
    let tag_end = html[id_pos..]
        .find('>')
        .map(|i| id_pos + i)
        .unwrap_or(id_pos);
    let tag = &html[tag_start..=tag_end];
    let href_key = "href=\"";
    let Some(h) = tag.find(href_key) else {
        return String::new();
    };
    let start = h + href_key.len();
    let end = tag[start..].find('"').map(|i| start + i).unwrap_or(start);
    tag[start..end].to_string()
}
