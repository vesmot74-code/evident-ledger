use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::errors::ApiError;

#[derive(Debug, sqlx::FromRow)]
struct EventAccessRow {
    event_id: Uuid,
    chain_id: Uuid,
    parent_event_id: Uuid,
    file_hash: String,
    sequence: i64,
    created_at: DateTime<Utc>,
    signature: String,
    account_id: Option<Uuid>,
    identity_key_id: Option<Uuid>,
    identity_signature: Option<String>,
    identity_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub event_id: Uuid,
    pub chain_id: Uuid,
    pub account_id: Uuid,
    pub parent_event_id: Uuid,
    pub file_hash: String,
    pub sequence: i64,
    pub created_at: DateTime<Utc>,
    pub signature: String,
    pub identity_key_id: Option<Uuid>,
    pub identity_signature: Option<String>,
    pub identity_fingerprint: Option<String>,
}

/// Ensures `event_id` exists and belongs to `account_id`.
///
/// Returns `404 Not Found` when the event is missing or owned by another account,
/// so foreign event existence is not revealed.
pub async fn verify_event_access(
    pool: &PgPool,
    account_id: Uuid,
    event_id: Uuid,
) -> Result<Event, ApiError> {
    let row = sqlx::query_as::<_, EventAccessRow>(
        r#"
        SELECT
            e.event_id,
            e.chain_id,
            e.parent_event_id,
            e.file_hash,
            e.sequence,
            e.created_at,
            e.signature,
            c.account_id,
            e.identity_key_id,
            e.identity_signature,
            e.identity_fingerprint
        FROM events e
        INNER JOIN chains c ON c.chain_id = e.chain_id
        WHERE e.event_id = $1
        "#,
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    let Some(row) = row else {
        return Err(ApiError::NotFound);
    };

    let Some(owner_id) = row.account_id else {
        return Err(ApiError::NotFound);
    };

    if owner_id != account_id {
        return Err(ApiError::NotFound);
    }

    Ok(Event {
        event_id: row.event_id,
        chain_id: row.chain_id,
        account_id: owner_id,
        parent_event_id: row.parent_event_id,
        file_hash: row.file_hash,
        sequence: row.sequence,
        created_at: row.created_at,
        signature: row.signature,
        identity_key_id: row.identity_key_id,
        identity_signature: row.identity_signature,
        identity_fingerprint: row.identity_fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    struct EventOwnershipFixture {
        account_a: Uuid,
        account_b: Uuid,
        event_a: Uuid,
        chain_a: Uuid,
    }

    impl EventOwnershipFixture {
        async fn setup(pool: &PgPool) -> Self {
            let account_a = Uuid::new_v4();
            let account_b = Uuid::new_v4();
            let chain_a = Uuid::new_v4();
            let event_a = Uuid::new_v4();
            let free_plan_id: Uuid =
                sqlx::query_scalar("SELECT plan_id FROM tariff_plans WHERE name = 'free'")
                    .fetch_one(pool)
                    .await
                    .expect("free tariff plan");

            sqlx::query(
                "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)",
            )
            .bind(account_a)
            .bind(format!("event-access-a-{account_a}@test.local"))
            .bind(free_plan_id)
            .execute(pool)
            .await
            .expect("insert account_a");

            sqlx::query(
                "INSERT INTO accounts (account_id, email, tariff_plan_id) VALUES ($1, $2, $3)",
            )
            .bind(account_b)
            .bind(format!("event-access-b-{account_b}@test.local"))
            .bind(free_plan_id)
            .execute(pool)
            .await
            .expect("insert account_b");

            sqlx::query("INSERT INTO chains (chain_id, account_id) VALUES ($1, $2)")
                .bind(chain_a)
                .bind(account_a)
                .execute(pool)
                .await
                .expect("insert chain_a");

            sqlx::query(
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
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                "#,
            )
            .bind(event_a)
            .bind(chain_a)
            .bind(Uuid::nil())
            .bind("aa".repeat(32))
            .bind(format!("idem-{event_a}"))
            .bind("")
            .bind(1_i64)
            .execute(pool)
            .await
            .expect("insert event_a");

            Self {
                account_a,
                account_b,
                event_a,
                chain_a,
            }
        }

        async fn teardown(pool: &PgPool, fixture: &Self) {
            let _ = sqlx::query("DELETE FROM events WHERE event_id = $1")
                .bind(fixture.event_a)
                .execute(pool)
                .await;
            let _ = sqlx::query("DELETE FROM chains WHERE chain_id = $1")
                .bind(fixture.chain_a)
                .execute(pool)
                .await;
            for account_id in [fixture.account_a, fixture.account_b] {
                let _ = sqlx::query("DELETE FROM accounts WHERE account_id = $1")
                    .bind(account_id)
                    .execute(pool)
                    .await;
            }
        }
    }

    async fn test_pool() -> PgPool {
        dotenvy::dotenv().ok();
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for event_access tests");
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("test db connection failed")
    }

    #[tokio::test]
    async fn owner_account_can_access_event() {
        let pool = test_pool().await;
        let fixture = EventOwnershipFixture::setup(&pool).await;

        let result = verify_event_access(&pool, fixture.account_a, fixture.event_a).await;
        assert!(result.is_ok(), "owner should access own event");
        assert_eq!(result.unwrap().event_id, fixture.event_a);

        EventOwnershipFixture::teardown(&pool, &fixture).await;
    }

    #[tokio::test]
    async fn foreign_account_gets_not_found() {
        let pool = test_pool().await;
        let fixture = EventOwnershipFixture::setup(&pool).await;

        let result = verify_event_access(&pool, fixture.account_b, fixture.event_a).await;
        assert_eq!(result.unwrap_err(), ApiError::NotFound);

        EventOwnershipFixture::teardown(&pool, &fixture).await;
    }

    #[tokio::test]
    async fn unknown_event_id_gets_not_found() {
        let pool = test_pool().await;
        let fixture = EventOwnershipFixture::setup(&pool).await;

        let result = verify_event_access(&pool, fixture.account_a, Uuid::new_v4()).await;
        assert_eq!(result.unwrap_err(), ApiError::NotFound);

        EventOwnershipFixture::teardown(&pool, &fixture).await;
    }
}
