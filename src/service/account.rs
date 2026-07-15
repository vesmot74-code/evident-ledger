use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub plan: String,
    pub period_start: chrono::NaiveDate,
    pub server_commits: i32,
    pub monthly_commits_limit: Option<i32>,
    pub tsa_requests: i32,
    pub monthly_tsa_limit: Option<i32>,
}

pub async fn get_usage(pool: &PgPool, account_id: Uuid) -> Result<UsageResponse, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT
            tp.name AS plan_name,
            tp.monthly_commits_limit,
            tp.monthly_tsa_limit,
            date_trunc('month', now())::date AS "period_start!",
            COALESCE(um.server_commits, 0) AS "server_commits!",
            COALESCE(um.tsa_requests, 0) AS "tsa_requests!"
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        LEFT JOIN usage_monthly um
            ON um.account_id = a.account_id
            AND um.period_start = date_trunc('month', now())::date
        WHERE a.account_id = $1
        "#,
        account_id
    )
    .fetch_one(pool)
    .await?;

    Ok(UsageResponse {
        plan: row.plan_name,
        period_start: row.period_start,
        server_commits: row.server_commits,
        monthly_commits_limit: row.monthly_commits_limit,
        tsa_requests: row.tsa_requests,
        monthly_tsa_limit: row.monthly_tsa_limit,
    })
}
