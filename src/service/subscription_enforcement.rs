//! Subscription billing enforcement for `/v1/*` (Stage 8.2c).

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AccountBillingState {
    pub plan_name: String,
    pub subscription_status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub monthly_commits_limit: Option<i32>,
}

/// Atomic lazy evaluation of pending downgrades and canceled → none transitions.
pub async fn apply_lazy_billing_transitions(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE accounts
        SET
            tariff_plan_id = CASE
                WHEN pending_tariff_plan_id IS NOT NULL AND current_period_end < now()
                    THEN pending_tariff_plan_id
                WHEN subscription_status = 'canceled' AND current_period_end < now()
                    THEN (SELECT plan_id FROM tariff_plans WHERE name = 'free')
                ELSE tariff_plan_id
            END,
            pending_tariff_plan_id = CASE
                WHEN (pending_tariff_plan_id IS NOT NULL AND current_period_end < now())
                  OR (subscription_status = 'canceled' AND current_period_end < now())
                    THEN NULL
                ELSE pending_tariff_plan_id
            END,
            subscription_status = CASE
                WHEN subscription_status = 'canceled' AND current_period_end < now() THEN 'none'
                ELSE subscription_status
            END,
            current_period_end = CASE
                WHEN pending_tariff_plan_id IS NOT NULL AND current_period_end < now()
                    THEN current_period_end + interval '1 month'
                WHEN subscription_status = 'canceled' AND current_period_end < now() THEN NULL
                ELSE current_period_end
            END
        WHERE account_id = $1
          AND (
              (pending_tariff_plan_id IS NOT NULL AND current_period_end < now())
              OR (subscription_status = 'canceled' AND current_period_end < now())
          )
        "#,
    )
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_billing_state(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<AccountBillingState, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            tp.name AS plan_name,
            a.subscription_status,
            a.current_period_end,
            tp.monthly_commits_limit
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;

    Ok(AccountBillingState {
        plan_name: row.try_get("plan_name")?,
        subscription_status: row.try_get("subscription_status")?,
        current_period_end: row.try_get("current_period_end")?,
        monthly_commits_limit: row.try_get("monthly_commits_limit")?,
    })
}

pub fn write_blocked_by_subscription(state: &AccountBillingState) -> bool {
    if state.plan_name == "free" {
        return false;
    }
    state.subscription_status == "past_due"
}

pub async fn usage_limit_exceeded(pool: &PgPool, account_id: Uuid) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            tp.monthly_commits_limit,
            COALESCE(um.server_commits, 0) AS current_usage
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        LEFT JOIN usage_monthly um
            ON um.account_id = a.account_id
           AND um.period_start = date_trunc('month', now())::date
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;

    let limit: Option<i32> = row.try_get("monthly_commits_limit")?;
    let current_usage: i32 = row.try_get("current_usage")?;
    Ok(limit.is_some_and(|limit| current_usage >= limit))
}

pub fn is_read_method(method: &axum::http::Method) -> bool {
    matches!(
        method,
        &axum::http::Method::GET | &axum::http::Method::HEAD | &axum::http::Method::OPTIONS
    )
}

pub async fn account_plan_name(pool: &PgPool, account_id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT tp.name
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
}

pub async fn account_subscription_status(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT subscription_status FROM accounts WHERE account_id = $1")
        .bind(account_id)
        .fetch_one(pool)
        .await
}
