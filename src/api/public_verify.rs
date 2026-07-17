//! Public verification HTTP handlers (Stage 6.3 / 6.4 / 6.5).

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;

use crate::api::v1::file_verification::normalize_query_file_hash;
use crate::middleware::public_rate_limit::{
    public_rate_limit_middleware, PublicRateLimitMiddlewareState,
};
use crate::public_certificate_pdf::render_public_certificate_pdf;
use crate::public_proof::PublicRegistryEntry;
use crate::state::rate_limiter::PublicRateLimitState;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PublicVerifyQuery {
    pub file_hash: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PublicVerifyResponse {
    pub exists: bool,
    pub public_proof_id: Option<String>,
    pub timestamp: Option<String>,
    pub tsa_class: Option<String>,
    pub integrity: Option<String>,
}

pub fn normalize_public_verify_hash(raw: Option<String>) -> Result<String, ()> {
    normalize_query_file_hash(raw).and_then(|opt| opt.ok_or(()))
}

pub async fn lookup_public_registry_entry(
    pool: &PgPool,
    file_hash: &str,
) -> Result<Option<PublicRegistryEntry>, sqlx::Error> {
    sqlx::query_as::<_, PublicRegistryEntry>(
        r#"
        SELECT public_proof_id, file_hash, proof_status, registered_at,
               tsa_class, integrity_state, enabled
        FROM public_proof_registry
        WHERE file_hash = $1
          AND enabled = true
        "#,
    )
    .bind(file_hash)
    .fetch_optional(pool)
    .await
}

pub async fn lookup_public_registry_by_id(
    pool: &PgPool,
    public_proof_id: &str,
) -> Result<Option<PublicRegistryEntry>, sqlx::Error> {
    sqlx::query_as::<_, PublicRegistryEntry>(
        r#"
        SELECT public_proof_id, file_hash, proof_status, registered_at,
               tsa_class, integrity_state, enabled
        FROM public_proof_registry
        WHERE public_proof_id = $1
          AND enabled = true
        "#,
    )
    .bind(public_proof_id)
    .fetch_optional(pool)
    .await
}

pub fn invalid_hash_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "invalid_hash" })),
    )
        .into_response()
}

fn response_from_entry(entry: Option<PublicRegistryEntry>) -> PublicVerifyResponse {
    match entry {
        Some(row) => PublicVerifyResponse {
            exists: true,
            public_proof_id: Some(row.public_proof_id),
            timestamp: Some(row.registered_at.to_rfc3339()),
            tsa_class: Some(row.tsa_class),
            integrity: Some(row.integrity_state),
        },
        None => PublicVerifyResponse {
            exists: false,
            public_proof_id: None,
            timestamp: None,
            tsa_class: None,
            integrity: None,
        },
    }
}

pub async fn verify_by_hash(
    pool: &PgPool,
    raw_hash: Option<String>,
) -> Result<Response, sqlx::Error> {
    let normalized = match normalize_public_verify_hash(raw_hash) {
        Ok(hash) => hash,
        Err(()) => return Ok(invalid_hash_response()),
    };

    let entry = lookup_public_registry_entry(pool, &normalized).await?;
    Ok((
        StatusCode::OK,
        Json(response_from_entry(entry)),
    )
        .into_response())
}

pub async fn public_verify_handler(
    State(state): State<AppState>,
    Query(query): Query<PublicVerifyQuery>,
) -> Result<Response, StatusCode> {
    verify_by_hash(&state.db, query.file_hash)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "public verify lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn public_certificate_pdf_handler(
    State(state): State<AppState>,
    Path(public_proof_id): Path<String>,
) -> Result<Response, StatusCode> {
    if !public_proof_id.starts_with("pv_") {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found" })),
        )
            .into_response());
    }

    let entry = lookup_public_registry_by_id(&state.db, &public_proof_id)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "public certificate lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let Some(entry) = entry else {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found" })),
        )
            .into_response());
    };

    let pdf_bytes = render_public_certificate_pdf(&entry);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/pdf".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"public-certificate-{}.pdf\"",
                    &entry.public_proof_id[3..11.min(entry.public_proof_id.len())]
                ),
            ),
        ],
        pdf_bytes,
    )
        .into_response())
}

pub fn public_router(state: AppState, rate_limits: PublicRateLimitState) -> Router {
    Router::new()
        .route(
            "/verify",
            get(public_verify_handler).layer(middleware::from_fn_with_state(
                PublicRateLimitMiddlewareState::verify(&rate_limits),
                public_rate_limit_middleware,
            )),
        )
        .route(
            "/verify/:public_proof_id/certificate.pdf",
            get(public_certificate_pdf_handler).layer(middleware::from_fn_with_state(
                PublicRateLimitMiddlewareState::certificate(&rate_limits),
                public_rate_limit_middleware,
            )),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    #[test]
    fn normalize_rejects_invalid_hash() {
        assert!(normalize_public_verify_hash(Some("not-a-valid-hash".into())).is_err());
        assert!(normalize_public_verify_hash(None).is_err());
    }

    #[tokio::test]
    async fn invalid_hash_does_not_query_database() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/unreachable")
            .expect("lazy pool");

        let response = verify_by_hash(&pool, Some("not-a-valid-hash".into()))
            .await
            .expect("validation must not reach db");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn found_and_not_found_use_same_lookup_path() {
        let pool = test_pool().await;
        let file_hash = test_hash("public-verify-lookup-path");
        cleanup(&pool, &file_hash).await;

        crate::public_proof::on_proof_anchored(
            &pool,
            uuid::Uuid::new_v4(),
            &file_hash,
            "basic",
        )
        .await
        .expect("anchor");

        assert!(lookup_public_registry_entry(&pool, &file_hash)
            .await
            .expect("lookup")
            .is_some());
        assert!(lookup_public_registry_entry(&pool, "a".repeat(64).as_str())
            .await
            .expect("lookup")
            .is_none());

        cleanup(&pool, &file_hash).await;
    }

    fn test_hash(label: &str) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(label.as_bytes()))
    }

    async fn test_pool() -> PgPool {
        dotenvy::dotenv().ok();
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for public_verify tests");
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("test db connection failed")
    }

    async fn cleanup(pool: &PgPool, file_hash: &str) {
        let _ = sqlx::query("DELETE FROM public_proof_materialization WHERE file_hash = $1")
            .bind(file_hash)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM public_proof_registry WHERE file_hash = $1")
            .bind(file_hash)
            .execute(pool)
            .await;
    }
}
