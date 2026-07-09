use crate::signing::ServerSigner;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub signer: Arc<ServerSigner>,
}
