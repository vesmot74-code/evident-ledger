use crate::config::AppConfig;
use crate::paddle::client::{HttpPaddleClient, PaddleClient};
use crate::signing::ServerSigner;
use sqlx::PgPool;
use std::sync::Arc;

pub mod rate_limiter;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub signer: Arc<ServerSigner>,
    pub config: AppConfig,
    pub paddle: Arc<dyn PaddleClient>,
}

impl AppState {
    pub fn new(db: PgPool, signer: Arc<ServerSigner>, config: AppConfig) -> Self {
        Self {
            db,
            signer,
            config: config.clone(),
            paddle: Arc::new(HttpPaddleClient::from_config(&config)),
        }
    }

    pub fn with_paddle(
        db: PgPool,
        signer: Arc<ServerSigner>,
        config: AppConfig,
        paddle: Arc<dyn PaddleClient>,
    ) -> Self {
        Self {
            db,
            signer,
            config,
            paddle,
        }
    }
}
