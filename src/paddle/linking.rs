//! Paddle customer ↔ Evident account linking (Stage 8.2d).

use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkCustomerError {
    CustomerAlreadyLinkedToOtherAccount,
    AccountAlreadyLinkedToOtherCustomer,
    Database,
}

/// Idempotently bind `paddle_customer_id` to `account_id`.
///
/// Rejects when the customer id is owned by another account, or when the
/// account already has a different customer id.
pub async fn link_paddle_customer_to_account(
    pool: &PgPool,
    account_id: Uuid,
    paddle_customer_id: &str,
) -> Result<(), LinkCustomerError> {
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT account_id
        FROM accounts
        WHERE paddle_customer_id = $1
        "#,
    )
    .bind(paddle_customer_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| LinkCustomerError::Database)?;

    if let Some(other) = owner {
        if other == account_id {
            return Ok(());
        }
        warn!(
            account_id = %account_id,
            existing_account_id = %other,
            paddle_customer_id,
            "rejecting paddle customer link: customer already bound to another account"
        );
        return Err(LinkCustomerError::CustomerAlreadyLinkedToOtherAccount);
    }

    let linked: Option<Uuid> = sqlx::query_scalar(
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
    .fetch_optional(pool)
    .await
    .map_err(|_| LinkCustomerError::Database)?;

    if linked.is_some() {
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
    .fetch_optional(pool)
    .await
    .map_err(|_| LinkCustomerError::Database)?
    .flatten();

    if current.as_deref() == Some(paddle_customer_id) {
        return Ok(());
    }

    if current.is_some() {
        warn!(
            account_id = %account_id,
            paddle_customer_id,
            "rejecting paddle customer link: account already has a different customer id"
        );
        return Err(LinkCustomerError::AccountAlreadyLinkedToOtherCustomer);
    }

    Err(LinkCustomerError::Database)
}
