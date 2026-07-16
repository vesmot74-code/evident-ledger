use axum::{
    async_trait,
    extract::FromRequestParts,
    http::request::Parts,
};

use crate::auth::AuthedAccount;
use crate::state::AppState;

use super::errors::ApiError;

/// v1 API authentication — reuses legacy `X-API-KEY` → `AuthedAccount` resolution.
pub struct V1Auth(pub AuthedAccount);

#[async_trait]
impl FromRequestParts<AppState> for V1Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        AuthedAccount::from_request_parts(parts, state)
            .await
            .map(V1Auth)
            .map_err(|_| ApiError::Unauthorized)
    }
}
