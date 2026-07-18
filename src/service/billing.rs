//! Billing service layer — Paddle checkout and customer management (Stage 8.3.2, 10.1).

use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::paddle::client::{paddle_client_error_is_unavailable, PaddleClient, PaddleClientError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingError {
    CustomerCreationFailed,
    CheckoutCreationFailed,
    AlreadyActive,
    AccountNotFound,
    PaddleUnavailable,
    InvalidPlan,
    PlanNotPurchasable,
    Internal,
}

struct AccountPaddleRow {
    email: String,
    paddle_customer_id: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct TariffPlanCheckoutRow {
    name: String,
    paddle_price_id: Option<String>,
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

/// Resolve a purchasable Paddle price id from `tariff_plans`.
pub async fn resolve_purchasable_plan(
    db: &PgPool,
    plan_name: &str,
) -> Result<String, BillingError> {
    let plan = sqlx::query_as::<_, TariffPlanCheckoutRow>(
        r#"
        SELECT name, paddle_price_id
        FROM tariff_plans
        WHERE name = $1
        "#,
    )
    .bind(plan_name)
    .fetch_optional(db)
    .await
    .map_err(|_| BillingError::Internal)?;

    match plan {
        None => Err(BillingError::InvalidPlan),
        Some(plan) if plan.name == "free" => Err(BillingError::InvalidPlan),
        Some(plan) if plan.paddle_price_id.is_none() => Err(BillingError::PlanNotPurchasable),
        Some(plan) => Ok(plan.paddle_price_id.expect("checked above")),
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

/// Create a checkout session for the requested plan.
pub async fn create_checkout(
    db: &PgPool,
    paddle: &dyn PaddleClient,
    account_id: Uuid,
    plan_name: &str,
) -> Result<String, BillingError> {
    let price_id = resolve_purchasable_plan(db, plan_name).await?;

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

    paddle
        .create_checkout(&customer_id, &price_id)
        .await
        .map_err(map_checkout_error)
}

/// Full upgrade flow: validate plan, ensure customer, then create checkout.
pub async fn initiate_upgrade(
    db: &PgPool,
    paddle: &dyn PaddleClient,
    account_id: Uuid,
    email: &str,
    plan_name: &str,
) -> Result<String, BillingError> {
    if has_active_subscription(db, account_id).await? {
        return Err(BillingError::AlreadyActive);
    }

    let price_id = resolve_purchasable_plan(db, plan_name).await?;
    let customer_id = ensure_paddle_customer(db, paddle, account_id, email).await?;

    paddle
        .create_checkout(&customer_id, &price_id)
        .await
        .map_err(map_checkout_error)
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> PgPool {
        dotenvy::dotenv().ok();
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("db")
    }

    #[tokio::test]
    async fn resolve_purchasable_plan_rejects_unknown_plan() {
        let pool = test_pool().await;
        let err = resolve_purchasable_plan(&pool, "hacked_plan")
            .await
            .expect_err("unknown plan");
        assert_eq!(err, BillingError::InvalidPlan);
    }

    #[tokio::test]
    async fn resolve_purchasable_plan_rejects_free_plan() {
        let pool = test_pool().await;
        let err = resolve_purchasable_plan(&pool, "free")
            .await
            .expect_err("free plan");
        assert_eq!(err, BillingError::InvalidPlan);
    }

    #[tokio::test]
    async fn resolve_purchasable_plan_rejects_missing_paddle_price() {
        let pool = test_pool().await;
        let original: Option<String> = sqlx::query_scalar(
            "SELECT paddle_price_id FROM tariff_plans WHERE name = 'identity'",
        )
        .fetch_one(&pool)
        .await
        .expect("identity price");

        sqlx::query("UPDATE tariff_plans SET paddle_price_id = NULL WHERE name = 'identity'")
            .execute(&pool)
            .await
            .expect("clear price");

        let err = resolve_purchasable_plan(&pool, "identity")
            .await
            .expect_err("missing price");
        assert_eq!(err, BillingError::PlanNotPurchasable);

        sqlx::query("UPDATE tariff_plans SET paddle_price_id = $1 WHERE name = 'identity'")
            .bind(original)
            .execute(&pool)
            .await
            .expect("restore price");
    }
}
