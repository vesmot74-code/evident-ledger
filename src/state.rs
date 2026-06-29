use sqlx::PgPool;
use std::sync::Arc;
use crate::signing::ServerSigner;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub signer: Arc<ServerSigner>,
}
