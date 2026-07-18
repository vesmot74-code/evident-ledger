//! Account registration and API key persistence (Stage 8.1).

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::auth::api_key::{self, GeneratedApiKey};

#[derive(Debug, Clone)]
pub struct RegisterResult {
    pub account_id: Uuid,
    pub api_key: String,
    pub tariff_plan_id: Uuid,
    pub plan_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AccountProfile {
    pub account_id: Uuid,
    pub email: String,
    pub tariff_plan_id: Uuid,
    pub plan_name: String,
    pub subscription_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DashboardProfile {
    pub account_id: Uuid,
    pub email: String,
    pub plan_name: String,
    pub plan_display_name: String,
    pub subscription_status: String,
    pub created_at: DateTime<Utc>,
    pub email_verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct SubscriptionSnapshot {
    pub plan_name: String,
    pub plan_display_name: String,
    pub subscription_status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub pending_plan_name: Option<String>,
    pub pending_plan_display_name: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MonthlyUsageSnapshot {
    pub period_start: chrono::NaiveDate,
    pub server_commits: i32,
    pub monthly_commits_limit: Option<i32>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ApiKeyRecord {
    pub api_key_id: Uuid,
    pub key_prefix: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct FreePlanRow {
    plan_id: Uuid,
    name: String,
}

#[derive(Debug, FromRow)]
struct CreatedAccountRow {
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub enum RegisterError {
    EmailAlreadyRegistered,
    Database(String),
}

#[derive(Debug)]
pub enum RevokeApiKeyError {
    LastActiveKey,
    NotFound,
    Database(String),
}

pub fn is_valid_email(email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() || email.len() > 254 {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

pub async fn register_account(pool: &PgPool, email: &str) -> Result<RegisterResult, RegisterError> {
    let email = email.trim().to_lowercase();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| RegisterError::Database(e.to_string()))?;

    if email_exists(&mut tx, &email).await? {
        return Err(RegisterError::EmailAlreadyRegistered);
    }

    let free_plan = sqlx::query_as::<_, FreePlanRow>(
        r#"
        SELECT plan_id, name
        FROM tariff_plans
        WHERE name = 'free'
        "#,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| RegisterError::Database(e.to_string()))?
    .ok_or_else(|| RegisterError::Database("free tariff plan missing".into()))?;

    let account_id = Uuid::new_v4();
    let created = sqlx::query_as::<_, CreatedAccountRow>(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id)
        VALUES ($1, $2, $3)
        RETURNING created_at
        "#,
    )
    .bind(account_id)
    .bind(&email)
    .bind(free_plan.plan_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| RegisterError::Database(e.to_string()))?;

    let generated = api_key::generate_api_key();
    insert_api_key(&mut tx, account_id, &generated, "default")
        .await
        .map_err(|e| RegisterError::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| RegisterError::Database(e.to_string()))?;

    Ok(RegisterResult {
        account_id,
        api_key: generated.full_key,
        tariff_plan_id: free_plan.plan_id,
        plan_name: free_plan.name,
        created_at: created.created_at,
    })
}

#[derive(Debug, Clone)]
pub struct WebRegisterResult {
    pub account_id: Uuid,
    pub email: String,
    pub plan_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub enum WebRegisterError {
    EmailAlreadyRegistered,
    Database(String),
}

#[derive(Debug)]
pub enum SetPasswordError {
    PasswordAlreadySet,
    NotFound,
    Database(String),
}

pub async fn register_web_account(
    pool: &PgPool,
    email: &str,
    password_hash: &str,
) -> Result<WebRegisterResult, WebRegisterError> {
    let email = email.trim().to_lowercase();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| WebRegisterError::Database(e.to_string()))?;

    if email_exists(&mut tx, &email)
        .await
        .map_err(|e| match e {
            RegisterError::Database(msg) => WebRegisterError::Database(msg),
            RegisterError::EmailAlreadyRegistered => WebRegisterError::EmailAlreadyRegistered,
        })?
    {
        return Err(WebRegisterError::EmailAlreadyRegistered);
    }

    let free_plan = sqlx::query_as::<_, FreePlanRow>(
        r#"
        SELECT plan_id, name
        FROM tariff_plans
        WHERE name = 'free'
        "#,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| WebRegisterError::Database(e.to_string()))?
    .ok_or_else(|| WebRegisterError::Database("free tariff plan missing".into()))?;

    let account_id = Uuid::new_v4();
    let created = sqlx::query_as::<_, CreatedAccountRow>(
        r#"
        INSERT INTO accounts (account_id, email, tariff_plan_id, password_hash, subscription_status)
        VALUES ($1, $2, $3, $4, 'none')
        RETURNING created_at
        "#,
    )
    .bind(account_id)
    .bind(&email)
    .bind(free_plan.plan_id)
    .bind(password_hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| WebRegisterError::Database(e.to_string()))?;

    let generated = api_key::generate_api_key();
    insert_api_key(&mut tx, account_id, &generated, "default")
        .await
        .map_err(|e| WebRegisterError::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| WebRegisterError::Database(e.to_string()))?;

    Ok(WebRegisterResult {
        account_id,
        email,
        plan_name: free_plan.name,
        created_at: created.created_at,
    })
}

pub async fn set_account_password(
    pool: &PgPool,
    account_id: Uuid,
    password_hash: &str,
) -> Result<(), SetPasswordError> {
    let updated = sqlx::query(
        r#"
        UPDATE accounts
        SET password_hash = $2
        WHERE account_id = $1 AND password_hash IS NULL
        "#,
    )
    .bind(account_id)
    .bind(password_hash)
    .execute(pool)
    .await
    .map_err(|e| SetPasswordError::Database(e.to_string()))?;

    if updated.rows_affected() == 1 {
        return Ok(());
    }

    let has_password: bool = sqlx::query_scalar(
        "SELECT password_hash IS NOT NULL FROM accounts WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| SetPasswordError::Database(e.to_string()))?
    .ok_or(SetPasswordError::NotFound)?;

    if has_password {
        Err(SetPasswordError::PasswordAlreadySet)
    } else {
        Err(SetPasswordError::NotFound)
    }
}

pub async fn get_account_profile(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<Option<AccountProfile>, sqlx::Error> {
    sqlx::query_as::<_, AccountProfile>(
        r#"
        SELECT
            a.account_id,
            a.email,
            a.tariff_plan_id,
            tp.name AS plan_name,
            a.subscription_status,
            a.created_at
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_dashboard_profile(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<Option<DashboardProfile>, sqlx::Error> {
    sqlx::query_as::<_, DashboardProfile>(
        r#"
        SELECT
            a.account_id,
            a.email,
            tp.name AS plan_name,
            tp.display_name AS plan_display_name,
            a.subscription_status,
            a.created_at,
            a.email_verified_at
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_subscription_snapshot(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<Option<SubscriptionSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, SubscriptionSnapshot>(
        r#"
        SELECT
            tp.name AS plan_name,
            tp.display_name AS plan_display_name,
            a.subscription_status,
            a.current_period_end,
            pending.name AS pending_plan_name,
            pending.display_name AS pending_plan_display_name
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        LEFT JOIN tariff_plans pending ON pending.plan_id = a.pending_tariff_plan_id
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_monthly_usage_snapshot(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<Option<MonthlyUsageSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, MonthlyUsageSnapshot>(
        r#"
        SELECT
            date_trunc('month', now())::date AS period_start,
            COALESCE(um.server_commits, 0) AS server_commits,
            tp.monthly_commits_limit
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        LEFT JOIN usage_monthly um
            ON um.account_id = a.account_id
           AND um.period_start = date_trunc('month', now())::date
        WHERE a.account_id = $1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_api_keys(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<Vec<ApiKeyRecord>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRecord>(
        r#"
        SELECT api_key_id, key_prefix, label, created_at, revoked_at
        FROM api_keys
        WHERE account_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

pub async fn create_api_key(
    pool: &PgPool,
    account_id: Uuid,
    label: &str,
) -> Result<(GeneratedApiKey, ApiKeyRecord), sqlx::Error> {
    let generated = api_key::generate_api_key();
    let mut tx = pool.begin().await?;
    insert_api_key(&mut tx, account_id, &generated, label).await?;
    let record = sqlx::query_as::<_, ApiKeyRecord>(
        r#"
        SELECT api_key_id, key_prefix, label, created_at, revoked_at
        FROM api_keys
        WHERE key_hash = $1
        "#,
    )
    .bind(&generated.key_hash)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((generated, record))
}

pub async fn revoke_api_key(
    pool: &PgPool,
    account_id: Uuid,
    api_key_id: Uuid,
) -> Result<(), RevokeApiKeyError> {
    let active_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM api_keys
        WHERE account_id = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|e| RevokeApiKeyError::Database(e.to_string()))?;

    let owns_key: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM api_keys
            WHERE api_key_id = $1 AND account_id = $2 AND revoked_at IS NULL
        )
        "#,
    )
    .bind(api_key_id)
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|e| RevokeApiKeyError::Database(e.to_string()))?;

    if !owns_key {
        return Err(RevokeApiKeyError::NotFound);
    }

    if active_count <= 1 {
        return Err(RevokeApiKeyError::LastActiveKey);
    }

    let updated = sqlx::query(
        r#"
        UPDATE api_keys
        SET revoked_at = now()
        WHERE api_key_id = $1
          AND account_id = $2
          AND revoked_at IS NULL
        "#,
    )
    .bind(api_key_id)
    .bind(account_id)
    .execute(pool)
    .await
    .map_err(|e| RevokeApiKeyError::Database(e.to_string()))?;

    if updated.rows_affected() == 0 {
        return Err(RevokeApiKeyError::NotFound);
    }

    Ok(())
}

async fn email_exists(
    tx: &mut Transaction<'_, Postgres>,
    email: &str,
) -> Result<bool, RegisterError> {
    let exists: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM accounts WHERE email = $1
        )
        "#,
    )
    .bind(email)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| RegisterError::Database(e.to_string()))?;

    Ok(exists)
}

async fn insert_api_key(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
    generated: &GeneratedApiKey,
    label: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO api_keys (account_id, key_hash, key_prefix, label)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(account_id)
    .bind(&generated.key_hash)
    .bind(&generated.key_prefix)
    .bind(label)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
