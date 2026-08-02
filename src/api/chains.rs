use crate::api::v1::errors::ApiError;
use crate::auth::{api_key_auth_middleware, AuthedAccount};
use crate::middleware::subscription_enforcement::subscription_enforcement_middleware;
use crate::service::chains::create_chain;
use crate::state::AppState;
use axum::{
    extract::State,
    middleware,
    routing::post,
    Json, Router,
};

fn map_internal(err: impl std::fmt::Display, context: &'static str) -> ApiError {
    tracing::error!(error = %err, "{context}");
    ApiError::Internal
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", post(handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            subscription_enforcement_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_key_auth_middleware,
        ))
        .with_state(state)
}

async fn handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
) -> Result<Json<serde_json::Value>, ApiError> {
    create_chain(&state.db, auth.account_id)
        .await
        .map(Json)
        .map_err(|e| map_internal(e, "legacy create_chain failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::Value;

    #[tokio::test]
    async fn map_internal_response_hides_leaky_details() {
        let leaky = "sqlx::Error Postgres protocol /Users/test/src/foo.rs: panic in /home/ci";
        let response = map_internal(leaky, "test chains leak").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&body);
        for needle in [
            "sqlx",
            "Postgres",
            "postgres",
            ".rs:",
            "panic",
            "/Users/",
            "/home/",
        ] {
            assert!(
                !text.contains(needle),
                "response must not contain {needle:?}: {text}"
            );
        }

        let json: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["error"]["code"], "internal_error");
        assert_eq!(json["error"]["message"], "Internal server error");
    }
}
