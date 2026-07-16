use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::event::SubmitEventRequest;
use crate::service::capabilities::get_account_capabilities;
use crate::service::ledger::{ensure_chain_access_in_tx, insert_event_in_tx, LedgerError};
use crate::signing::ServerSigner;

use super::errors::ApiError;
use super::idempotency::{
    canonical_json_sha256, find_active_in_tx, insert_in_tx, IdempotencyRecord,
    IDEMPOTENCY_TTL_HOURS,
};
use super::proof_material::{persist_event_signature, proof_snapshot_at_event};
use super::proof_status::{derive_proof_status, ProofStatus};
use super::validation::{is_valid_event_type, is_valid_file_hash};

#[derive(Debug, Deserialize)]
pub struct V1SubmitEventRequest {
    pub chain_id: Uuid,
    pub file_hash: String,
    pub event_type: String,
}

pub fn request_hash(body: &V1SubmitEventRequest) -> String {
    let payload = json!({
        "chain_id": body.chain_id,
        "file_hash": body.file_hash,
        "event_type": body.event_type,
    });
    canonical_json_sha256(&payload)
}

pub fn validate_submit_request(body: &V1SubmitEventRequest) -> Result<(), ApiError> {
    if !is_valid_event_type(&body.event_type) {
        return Err(ApiError::InvalidRequest);
    }
    if !is_valid_file_hash(&body.file_hash) {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

fn normalized_file_hash(file_hash: &str) -> String {
    file_hash.trim().to_ascii_lowercase()
}

fn trust_level_from_plan(plan_name: &str) -> &'static str {
    match plan_name {
        "identity" => "IDENTITY",
        "vault" => "VAULT",
        "legal" => "ENHANCED",
        _ => "BASIC",
    }
}

fn build_v1_response(
    event_id: Uuid,
    chain_id: Uuid,
    sequence: i64,
    proof_status: ProofStatus,
    trust_level: &str,
    request_id: Uuid,
) -> Value {
    json!({
        "event_id": event_id,
        "chain_id": chain_id,
        "sequence": sequence,
        "proof_status": proof_status.as_str(),
        "trust_level": trust_level,
        "request_id": request_id,
    })
}

async fn proof_context_for_event(
    conn: &mut sqlx::PgConnection,
    signer: &ServerSigner,
    chain_id: Uuid,
    event_id: Uuid,
    sequence: i64,
) -> Result<(ProofStatus, String), ApiError> {
    let snapshot = proof_snapshot_at_event(conn, signer, chain_id, event_id, sequence)
        .await
        .map_err(|_| ApiError::Internal)?;
    persist_event_signature(conn, event_id, &snapshot.signature)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok((
        derive_proof_status(&snapshot.context),
        snapshot.signature,
    ))
}

fn map_ledger_error(err: LedgerError) -> ApiError {
    match err {
        LedgerError::ChainNotFound | LedgerError::ChainAccessDenied => ApiError::NotFound,
        LedgerError::UsageLimitExceeded | LedgerError::TsaLimitExceeded => ApiError::Internal,
        LedgerError::QualifiedTsaUnavailable => ApiError::Internal,
        LedgerError::ParentMismatch | LedgerError::DuplicateIdempotencyKey
        | LedgerError::DuplicateChainSequence => ApiError::Conflict,
        LedgerError::DatabaseError(_) => ApiError::Internal,
    }
}

pub async fn submit_v1_event(
    pool: &PgPool,
    signer: &ServerSigner,
    account_id: Uuid,
    idempotency_key: &str,
    body: V1SubmitEventRequest,
) -> Result<(StatusCode, Value), ApiError> {
    validate_submit_request(&body)?;

    let capabilities = get_account_capabilities(pool, account_id)
        .await
        .map_err(|_| ApiError::Internal)?;
    let trust_level = trust_level_from_plan(&capabilities.plan_name);
    let file_hash = normalized_file_hash(&body.file_hash);

    let request_hash = request_hash(&V1SubmitEventRequest {
        chain_id: body.chain_id,
        file_hash: file_hash.clone(),
        event_type: body.event_type.clone(),
    });

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;

    // Replay: return stored response_json as-is (no proof re-derivation).
    if let Some(record) = find_active_in_tx(&mut *tx, account_id, idempotency_key)
        .await
        .map_err(|_| ApiError::Internal)?
    {
        if record.request_hash != request_hash {
            tx.rollback().await.ok();
            return Err(ApiError::Conflict);
        }
        tx.commit().await.map_err(|_| ApiError::Internal)?;
        return Ok((StatusCode::OK, record.response_json));
    }

    ensure_chain_access_in_tx(&mut *tx, account_id, body.chain_id)
        .await
        .map_err(map_ledger_error)?;

    let ledger_req = SubmitEventRequest {
        chain_id: body.chain_id,
        file_hash,
        idempotency_key: idempotency_key.to_string(),
        parent_event_id: None,
    };

    let (event_id, sequence) =
        insert_event_in_tx(&mut *tx, pool, account_id, &ledger_req)
            .await
            .map_err(map_ledger_error)?;

    let (proof_status, _signature) =
        proof_context_for_event(&mut *tx, signer, body.chain_id, event_id, sequence).await?;
    let response_json = build_v1_response(
        event_id,
        body.chain_id,
        sequence,
        proof_status,
        trust_level,
        ApiError::request_id(),
    );

    let now = Utc::now();
    let record = IdempotencyRecord {
        id: Uuid::new_v4(),
        account_id,
        idempotency_key: idempotency_key.to_string(),
        request_hash,
        response_json: response_json.clone(),
        created_at: now,
        expires_at: now + Duration::hours(IDEMPOTENCY_TTL_HOURS),
    };

    insert_in_tx(&mut *tx, &record)
        .await
        .map_err(|err| {
            if let sqlx::Error::Database(db_err) = &err {
                if db_err.constraint() == Some("uniq_idempotency_account_key") {
                    return ApiError::Conflict;
                }
            }
            ApiError::Internal
        })?;

    tx.commit().await.map_err(|_| ApiError::Internal)?;

    post_commit_tsa(pool, body.chain_id, event_id).await;

    Ok((StatusCode::OK, response_json))
}

async fn post_commit_tsa(pool: &PgPool, chain_id: Uuid, event_id: Uuid) {
    if let Some(root) = crate::service::ledger::compute_chain_root(pool, chain_id).await {
        crate::tsa_worker::stamp_chain(pool, chain_id, &root, event_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ledger::LedgerError;

    #[test]
    fn duplicate_chain_sequence_maps_to_api_conflict() {
        assert_eq!(
            map_ledger_error(LedgerError::DuplicateChainSequence),
            ApiError::Conflict
        );
    }
}
