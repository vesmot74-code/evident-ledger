//! Stage 9.5 — read-only identity dashboard API and UI tests.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use ed25519_dalek::{Signer, SigningKey};
use evident_ledger::api::{auth, v1};
use evident_ledger::auth::api_key;
use evident_ledger::merkle::MerkleTree;
use evident_ledger::models::event::Event;
use evident_ledger::service::identity_keys::IdentityKeyRepository;
use evident_ledger::service::identity_verification::IdentityVerificationService;
use evident_ledger::state::rate_limiter::LoginRateLimitState;
use evident_ledger::state::AppState;
use evident_ledger::web::dashboard as dashboard_ui;
use rand::rngs::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tower::util::ServiceExt;
use uuid::Uuid;

async fn test_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("db");
    sqlx::migrate!().run(&pool).await.expect("migrate");
    pool
}

fn test_state(pool: sqlx::PgPool) -> AppState {
    common::test_app_state(pool)
}

fn v1_app(state: AppState) -> axum::Router {
    v1::router(state)
}

fn dashboard_app(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/login", axum::routing::get(dashboard_ui::login_page))
        .nest(
            "/auth",
            auth::router(state.clone(), LoginRateLimitState::from_config(false)),
        )
        .nest("/dashboard", dashboard_ui::router(state))
}

fn peer_request(
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let mut req = builder.body(body).expect("request");
    req.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 60)),
        0,
    )));
    req
}

fn cookie_header_from_set_cookie(set_cookies: &[String]) -> Option<String> {
    set_cookies
        .iter()
        .find_map(|line| line.split(';').next().map(str::trim))
        .map(str::to_string)
}

async fn enable_machine_tsa_for_plan(pool: &sqlx::PgPool, plan_name: &str) {
    sqlx::query("UPDATE tariff_plans SET tsa_mode = 'machine' WHERE name = $1")
        .bind(plan_name)
        .execute(pool)
        .await
        .expect("tsa mode");
}

async fn plan_id(pool: &sqlx::PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("plan")
}

struct TestAccount {
    account_id: Uuid,
    api_key: String,
    chain_id: Uuid,
}

async fn create_test_account(pool: &sqlx::PgPool, plan_name: &str, label: &str) -> TestAccount {
    let account_id = Uuid::new_v4();
    let plan = plan_id(pool, plan_name).await;
    sqlx::query(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, subscription_status)
        VALUES ($1, $2, $3, 'active')
        "#,
    )
    .bind(account_id)
    .bind(format!("{account_id}@{label}.test"))
    .bind(plan)
    .execute(pool)
    .await
    .expect("account");

    let generated = api_key::generate_api_key();
    sqlx::query(
        r#"
        INSERT INTO api_keys (api_key_id, account_id, key_hash, key_prefix, label)
        VALUES ($1, $2, $3, $4, 'test')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(account_id)
    .bind(&generated.key_hash)
    .bind(&generated.key_prefix)
    .execute(pool)
    .await
    .expect("api key");

    let chain_id = Uuid::new_v4();
    sqlx::query("INSERT INTO chains (chain_id, head_event_id, account_id) VALUES ($1, NULL, $2)")
        .bind(chain_id)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("chain");

    TestAccount {
        account_id,
        api_key: generated.full_key,
        chain_id,
    }
}

async fn create_identity_key(
    pool: &sqlx::PgPool,
    account_id: Uuid,
    signing_key: &SigningKey,
) -> Uuid {
    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    let fingerprint = IdentityKeyRepository::fingerprint_from_public_key_hex(&public_key_hex)
        .expect("fingerprint");
    IdentityKeyRepository::create(
        pool,
        account_id,
        &public_key_hex,
        &fingerprint,
        Some("dashboard-key"),
    )
    .await
    .expect("identity key")
    .id
}

fn valid_file_hash(label: &str) -> String {
    format!("{:x}", Sha256::digest(label.as_bytes()))
}

fn sign_event_hash(signing_key: &SigningKey, canonical_hash_hex: &str) -> String {
    let raw = hex::decode(canonical_hash_hex).expect("hash hex");
    hex::encode(signing_key.sign(&raw).to_bytes())
}

async fn call_json(app: axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({ "raw": String::from_utf8_lossy(&bytes) }))
    };
    (status, json)
}

async fn submit_signed_event(
    pool: &sqlx::PgPool,
    account: &TestAccount,
    label: &str,
    signing_key: &SigningKey,
    key_id: Uuid,
) -> Uuid {
    let event_id = Uuid::new_v4();
    let file_hash = valid_file_hash(label);
    let sequence: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE chain_id = $1")
            .bind(account.chain_id)
            .fetch_one(pool)
            .await
            .expect("sequence");

    let parent_event_id: Option<Uuid> =
        sqlx::query_scalar("SELECT head_event_id FROM chains WHERE chain_id = $1")
            .bind(account.chain_id)
            .fetch_one(pool)
            .await
            .expect("head");

    let parent = parent_event_id.unwrap_or(Uuid::nil());
    let canonical_hash = MerkleTree::build_leaf(sequence, &event_id, &parent, &file_hash);
    let signature = sign_event_hash(signing_key, &canonical_hash);

    let app = v1_app(test_state(pool.clone()));
    let body = json!({
        "chain_id": account.chain_id,
        "file_hash": file_hash,
        "event_type": "submission",
        "event_id": event_id,
        "identity_signature": {
            "key_id": key_id,
            "signature": signature,
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/events")
        .header("X-API-KEY", &account.api_key)
        .header("content-type", "application/json")
        .header("Idempotency-Key", format!("idem-{label}-{event_id}"))
        .body(Body::from(body.to_string()))
        .expect("request");

    let (status, _) = call_json(app, req).await;
    assert!(status.is_success(), "submit failed: {status}");
    event_id
}

async fn cleanup_account(pool: &sqlx::PgPool, account_id: Uuid) {
    let _ = sqlx::query("DELETE FROM usage_monthly WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM idempotency_records WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM events WHERE chain_id IN (SELECT chain_id FROM chains WHERE account_id = $1)",
    )
    .bind(account_id)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM identity_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM chains WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM api_keys WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await;
}

async fn login_session(app: axum::Router, email: &str, password: &str) -> String {
    let svc = app.clone().into_service();
    let response = svc
        .oneshot(peer_request(
            "POST",
            "/auth/login",
            Some(json!({ "email": email, "password": password })),
            None,
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_string))
        .collect();
    cookie_header_from_set_cookie(&cookies).expect("session cookie")
}

async fn set_web_password(pool: &sqlx::PgPool, account_id: Uuid, password: &str) {
    let hash = evident_ledger::auth::password::hash_password(password).expect("hash");
    sqlx::query("UPDATE accounts SET password_hash = $1 WHERE account_id = $2")
        .bind(hash)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("password");
}

#[tokio::test]
async fn list_identity_keys_returns_keys() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "dash-list").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri("/identity/keys")
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| { k["key_id"].as_str() == Some(key_id.to_string().as_str()) }));

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn list_identity_keys_is_scoped_to_account() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let owner = create_test_account(&pool, "identity", "dash-owner").await;
    let other = create_test_account(&pool, "identity", "dash-other").await;
    let owner_key = SigningKey::generate(&mut OsRng);
    let other_key = SigningKey::generate(&mut OsRng);
    let owner_id = create_identity_key(&pool, owner.account_id, &owner_key).await;
    create_identity_key(&pool, other.account_id, &other_key).await;

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri("/identity/keys")
        .header("X-API-KEY", &owner.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::OK);
    let keys = body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["key_id"], owner_id.to_string());

    cleanup_account(&pool, owner.account_id).await;
    cleanup_account(&pool, other.account_id).await;
}

#[tokio::test]
async fn list_key_events_returns_signed_events() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "dash-events").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id = submit_signed_event(&pool, &account, "dash-event-1", &signing_key, key_id).await;

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/identity/keys/{key_id}/events"))
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["key_id"], key_id.to_string());
    assert_eq!(body["events"].as_array().unwrap().len(), 1);
    assert_eq!(body["events"][0]["event_id"], event_id.to_string());

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn list_key_events_foreign_key_returns_not_found() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let owner = create_test_account(&pool, "identity", "dash-foreign-owner").await;
    let other = create_test_account(&pool, "identity", "dash-foreign-other").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, owner.account_id, &signing_key).await;

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/identity/keys/{key_id}/events"))
        .header("X-API-KEY", &other.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    cleanup_account(&pool, owner.account_id).await;
    cleanup_account(&pool, other.account_id).await;
}

#[tokio::test]
async fn list_identity_keys_events_count_matches_database() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "dash-count").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    submit_signed_event(&pool, &account, "dash-count-1", &signing_key, key_id).await;
    submit_signed_event(&pool, &account, "dash-count-2", &signing_key, key_id).await;

    let db_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE identity_key_id = $1")
            .bind(key_id)
            .fetch_one(&pool)
            .await
            .expect("count");

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri("/identity/keys")
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(db_count, 2);
    assert_eq!(body["keys"][0]["events_count"], 2);

    cleanup_account(&pool, account.account_id).await;
}

async fn expected_signature_valid(pool: &sqlx::PgPool, event_id: Uuid) -> bool {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            String,
            i64,
            Option<Uuid>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT event_id, chain_id, parent_event_id, file_hash, sequence,
               identity_key_id, identity_signature, identity_fingerprint
        FROM events WHERE event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("event");

    let event = Event {
        event_id: row.0,
        chain_id: row.1,
        parent_event_id: row.2,
        file_hash: row.3,
        sequence: row.4,
        identity_key_id: row.5,
        identity_signature: row.6,
        identity_fingerprint: row.7,
    };
    let canonical_hash = MerkleTree::build_leaf(
        event.sequence,
        &event.event_id,
        &event.parent_event_id,
        &event.file_hash,
    );
    IdentityVerificationService::verify(pool, &event, &canonical_hash)
        .await
        .expect("verify")
        .valid
}

#[tokio::test]
async fn list_key_events_signature_valid_matches_verification_service() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "dash-valid").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    let event_id = submit_signed_event(&pool, &account, "dash-valid-1", &signing_key, key_id).await;

    let expected = expected_signature_valid(&pool, event_id).await;

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/identity/keys/{key_id}/events"))
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["events"][0]["identity_signature_valid"], expected);
    assert_eq!(body["events"][0]["identity_signature_valid"], true);

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn list_key_events_revoked_key_history_stays_valid() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "dash-revoked").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    submit_signed_event(&pool, &account, "dash-revoked-1", &signing_key, key_id).await;

    IdentityKeyRepository::revoke(&pool, key_id, account.account_id)
        .await
        .expect("revoke");

    let app = v1_app(test_state(pool.clone()));
    let req = Request::builder()
        .method("GET")
        .uri(format!("/identity/keys/{key_id}/events"))
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, body) = call_json(app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["key_status"], "revoked");
    assert_eq!(body["events"][0]["identity_signature_valid"], true);

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn dashboard_identity_page_requires_session() {
    let pool = test_pool().await;
    let app = dashboard_app(test_state(pool));

    let req = peer_request("GET", "/dashboard/identity", None, None);
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "/login"
    );
}

#[tokio::test]
async fn dashboard_identity_page_renders_with_session() {
    let pool = test_pool().await;
    let account = create_test_account(&pool, "identity", "dash-ui").await;
    set_web_password(&pool, account.account_id, "dashboard-pass").await;
    let email = format!("{}@dash-ui.test", account.account_id);

    let app = dashboard_app(test_state(pool.clone()));
    let cookie = login_session(app.clone(), &email, "dashboard-pass").await;

    let req = peer_request("GET", "/dashboard/identity", None, Some(&cookie));
    let svc = app.into_service();
    let response = svc.oneshot(req).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let html = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&html);
    assert!(html.contains("Identity Keys"));

    cleanup_account(&pool, account.account_id).await;
}

#[tokio::test]
async fn list_key_events_pagination_cursor_works_at_boundary() {
    let pool = test_pool().await;
    enable_machine_tsa_for_plan(&pool, "identity").await;
    let account = create_test_account(&pool, "identity", "dash-page").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let key_id = create_identity_key(&pool, account.account_id, &signing_key).await;
    submit_signed_event(&pool, &account, "dash-page-1", &signing_key, key_id).await;
    submit_signed_event(&pool, &account, "dash-page-2", &signing_key, key_id).await;
    submit_signed_event(&pool, &account, "dash-page-3", &signing_key, key_id).await;

    let app = v1_app(test_state(pool.clone()));

    let req = Request::builder()
        .method("GET")
        .uri(format!("/identity/keys/{key_id}/events?limit=2"))
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, page1) = call_json(app.clone(), req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page1["events"].as_array().unwrap().len(), 2);
    let cursor = page1["next_cursor"].as_str().expect("next cursor");

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/identity/keys/{key_id}/events?limit=2&cursor={cursor}"
        ))
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, page2) = call_json(app.clone(), req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page2["events"].as_array().unwrap().len(), 1);
    assert!(page2["next_cursor"].is_null());

    let last_event_id = Uuid::parse_str(page2["events"][0]["event_id"].as_str().unwrap()).unwrap();
    let last_sequence = page2["events"][0]["sequence"].as_i64().unwrap();
    let tail_cursor =
        evident_ledger::service::identity_dashboard::encode_cursor(last_sequence, last_event_id);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/identity/keys/{key_id}/events?limit=2&cursor={tail_cursor}"
        ))
        .header("X-API-KEY", &account.api_key)
        .body(Body::empty())
        .expect("request");
    let (status, page3) = call_json(app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page3["events"].as_array().unwrap().len(), 0);
    assert!(page3["next_cursor"].is_null());

    cleanup_account(&pool, account.account_id).await;
}
