use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::merkle::MerkleTree;
use crate::models::event::SubmitEventRequest;
use crate::public_proof::tsa_class_from_plan;
use crate::service::capabilities::get_account_capabilities;
use crate::service::entitlements::{require_feature, Feature};
use crate::service::identity_signing::{IdentitySigningError, IdentitySigningService};
use crate::service::ledger::{
    ensure_chain_access_in_tx, insert_event_in_tx, plan_next_event, LedgerError,
};
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
pub struct V1IdentitySignature {
    pub key_id: Uuid,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
pub struct V1SubmitEventRequest {
    pub chain_id: Uuid,
    pub file_hash: String,
    pub event_type: String,
    /// Client-assigned event id (required when `identity_signature` is present).
    pub event_id: Option<Uuid>,
    pub identity_signature: Option<V1IdentitySignature>,
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
    if body.identity_signature.is_some() && body.event_id.is_none() {
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
    Ok((derive_proof_status(&snapshot.context), snapshot.signature))
}

fn map_ledger_error(err: LedgerError) -> ApiError {
    match err {
        LedgerError::ChainNotFound | LedgerError::ChainAccessDenied => ApiError::NotFound,
        LedgerError::UsageLimitExceeded => ApiError::UsageLimitExceeded,
        LedgerError::TsaLimitExceeded => ApiError::Internal,
        LedgerError::QualifiedTsaUnavailable => ApiError::Internal,
        LedgerError::ParentMismatch
        | LedgerError::DuplicateIdempotencyKey
        | LedgerError::DuplicateChainSequence => ApiError::Conflict,
        LedgerError::DatabaseError(_) => ApiError::Internal,
    }
}

fn map_identity_signing_error(err: IdentitySigningError) -> ApiError {
    match err {
        IdentitySigningError::KeyNotFound => ApiError::IdentityKeyNotFound,
        IdentitySigningError::KeyRevoked => ApiError::IdentityKeyRevoked,
        IdentitySigningError::KeyNotVerified => ApiError::IdentityKeyNotVerified,
        IdentitySigningError::InvalidSignature => ApiError::InvalidIdentitySignature,
        IdentitySigningError::InvalidEventHash | IdentitySigningError::Database(_) => {
            ApiError::Internal
        }
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
    let tsa_class = tsa_class_from_plan(&capabilities.plan_name);
    let file_hash = normalized_file_hash(&body.file_hash);

    let request_hash = request_hash(&V1SubmitEventRequest {
        chain_id: body.chain_id,
        file_hash: file_hash.clone(),
        event_type: body.event_type.clone(),
        event_id: body.event_id,
        identity_signature: None,
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

    let planned = plan_next_event(&mut *tx, body.chain_id, body.event_id)
        .await
        .map_err(map_ledger_error)?;

    let canonical_event_hash = MerkleTree::build_leaf(
        planned.sequence,
        &planned.event_id,
        &planned.parent_event_id,
        &file_hash,
    );

    let identity_fields = if let Some(identity_signature) = &body.identity_signature {
        require_feature(pool, account_id, Feature::Identity)
            .await
            .map_err(|_| ApiError::EntitlementMissing)?;

        let (key_id, signature, fingerprint) = IdentitySigningService::validate_and_prepare(
            pool,
            account_id,
            identity_signature.key_id,
            &identity_signature.signature,
            &canonical_event_hash,
        )
        .await
        .map_err(map_identity_signing_error)?;

        Some((key_id, signature, fingerprint))
    } else {
        None
    };

    let ledger_req = SubmitEventRequest {
        chain_id: body.chain_id,
        file_hash: file_hash.clone(),
        idempotency_key: idempotency_key.to_string(),
        parent_event_id: None,
        event_id: Some(planned.event_id),
        identity_key_id: identity_fields.as_ref().map(|(key_id, _, _)| *key_id),
        identity_signature: identity_fields
            .as_ref()
            .map(|(_, signature, _)| signature.clone()),
        identity_fingerprint: identity_fields
            .as_ref()
            .map(|(_, _, fingerprint)| fingerprint.clone()),
    };

    let (event_id, sequence) = insert_event_in_tx(&mut *tx, pool, account_id, &ledger_req)
        .await
        .map_err(map_ledger_error)?;

    debug_assert_eq!(event_id, planned.event_id);

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

    insert_in_tx(&mut *tx, &record).await.map_err(|err| {
        if let sqlx::Error::Database(db_err) = &err {
            if db_err.constraint() == Some("uniq_idempotency_account_key") {
                return ApiError::Conflict;
            }
        }
        ApiError::Internal
    })?;

    tx.commit().await.map_err(|_| ApiError::Internal)?;

    // Strategy (b): public materialization runs after the anchoring commit, in a separate
    // transaction inside `on_proof_anchored`. Failures are non-fatal — the public layer is
    // a derived projection and must not block core event submission when materialization fails.
    if proof_status == ProofStatus::Anchored {
        materialize_public_proof_after_anchor(pool, event_id, &file_hash, tsa_class).await;
    }

    post_commit_tsa(pool, body.chain_id, event_id).await;

    Ok((StatusCode::OK, response_json))
}

async fn post_commit_tsa(pool: &PgPool, chain_id: Uuid, event_id: Uuid) {
    if let Some(root) = crate::service::ledger::compute_chain_root(pool, chain_id).await {
        crate::tsa_worker::stamp_chain(pool, chain_id, &root, event_id).await;
    }
}

async fn materialize_public_proof_after_anchor(
    pool: &PgPool,
    proof_id: Uuid,
    file_hash: &str,
    tsa_class: &str,
) {
    if let Err(err) =
        crate::public_proof::on_proof_anchored(pool, proof_id, file_hash, tsa_class).await
    {
        tracing::error!(
            proof_id = %proof_id,
            file_hash = %file_hash,
            error = %err,
            "public proof materialization failed after anchor"
        );
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

    #[tokio::test]
    async fn materialization_failure_does_not_panic_or_propagate() {
        use sqlx::postgres::PgPoolOptions;

        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/nonexistent")
            .expect("lazy pool");
        materialize_public_proof_after_anchor(&pool, Uuid::new_v4(), &"a".repeat(64), "basic")
            .await;
    }
}
