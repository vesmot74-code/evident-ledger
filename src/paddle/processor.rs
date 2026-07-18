//! Paddle webhook business logic (Stage 8.2b, extended 8.2d).

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use super::models::{is_downgrade, is_upgrade, PaddleWebhookEvent, TariffPlanRow};
use super::webhook_store::{
    find_by_paddle_event_id, insert_pending_link, insert_received, insert_received_unlinked,
    last_processed_occurred_at, mark_failed, mark_processed, mark_processing,
    mark_waiting_for_account_link, payload_hash, WebhookEventRow,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookOutcome {
    Processed,
    Idempotent,
    WaitingForAccountLink,
}

#[derive(Debug)]
pub enum WebhookError {
    PayloadHashConflict,
    InvalidStatusTransition,
    PlanNotFound,
    MissingField(&'static str),
    Database(String),
}

#[derive(Debug, Clone)]
pub struct AccountBillingRow {
    pub account_id: Uuid,
    pub tariff_plan_id: Uuid,
    pub subscription_status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub pending_tariff_plan_id: Option<Uuid>,
}

pub async fn resolve_account_by_paddle_customer(
    pool: &PgPool,
    paddle_customer_id: &str,
) -> Result<Option<AccountBillingRow>, sqlx::Error> {
    sqlx::query_as::<_, AccountBillingRow>(
        r#"
        SELECT
            account_id,
            tariff_plan_id,
            subscription_status,
            current_period_end,
            pending_tariff_plan_id
        FROM accounts
        WHERE paddle_customer_id = $1
        "#,
    )
    .bind(paddle_customer_id)
    .fetch_optional(pool)
    .await
}

pub async fn process_paddle_webhook(
    pool: &PgPool,
    event: &PaddleWebhookEvent,
    raw_body: &[u8],
) -> Result<WebhookOutcome, WebhookError> {
    let hash = payload_hash(raw_body);

    if let Some(existing) = find_by_paddle_event_id(pool, &event.event_id)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?
    {
        return handle_existing_event(existing, &hash);
    }

    let customer_id = event
        .customer_id()
        .ok_or(WebhookError::MissingField("customer_id"))?;

    let account = resolve_account_by_paddle_customer(pool, customer_id)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    let Some(account) = account else {
        return store_unlinked_webhook(pool, event, &hash, customer_id).await;
    };

    process_linked_webhook(pool, event, &hash, account).await
}

async fn store_unlinked_webhook(
    pool: &PgPool,
    event: &PaddleWebhookEvent,
    hash: &str,
    customer_id: &str,
) -> Result<WebhookOutcome, WebhookError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    let webhook_id = match insert_received_unlinked(
        &mut tx,
        &event.event_id,
        &event.normalized_event_type(),
        hash,
        event.subscription_id(),
        event.occurred_at,
    )
    .await
    {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            tx.rollback().await.ok();
            if let Some(existing) = find_by_paddle_event_id(pool, &event.event_id)
                .await
                .map_err(|e| WebhookError::Database(e.to_string()))?
            {
                return handle_existing_event(existing, hash);
            }
            return Err(WebhookError::Database(e.to_string()));
        }
        Err(e) => return Err(WebhookError::Database(e.to_string())),
    };

    if !mark_processing(&mut tx, webhook_id)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?
    {
        tx.rollback().await.ok();
        return Err(WebhookError::InvalidStatusTransition);
    }

    let paddle_email = event.customer_email().unwrap_or("");
    insert_pending_link(&mut tx, customer_id, paddle_email)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    mark_waiting_for_account_link(&mut tx, webhook_id)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    Ok(WebhookOutcome::WaitingForAccountLink)
}

async fn process_linked_webhook(
    pool: &PgPool,
    event: &PaddleWebhookEvent,
    hash: &str,
    account: AccountBillingRow,
) -> Result<WebhookOutcome, WebhookError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    let webhook_id = match insert_received(
        &mut tx,
        &event.event_id,
        &event.normalized_event_type(),
        hash,
        account.account_id,
        event.subscription_id(),
        event.occurred_at,
    )
    .await
    {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            tx.rollback().await.ok();
            if let Some(existing) = find_by_paddle_event_id(pool, &event.event_id)
                .await
                .map_err(|e| WebhookError::Database(e.to_string()))?
            {
                return handle_existing_event(existing, hash);
            }
            return Err(WebhookError::Database(e.to_string()));
        }
        Err(e) => return Err(WebhookError::Database(e.to_string())),
    };

    if !mark_processing(&mut tx, webhook_id)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?
    {
        tx.rollback().await.ok();
        return Err(WebhookError::InvalidStatusTransition);
    }

    let skip_state = should_skip_out_of_order(&mut tx, account.account_id, event.occurred_at)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    if !skip_state {
        if let Err(e) = apply_event(&mut tx, &account, event).await {
            mark_failed(&mut tx, webhook_id, &format!("{e:?}"))
                .await
                .ok();
            tx.commit().await.ok();
            return Err(e);
        }
    }

    mark_processed(&mut tx, webhook_id)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;

    Ok(WebhookOutcome::Processed)
}

fn handle_existing_event(
    existing: WebhookEventRow,
    hash: &str,
) -> Result<WebhookOutcome, WebhookError> {
    if existing.payload_hash != hash {
        return Err(WebhookError::PayloadHashConflict);
    }
    if existing.status == "processed" || existing.status == "waiting_for_account_link" {
        return Ok(WebhookOutcome::Idempotent);
    }
    Err(WebhookError::InvalidStatusTransition)
}

async fn should_skip_out_of_order(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    occurred_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let last = last_processed_occurred_at(tx, account_id).await?;
    Ok(last.is_some_and(|ts| occurred_at < ts))
}

async fn apply_event(
    tx: &mut Transaction<'_, Postgres>,
    account: &AccountBillingRow,
    event: &PaddleWebhookEvent,
) -> Result<(), WebhookError> {
    match event.normalized_event_type().as_str() {
        "subscription_created" => subscription_created(tx, account.account_id, event).await,
        "subscription_updated" => subscription_updated(tx, account, event).await,
        "subscription_payment_succeeded" => {
            subscription_payment_succeeded(tx, account.account_id, event).await
        }
        "subscription_payment_failed" => subscription_payment_failed(tx, account.account_id).await,
        "subscription_canceled" => subscription_canceled(tx, account.account_id, event).await,
        other => Err(WebhookError::Database(format!("unsupported event_type: {other}"))),
    }
}

async fn subscription_created(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    event: &PaddleWebhookEvent,
) -> Result<(), WebhookError> {
    let plan = load_plan_by_price_id(
        tx,
        event.price_id().ok_or(WebhookError::MissingField("price_id"))?,
    )
    .await?;
    let period_end = event
        .period_end()
        .ok_or(WebhookError::MissingField("current_period_end"))?;

    sqlx::query(
        r#"
        UPDATE accounts
        SET
            tariff_plan_id = $2,
            pending_tariff_plan_id = NULL,
            subscription_status = 'active',
            current_period_end = $3,
            paddle_subscription_id = COALESCE($4, paddle_subscription_id)
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .bind(plan.plan_id)
    .bind(period_end)
    .bind(event.subscription_id())
    .execute(&mut **tx)
    .await
    .map_err(|e| WebhookError::Database(e.to_string()))?;

    Ok(())
}

async fn subscription_updated(
    tx: &mut Transaction<'_, Postgres>,
    account: &AccountBillingRow,
    event: &PaddleWebhookEvent,
) -> Result<(), WebhookError> {
    let new_plan = load_plan_by_price_id(
        tx,
        event.price_id().ok_or(WebhookError::MissingField("price_id"))?,
    )
    .await?;
    let current_plan = load_plan_by_id(tx, account.tariff_plan_id).await?;
    let period_end = event
        .period_end()
        .ok_or(WebhookError::MissingField("current_period_end"))?;

    if is_upgrade(&current_plan, &new_plan) {
        sqlx::query(
            r#"
            UPDATE accounts
            SET
                tariff_plan_id = $2,
                pending_tariff_plan_id = NULL,
                subscription_status = 'active',
                current_period_end = $3,
                paddle_subscription_id = COALESCE($4, paddle_subscription_id)
            WHERE account_id = $1
            "#,
        )
        .bind(account.account_id)
        .bind(new_plan.plan_id)
        .bind(period_end)
        .bind(event.subscription_id())
        .execute(&mut **tx)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;
    } else if is_downgrade(&current_plan, &new_plan) {
        sqlx::query(
            r#"
            UPDATE accounts
            SET pending_tariff_plan_id = $2
            WHERE account_id = $1
            "#,
        )
        .bind(account.account_id)
        .bind(new_plan.plan_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;
    } else {
        sqlx::query(
            r#"
            UPDATE accounts
            SET
                subscription_status = 'active',
                current_period_end = $2,
                paddle_subscription_id = COALESCE($3, paddle_subscription_id)
            WHERE account_id = $1
            "#,
        )
        .bind(account.account_id)
        .bind(period_end)
        .bind(event.subscription_id())
        .execute(&mut **tx)
        .await
        .map_err(|e| WebhookError::Database(e.to_string()))?;
    }

    Ok(())
}

async fn subscription_payment_succeeded(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    event: &PaddleWebhookEvent,
) -> Result<(), WebhookError> {
    let period_end = event
        .period_end()
        .ok_or(WebhookError::MissingField("current_period_end"))?;

    sqlx::query(
        r#"
        UPDATE accounts
        SET subscription_status = 'active', current_period_end = $2
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .bind(period_end)
    .execute(&mut **tx)
    .await
    .map_err(|e| WebhookError::Database(e.to_string()))?;

    Ok(())
}

async fn subscription_payment_failed(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
) -> Result<(), WebhookError> {
    sqlx::query(
        r#"
        UPDATE accounts SET subscription_status = 'past_due' WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| WebhookError::Database(e.to_string()))?;

    Ok(())
}

async fn subscription_canceled(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    event: &PaddleWebhookEvent,
) -> Result<(), WebhookError> {
    let period_end = event
        .period_end()
        .ok_or(WebhookError::MissingField("current_period_end"))?;

    sqlx::query(
        r#"
        UPDATE accounts
        SET subscription_status = 'canceled', current_period_end = $2
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .bind(period_end)
    .execute(&mut **tx)
    .await
    .map_err(|e| WebhookError::Database(e.to_string()))?;

    Ok(())
}

async fn load_plan_by_price_id(
    tx: &mut Transaction<'_, Postgres>,
    paddle_price_id: &str,
) -> Result<TariffPlanRow, WebhookError> {
    sqlx::query_as::<_, TariffPlanRow>(
        r#"
        SELECT plan_id, name, priority
        FROM tariff_plans
        WHERE paddle_price_id = $1
        "#,
    )
    .bind(paddle_price_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| WebhookError::Database(e.to_string()))?
    .ok_or(WebhookError::PlanNotFound)
}

async fn load_plan_by_id(
    tx: &mut Transaction<'_, Postgres>,
    plan_id: Uuid,
) -> Result<TariffPlanRow, WebhookError> {
    sqlx::query_as::<_, TariffPlanRow>(
        r#"
        SELECT plan_id, name, priority FROM tariff_plans WHERE plan_id = $1
        "#,
    )
    .bind(plan_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| WebhookError::Database(e.to_string()))
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for TariffPlanRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            plan_id: row.try_get("plan_id")?,
            name: row.try_get("name")?,
            priority: row.try_get("priority")?,
        })
    }
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for AccountBillingRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            account_id: row.try_get("account_id")?,
            tariff_plan_id: row.try_get("tariff_plan_id")?,
            subscription_status: row.try_get("subscription_status")?,
            current_period_end: row.try_get("current_period_end")?,
            pending_tariff_plan_id: row.try_get("pending_tariff_plan_id")?,
        })
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db) = err {
        return db.code().as_deref() == Some("23505");
    }
    false
}
