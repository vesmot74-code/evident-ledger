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
mod proof_format;
mod paddle;
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

    let pool = db::create_pool().await;
    let signer = Arc::new(signing::ServerSigner::load_or_create("signing_key.bin"));
    let config = config::AppConfig::from_env();

    println!("Public key: {}", signer.public_key_hex());
    if config.dev_mode {
        println!("Dev mode: enabled (tariff switcher available)");
    }

    let state = state::AppState {
        db: pool,
        signer,
        config: config.clone(),
    };

    let rate_limits =
        state::rate_limiter::PublicRateLimitState::from_config(config.trust_proxy_headers);
    let public_routes = api::public_verify::public_router(state.clone(), rate_limits.clone());
    let accounts_routes = api::accounts::router(state.clone(), rate_limits.clone());

    let app = axum::Router::new()
        .route(
            "/",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("../static/index.html"))
            }),
        )
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
        .nest("/account", api::account::router(state.clone()))
        .nest("/backup", api::backup::router(state.clone()))
        .nest("/chains", api::chains::router(state.clone()))
        .nest("/events", api::events::router(state.clone()))
        .nest("/verify", api::verify::router(state.clone()))
        .nest("/identity", api::identity::router(state.clone()))
        .nest("/v1", api::v1::router(state.clone()))
        .nest("/accounts", accounts_routes)
        .nest("/paddle", api::paddle_webhook::router(state.clone()))
        .nest("/public", public_routes);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Evident Ledger running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
