use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{models::event::SubmitEventRequest, signing::ServerSigner};

#[derive(Debug)]
pub enum LedgerError {
    ChainNotFound,
    ChainAccessDenied,
    ParentMismatch,
    DuplicateIdempotencyKey,
    UsageLimitExceeded,
    TsaLimitExceeded,
    QualifiedTsaUnavailable,
    DatabaseError(sqlx::Error),
}

impl From<sqlx::Error> for LedgerError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db_err) = err {
            if db_err.constraint() == Some("uniq_idempotency") {
                return LedgerError::DuplicateIdempotencyKey;
            }
        }
        LedgerError::DatabaseError(err)
    }
}

impl IntoResponse for LedgerError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            LedgerError::ChainNotFound => (StatusCode::NOT_FOUND, "Chain not found"),
            LedgerError::ChainAccessDenied => (
                StatusCode::FORBIDDEN,
                "Chain belongs to a different account",
            ),
            LedgerError::ParentMismatch => {
                (StatusCode::CONFLICT, "Parent hash mismatch — fork detected")
            }
            LedgerError::DuplicateIdempotencyKey => {
                (StatusCode::CONFLICT, "Duplicate idempotency key")
            }
            LedgerError::UsageLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "Monthly commit limit exceeded for your tariff plan",
            ),
            LedgerError::TsaLimitExceeded => (
                StatusCode::TOO_MANY_REQUESTS,
                "Monthly TSA limit exceeded for your tariff plan",
            ),
            LedgerError::QualifiedTsaUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Qualified TSA is not yet available for your tariff plan",
            ),
            LedgerError::DatabaseError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Database error"),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

/// Ensures chain exists and `account_id` may append to it.
pub async fn ensure_chain_access_in_tx(
    conn: &mut sqlx::PgConnection,
    account_id: Uuid,
    chain_id: Uuid,
) -> Result<(), LedgerError> {
    #[derive(sqlx::FromRow)]
    struct ChainRow {
        account_id: Option<Uuid>,
    }

    let inserted = sqlx::query_as::<_, ChainRow>(
        r#"
        INSERT INTO chains (chain_id, head_event_id, account_id)
        VALUES ($1, NULL, $2)
        ON CONFLICT (chain_id) DO NOTHING
        RETURNING account_id
        "#,
    )
    .bind(chain_id)
    .bind(account_id)
    .fetch_optional(&mut *conn)
    .await?;

    let chain = match inserted {
        Some(row) => row,
        None => sqlx::query_as::<_, ChainRow>(
            r#"
            SELECT account_id
            FROM chains
            WHERE chain_id = $1
            FOR UPDATE
            "#,
        )
        .bind(chain_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(LedgerError::ChainNotFound)?,
    };

    match chain.account_id {
        Some(owner) if owner != account_id => Err(LedgerError::ChainAccessDenied),
        None => {
            sqlx::query(
                "UPDATE chains SET account_id = $1 WHERE chain_id = $2",
            )
            .bind(account_id)
            .bind(chain_id)
            .execute(&mut *conn)
            .await?;
            Ok(())
        }
        _ => Ok(()),
    }
}

pub async fn submit_event(
    pool: &PgPool,
    signer: &ServerSigner,
    account_id: Uuid,
    req: SubmitEventRequest,
) -> Result<Value, LedgerError> {
    let mut tx = pool.begin().await?;

    ensure_chain_access_in_tx(&mut *tx, account_id, req.chain_id).await?;

    // legacy idempotency (chain_id + body key)
    if let Some(existing) = sqlx::query!(
        r#"
        SELECT event_id, file_hash
        FROM events
        WHERE chain_id = $1 AND idempotency_key = $2
        "#,
        req.chain_id,
        req.idempotency_key
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        let head_event_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT head_event_id FROM chains WHERE chain_id = $1",
        )
        .bind(req.chain_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(LedgerError::DatabaseError)?;

        return Ok(json!({
            "event_id": existing.event_id,
            "chain_id": req.chain_id,
            "head_event_id": head_event_id,
            "cached": true
        }));
    }

    let (event_id, sequence) = insert_event_in_tx(&mut *tx, pool, account_id, &req).await?;
    tx.commit().await?;

    finalize_event_submission(pool, signer, req.chain_id, event_id, sequence).await
}

/// Inserts a ledger event inside an open transaction (no commit).
pub async fn insert_event_in_tx(
    conn: &mut sqlx::PgConnection,
    pool: &PgPool,
    account_id: Uuid,
    req: &SubmitEventRequest,
) -> Result<(Uuid, i64), LedgerError> {
    sqlx::query!(
        r#"
        INSERT INTO usage_monthly (account_id, period_start)
        VALUES ($1, date_trunc('month', now())::date)
        ON CONFLICT (account_id, period_start) DO NOTHING
        "#,
        account_id
    )
    .execute(&mut *conn)
    .await?;

    let usage = sqlx::query!(
        r#"
        SELECT server_commits, tsa_requests
        FROM usage_monthly
        WHERE account_id = $1 AND period_start = date_trunc('month', now())::date
        FOR UPDATE
        "#,
        account_id
    )
    .fetch_one(&mut *conn)
    .await?;

    // Capabilities читаются вне транзакции tx (это отдельный pool-запрос,
    // без FOR UPDATE — тарифный план не меняется во время commit, только
    // usage-счётчики нуждаются в блокировке, что уже сделано выше).
    let capabilities =
        crate::service::capabilities::get_account_capabilities(pool, account_id).await?;

    if !capabilities.can_commit(usage.server_commits) {
        return Err(LedgerError::UsageLimitExceeded);
    }
    // TSA-квота проверяется и резервируется здесь же, ДО реального вызова TSA
    // (который произойдёт позже, после tx.commit()) — иначе параллельные запросы
    // могли бы обойти лимит, пока счётчик ещё не обновлён.
    if !capabilities.can_use_tsa(usage.tsa_requests) {
        return Err(LedgerError::TsaLimitExceeded);
    }
    // Тариф Free получает только "machine"-уровень TSA. Если план обещает
    // "qualified" TSA (Legal/Vault/Identity), но реального квалифицированного
    // провайдера ещё нет — честно возвращаем ошибку недоступности, а не
    // молча выдаём machine-TSA под видом qualified. Ложная юридическая
    // значимость хуже отсутствия функции.
    if !capabilities.tsa_available() {
        return Err(LedgerError::QualifiedTsaUnavailable);
    }

    let head_event_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT head_event_id FROM chains WHERE chain_id = $1",
    )
    .bind(req.chain_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(LedgerError::DatabaseError)?;

    let parent_event_id = head_event_id.unwrap_or(Uuid::nil());

    // Sequence is assigned inside the open transaction while the chain row is locked
    // (FOR UPDATE in ensure_chain_access_in_tx), so concurrent commits on the same
    // chain_id get strictly monotonic sequence numbers.
    // TODO(Stage 3): add DB constraint UNIQUE (chain_id, sequence) as defense-in-depth
    // for alternate write paths (bulk-import, admin tools) that bypass this tx lock.
    let sequence = sqlx::query_scalar!(
        r#"
        SELECT COALESCE(MAX(sequence), 0) + 1
        FROM events
        WHERE chain_id = $1
        "#,
        req.chain_id
    )
    .fetch_one(&mut *conn)
    .await?;

    let event_id = Uuid::new_v4();

    sqlx::query!(
        r#"
        INSERT INTO events (
            event_id,
            chain_id,
            parent_event_id,
            file_hash,
            idempotency_key,
            signature,
            sequence
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7)
        "#,
        event_id,
        req.chain_id,
        parent_event_id,
        req.file_hash,
        req.idempotency_key,
        // TODO(Stage 3): persist server signature here; currently proof signs ephemerally
        // in ProofContext::load / finalize_event_submission and is not stored per event.
        "",
        sequence
    )
    .execute(&mut *conn)
    .await?;

    // 5. update head
    sqlx::query!(
        r#"
        UPDATE chains
        SET head_event_id = $1
        WHERE chain_id = $2
        "#,
        event_id,
        req.chain_id
    )
    .execute(&mut *conn)
    .await?;

    // usage: резервируем commit и TSA-запрос заранее, атомарно, до сетевого вызова TSA
    sqlx::query!(
        r#"
        UPDATE usage_monthly
        SET server_commits = server_commits + 1,
            tsa_requests = tsa_requests + 1
        WHERE account_id = $1 AND period_start = date_trunc('month', now())::date
        "#,
        account_id
    )
    .execute(&mut *conn)
    .await?;

    Ok((event_id, sequence.unwrap_or(1)))
}

async fn finalize_event_submission(
    pool: &PgPool,
    signer: &ServerSigner,
    chain_id: Uuid,
    event_id: Uuid,
    sequence: i64,
) -> Result<Value, LedgerError> {
    if let Some(root) = compute_chain_root(pool, chain_id).await {
        crate::tsa_worker::stamp_chain(pool, chain_id, &root, event_id).await;
    }

    // Потом получаем TSA из БД
    let tsa_record = sqlx::query!(
        r#"
        SELECT tsa_timestamp, tsa_serial, length(tsa_token) as token_bytes
        FROM tsa_tokens
        WHERE chain_id = $1 AND event_id = $2
        "#,
        chain_id,
        event_id
    )
    .fetch_optional(pool)
    .await?;

    let events = sqlx::query_as!(
        crate::db::EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await?;

    let root = crate::merkle::MerkleTree::recompute_root_from_events(&events);
    let chain_head = event_id.to_string();
    let signature = signer.sign_root(&chain_id.to_string(), &root, &chain_head);
    let public_key = signer.public_key_hex();

    let event_payloads: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "sequence": event.sequence,
                "event_id": event.event_id,
                "parent_event_id": event.parent_event_id,
                "file_hash": event.file_hash,
            })
        })
        .collect();

    Ok(json!({
        "leaf_version": crate::proof_format::LEAF_VERSION,
        "event_id": event_id,
        "chain_id": chain_id,
        "head_event_id": event_id,
        "sequence": sequence,
        "cached": false,
        "proof": {
            "version": crate::proof_format::PROOF_VERSION,
            "type": crate::proof_format::PROOF_TYPE,
            "root": root,
            "chain_head": chain_head,
            "signature": signature,
            "public_key": public_key,
            "leaves_count": event_payloads.len()
        },
        "events": event_payloads,
        "tsa": tsa_record.map(|t| json!({
            "timestamp": t.tsa_timestamp,
            "serial": t.tsa_serial,
            "token_bytes": t.token_bytes,
        }))
    }))
}

pub fn spawn_tsa_stamp(pool: PgPool, chain_id: Uuid, merkle_root: String, head_event_id: Uuid) {
    tokio::spawn(async move {
        crate::tsa_worker::stamp_chain(&pool, chain_id, &merkle_root, head_event_id).await;
    });
}

pub async fn compute_chain_root(pool: &PgPool, chain_id: Uuid) -> Option<String> {
    let events = sqlx::query_as!(
        crate::db::EventRow,
        r#"
        SELECT event_id, parent_event_id, file_hash, created_at, sequence
        FROM events
        WHERE chain_id = $1
        ORDER BY sequence ASC
        "#,
        chain_id
    )
    .fetch_all(pool)
    .await
    .ok()?;

    if events.is_empty() {
        return None;
    }
    Some(crate::merkle::MerkleTree::recompute_root_from_events(
        &events,
    ))
}
