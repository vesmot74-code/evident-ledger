//! Public verification HTTP handlers (Stage 6.3 / 6.4 / 6.5 / 6.6).

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::middleware::public_rate_limit::{
    public_rate_limit_middleware, PublicRateLimitMiddlewareState,
};
use crate::middleware::public_request::PublicRequestMetadata;
use crate::public_certificate_pdf::render_public_certificate_pdf;
use crate::public_proof::PublicRegistryEntry;
use crate::public_verification_audit::{
    log_public_verification_audit, PublicVerificationAuditEvent, PublicVerificationOutcome,
    PublicVerificationRateLimitAction, PublicVerificationRequestType,
};
use crate::public_verify_validation::{validate_public_file_hash, validate_public_proof_id};
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

pub fn invalid_request_response(request_id: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "code": "invalid_request",
                "message": "Invalid request",
                "request_id": request_id,
            }
        })),
    )
        .into_response()
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

fn audit_verify(
    metadata: &PublicRequestMetadata,
    request_id: &str,
    outcome: PublicVerificationOutcome,
) {
    log_public_verification_audit(&PublicVerificationAuditEvent::new(
        PublicVerificationRequestType::Verify,
        outcome,
        metadata.rate_limit_action,
        request_id,
        metadata.client_ip_hash.clone(),
    ));
}

fn audit_certificate(
    metadata: &PublicRequestMetadata,
    request_id: &str,
    outcome: PublicVerificationOutcome,
) {
    log_public_verification_audit(&PublicVerificationAuditEvent::new(
        PublicVerificationRequestType::CertificatePdf,
        outcome,
        metadata.rate_limit_action,
        request_id,
        metadata.client_ip_hash.clone(),
    ));
}

/// Core verify path: validate → single registry lookup → response.
pub async fn verify_by_hash(
    pool: &PgPool,
    raw_hash: Option<String>,
    metadata: Option<&PublicRequestMetadata>,
) -> Result<Response, sqlx::Error> {
    let request_id = Uuid::new_v4().to_string();

    let normalized = match validate_public_file_hash(raw_hash) {
        Ok(hash) => hash,
        Err(()) => {
            if let Some(metadata) = metadata {
                audit_verify(
                    metadata,
                    &request_id,
                    PublicVerificationOutcome::InvalidRequest,
                );
            }
            return Ok(invalid_request_response(request_id));
        }
    };

    let entry = lookup_public_registry_entry(pool, &normalized).await?;
    let outcome = if entry.is_some() {
        PublicVerificationOutcome::Success
    } else {
        PublicVerificationOutcome::NotFound
    };
    if let Some(metadata) = metadata {
        audit_verify(metadata, &request_id, outcome);
    }
    Ok((StatusCode::OK, Json(response_from_entry(entry))).into_response())
}

/// Test hook: same code path with injectable lookup for call-count assertions.
#[doc(hidden)]
pub async fn verify_by_hash_with_lookup(
    raw_hash: Option<String>,
    lookup: impl FnOnce(
        &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Option<PublicRegistryEntry>, sqlx::Error>>
                + Send,
        >,
    >,
    metadata: Option<&PublicRequestMetadata>,
) -> Result<(Response, u32), sqlx::Error> {
    let request_id = Uuid::new_v4().to_string();
    let normalized = match validate_public_file_hash(raw_hash) {
        Ok(hash) => hash,
        Err(()) => {
            if let Some(metadata) = metadata {
                audit_verify(
                    metadata,
                    &request_id,
                    PublicVerificationOutcome::InvalidRequest,
                );
            }
            return Ok((invalid_request_response(request_id), 0));
        }
    };

    let mut calls = 0u32;
    let entry = {
        calls += 1;
        lookup(&normalized).await?
    };
    let outcome = if entry.is_some() {
        PublicVerificationOutcome::Success
    } else {
        PublicVerificationOutcome::NotFound
    };
    if let Some(metadata) = metadata {
        audit_verify(metadata, &request_id, outcome);
    }
    Ok((
        (StatusCode::OK, Json(response_from_entry(entry))).into_response(),
        calls,
    ))
}

pub async fn public_verify_handler(
    State(state): State<AppState>,
    Query(query): Query<PublicVerifyQuery>,
    metadata: Option<Extension<PublicRequestMetadata>>,
) -> Result<Response, StatusCode> {
    verify_by_hash(&state.db, query.file_hash, metadata.as_deref())
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "public verify lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

pub async fn public_certificate_pdf_handler(
    State(state): State<AppState>,
    Path(public_proof_id): Path<String>,
    metadata: Option<Extension<PublicRequestMetadata>>,
) -> Result<Response, StatusCode> {
    let request_id = Uuid::new_v4().to_string();

    if !validate_public_proof_id(&public_proof_id) {
        if let Some(Extension(metadata)) = metadata.as_ref() {
            audit_certificate(
                metadata,
                &request_id,
                PublicVerificationOutcome::InvalidRequest,
            );
        }
        return Ok(invalid_request_response(request_id));
    }

    let entry = lookup_public_registry_by_id(&state.db, &public_proof_id)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "public certificate lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let Some(entry) = entry else {
        if let Some(Extension(metadata)) = metadata.as_ref() {
            audit_certificate(metadata, &request_id, PublicVerificationOutcome::NotFound);
        }
        return Ok((StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response());
    };

    if let Some(Extension(metadata)) = metadata.as_ref() {
        audit_certificate(metadata, &request_id, PublicVerificationOutcome::Success);
    }

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
    use crate::middleware::public_request::PublicRequestMetadata;
    use sqlx::postgres::PgPoolOptions;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn invalid_hash_does_not_query_database() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/unreachable")
            .expect("lazy pool");

        let response = verify_by_hash(&pool, Some("not-a-valid-hash".into()), None)
            .await
            .expect("validation must not reach db");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn found_and_not_found_use_same_lookup_path() {
        let pool = test_pool().await;
        let file_hash = test_hash("public-verify-lookup-path");
        cleanup(&pool, &file_hash).await;

        crate::public_proof::on_proof_anchored(&pool, uuid::Uuid::new_v4(), &file_hash, "basic")
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

    #[tokio::test]
    async fn unified_lookup_calls_repository_once_for_found_and_missing() {
        let counter = Arc::new(AtomicUsize::new(0));
        let hash = test_hash("unified-path-found");
        let metadata = PublicRequestMetadata {
            client_ip_hash: Some("aa".repeat(64)),
            rate_limit_action: PublicVerificationRateLimitAction::Allowed,
        };

        let counter_missing = counter.clone();
        let (resp_missing, calls_missing) = verify_by_hash_with_lookup(
            Some(hash.clone()),
            move |_| {
                counter_missing.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move { Ok(None) })
            },
            Some(&metadata),
        )
        .await
        .expect("missing");
        assert_eq!(calls_missing, 1);
        assert_eq!(resp_missing.status(), StatusCode::OK);

        let counter_found = counter.clone();
        let found_entry = PublicRegistryEntry {
            public_proof_id: crate::public_proof::generate_public_id(),
            file_hash: hash.clone(),
            proof_status: "REGISTERED".into(),
            registered_at: chrono::Utc::now(),
            tsa_class: "basic".into(),
            integrity_state: "VALID".into(),
            enabled: true,
        };
        let (resp_found, calls_found) = verify_by_hash_with_lookup(
            Some(hash),
            move |_| {
                counter_found.fetch_add(1, Ordering::SeqCst);
                let entry = found_entry.clone();
                Box::pin(async move { Ok(Some(entry)) })
            },
            Some(&metadata),
        )
        .await
        .expect("found");
        assert_eq!(calls_found, 1);
        assert_eq!(resp_found.status(), StatusCode::OK);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    fn test_hash(label: &str) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(label.as_bytes()))
    }

    async fn test_pool() -> PgPool {
        dotenvy::dotenv().ok();
        let database_url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for public_verify tests");
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
