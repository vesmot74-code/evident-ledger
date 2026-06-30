use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;

mod db;
mod api;
mod service;
mod models;
mod merkle;
mod signing;
mod state;
mod tsa_worker;
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let pool = db::create_pool().await;
    let signer = Arc::new(signing::ServerSigner::load_or_create("signing_key.bin"));

    println!("Public key: {}", signer.public_key_hex());

    let state = state::AppState { db: pool, signer };

    let app = Router::new()
        .nest("/events", api::events::router(state.clone()))
        .nest("/verify", api::verify::router(state.clone()))
        .nest("/chains", api::chains::router(state.clone()));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Evident Ledger running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
