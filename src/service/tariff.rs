//! Tariff plan queries for dashboard upgrade UI (Stage 10.2).

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlanPreview {
    pub name: String,
    pub display_name: String,
}

/// Plans above the account's current tier that can be purchased right now.
pub async fn list_upgradeable_plans(
    db: &PgPool,
    account_id: Uuid,
) -> Result<Vec<PlanPreview>, sqlx::Error> {
    sqlx::query_as::<_, PlanPreview>(
        r#"
        SELECT tp.name, tp.display_name
        FROM tariff_plans tp
        WHERE tp.priority > (
            SELECT cur.priority
            FROM accounts a
            JOIN tariff_plans cur ON cur.plan_id = a.tariff_plan_id
            WHERE a.account_id = $1
        )
        AND tp.paddle_price_id IS NOT NULL
        ORDER BY tp.priority
        "#,
    )
    .bind(account_id)
    .fetch_all(db)
    .await
}
