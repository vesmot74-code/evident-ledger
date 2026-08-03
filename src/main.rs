use std::net::SocketAddr;
use std::sync::Arc;

mod api;
mod auth;
mod config;
mod db;
mod hash_attestation;
mod hash_attestation_pdf;
mod merkle;
mod middleware;
mod models;
mod paddle;
mod proof_format;
mod public_certificate_pdf;
mod public_proof;
mod public_verification_audit;
mod public_verify_validation;
mod sac;
mod sac_pdf;
mod service;
mod signing;
mod state;
mod tsa;
mod tsa_worker;
mod web;

async fn serve_whitepaper_pdf() -> impl axum::response::IntoResponse {
    let pdf_bytes: &'static [u8] =
        include_bytes!("../docs/whitepaper/Evident_Ledger_Technical_Whitepaper_v1.0.pdf");

    (
        [
            (axum::http::header::CONTENT_TYPE, "application/pdf"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "inline; filename=\"Evident_Ledger_Technical_Whitepaper_v1.0.pdf\"",
            ),
        ],
        pdf_bytes,
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    let config = config::AppConfig::from_env();

    let (ca_path, untrusted_path) = tsa::freetsa_trust_path_options_from_env();
    if let Err(err) = tsa::check_tsa_configuration(ca_path.as_deref(), untrusted_path.as_deref()) {
        let app_env = std::env::var("APP_ENV").ok();
        let production =
            tsa::is_production_tsa_env(config.environment.as_str(), app_env.as_deref());
        tsa::enforce_tsa_trust_at_startup(production, &err);
    }

    let signer = if config.environment == "production" {
        if !std::path::Path::new(&config.signing_key_path).exists() {
            eprintln!(
                "STARTUP_ERROR signing: Production signing key missing: {}",
                config.signing_key_path_display().display()
            );
            std::process::exit(2);
        }
        match signing::ServerSigner::load_or_create(&config.signing_key_path) {
            Ok(signer) => Arc::new(signer),
            Err(err) => {
                eprintln!("STARTUP_ERROR signing: {}", err);
                std::process::exit(2);
            }
        }
    } else {
        if std::env::var("SIGNING_KEY_PATH").is_err() {
            eprintln!("WARNING: SIGNING_KEY_PATH is not set. Using local development signing key.");
        }
        match signing::ServerSigner::load_or_create(&config.signing_key_path) {
            Ok(signer) => Arc::new(signer),
            Err(err) => {
                eprintln!("STARTUP_ERROR signing: {}", err);
                std::process::exit(2);
            }
        }
    };

    println!("Public key: {}", signer.public_key_hex());
    println!("Signing key SHA256: {}", signer.sha256_fingerprint());
    println!(
        "Signing key path: {}",
        config.signing_key_path_display().display()
    );
    if config.dev_mode {
        println!("Dev mode: enabled (tariff switcher available)");
    }
    println!("Environment: {}", config.environment);
    if !config.paddle_enabled {
        println!("Paddle billing: disabled (PADDLE_ENABLED=false)");
    }

    let pool = db::create_pool().await;
    let state = state::AppState::new(pool, signer, config.clone());

    let rate_limits =
        state::rate_limiter::PublicRateLimitState::from_config(config.trust_proxy_headers);
    let login_limits =
        state::rate_limiter::LoginRateLimitState::from_config(config.trust_proxy_headers);
    let public_routes = api::public_verify::public_router(state.clone(), rate_limits.clone());
    let accounts_routes = api::accounts::router(state.clone(), rate_limits.clone());
    let auth_routes = api::auth::router(state.clone(), login_limits);
    let dashboard_ui = web::dashboard::router(state.clone())
        .merge(api::dashboard_desktop::ui_router(state.clone()));
    let dashboard_api = api::dashboard::router(state.clone())
        .merge(api::dashboard_desktop::api_router(state.clone()));
    let dashboard_billing = api::dashboard_billing::router(state.clone());

    let landing = axum::Router::new()
        .route("/", axum::routing::get(web::landing::index))
        .with_state(state.clone());

    let mut app = axum::Router::new()
        .merge(landing)
        .route(
            "/verify-ui",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("../static/verify.html"))
            }),
        )
        .route(
            "/whitepaper",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("../static/whitepaper.html"))
            }),
        )
        .route("/whitepaper.pdf", axum::routing::get(serve_whitepaper_pdf))
        .route("/login", axum::routing::get(web::dashboard::login_page))
        .route(
            "/register",
            axum::routing::get(web::dashboard::register_page),
        )
        .nest("/account", api::account::router(state.clone()))
        .nest("/backup", api::backup::router(state.clone()))
        .nest("/chains", api::chains::router(state.clone()))
        .nest("/events", api::events::router(state.clone()))
        .nest("/verify", api::verify::router(state.clone()))
        .nest("/identity", api::identity::router(state.clone()))
        .nest("/v1", api::v1::router(state.clone()))
        .nest("/accounts", accounts_routes)
        .nest("/auth", auth_routes)
        .nest(
            "/dashboard",
            dashboard_ui.merge(dashboard_api).merge(dashboard_billing),
        )
        .nest("/public", public_routes);

    if config.paddle_enabled {
        app = app.nest("/paddle", api::paddle_webhook::router(state.clone()));
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Evident Ledger running on http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("STARTUP_ERROR network: failed to bind {}: {}", addr, err);
            std::process::exit(5);
        }
    };
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap_or_else(|err| {
        eprintln!("SERVER_RUNTIME_ERROR: {}", err);
        std::process::exit(27);
    });
}
