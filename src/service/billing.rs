//! Billing service layer — Paddle checkout and customer management (Stage 8.3.2).

use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::paddle::client::{paddle_client_error_is_unavailable, PaddleClient, PaddleClientError};

/// MVP upgrade target — fixed server-side, not accepted from clients.
pub const DEFAULT_UPGRADE_PLAN_NAME: &str = "legal";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingError {
    CustomerCreationFailed,
    CheckoutCreationFailed,
    AlreadyActive,
    AccountNotFound,
    PaddleUnavailable,
    Internal,
}

struct AccountPaddleRow {
    email: String,
    paddle_customer_id: Option<String>,
}

/// Check whether the account already has an active paid subscription.
pub async fn has_active_subscription(
    db: &PgPool,
    account_id: Uuid,
) -> Result<bool, BillingError> {
    let status: Option<String> = sqlx::query_scalar(
        r#"
        SELECT subscription_status
        FROM accounts
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(db)
    .await
    .map_err(|_| BillingError::Internal)?;

    match status {
        Some(value) => Ok(value == "active"),
        None => Err(BillingError::AccountNotFound),
    }
}

/// Ensure a Paddle customer exists for the account (idempotent, row-locked).
pub async fn ensure_paddle_customer(
    db: &PgPool,
    paddle: &dyn PaddleClient,
    account_id: Uuid,
    email: &str,
) -> Result<String, BillingError> {
    let mut tx = db.begin().await.map_err(|_| BillingError::Internal)?;

    let row = lock_account_paddle_row(&mut tx, account_id)
        .await?
        .ok_or(BillingError::AccountNotFound)?;

    if let Some(customer_id) = row.paddle_customer_id {
        tx.commit().await.map_err(|_| BillingError::Internal)?;
        return Ok(customer_id);
    }

    let customer_id = paddle
        .create_customer(email)
        .await
        .map_err(map_create_customer_error)?;

    save_paddle_customer_id(&mut tx, account_id, &customer_id).await?;

    tx.commit().await.map_err(|_| BillingError::Internal)?;
    Ok(customer_id)
}

/// Create a checkout session for the default upgrade plan (`legal`).
pub async fn create_checkout(
    db: &PgPool,
    paddle: &dyn PaddleClient,
    account_id: Uuid,
) -> Result<String, BillingError> {
    let customer_id: Option<String> = sqlx::query_scalar(
        r#"
        SELECT paddle_customer_id
        FROM accounts
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(db)
    .await
    .map_err(|_| BillingError::Internal)?
    .flatten();

    let Some(customer_id) = customer_id else {
        return Err(BillingError::CustomerCreationFailed);
    };

    let price_id: Option<String> = sqlx::query_scalar(
        r#"
        SELECT paddle_price_id
        FROM tariff_plans
        WHERE name = $1
        "#,
    )
    .bind(DEFAULT_UPGRADE_PLAN_NAME)
    .fetch_optional(db)
    .await
    .map_err(|_| BillingError::Internal)?;

    let Some(price_id) = price_id else {
        return Err(BillingError::Internal);
    };

    paddle
        .create_checkout(&customer_id, &price_id)
        .await
        .map_err(map_checkout_error)
}

/// Full upgrade flow: ensure customer, then create checkout for the default plan.
pub async fn initiate_upgrade(
    db: &PgPool,
    paddle: &dyn PaddleClient,
    account_id: Uuid,
    email: &str,
) -> Result<String, BillingError> {
    if has_active_subscription(db, account_id).await? {
        return Err(BillingError::AlreadyActive);
    }

    ensure_paddle_customer(db, paddle, account_id, email).await?;
    create_checkout(db, paddle, account_id).await
}

async fn lock_account_paddle_row(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
) -> Result<Option<AccountPaddleRow>, BillingError> {
    sqlx::query_as::<_, AccountPaddleRow>(
        r#"
        SELECT email, paddle_customer_id
        FROM accounts
        WHERE account_id = $1
        FOR UPDATE
        "#,
    )
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| BillingError::Internal)
}

async fn save_paddle_customer_id(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    paddle_customer_id: &str,
) -> Result<(), BillingError> {
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT account_id
        FROM accounts
        WHERE paddle_customer_id = $1
        "#,
    )
    .bind(paddle_customer_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| BillingError::Internal)?;

    if let Some(other) = owner {
        if other != account_id {
            return Err(BillingError::Internal);
        }
        return Ok(());
    }

    let updated: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE accounts
        SET paddle_customer_id = $1
        WHERE account_id = $2
          AND (paddle_customer_id IS NULL OR paddle_customer_id = $1)
        RETURNING account_id
        "#,
    )
    .bind(paddle_customer_id)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| BillingError::Internal)?;

    if updated.is_some() {
        return Ok(());
    }

    let current: Option<String> = sqlx::query_scalar(
        r#"
        SELECT paddle_customer_id
        FROM accounts
        WHERE account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| BillingError::Internal)?
    .flatten();

    if current.as_deref() == Some(paddle_customer_id) {
        Ok(())
    } else {
        Err(BillingError::Internal)
    }
}

fn map_create_customer_error(err: PaddleClientError) -> BillingError {
    if paddle_client_error_is_unavailable(&err) {
        BillingError::PaddleUnavailable
    } else {
        BillingError::CustomerCreationFailed
    }
}

fn map_checkout_error(err: PaddleClientError) -> BillingError {
    if paddle_client_error_is_unavailable(&err) {
        BillingError::PaddleUnavailable
    } else {
        BillingError::CheckoutCreationFailed
    }
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for AccountPaddleRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            email: row.try_get("email")?,
            paddle_customer_id: row.try_get("paddle_customer_id")?,
        })
    }
}
