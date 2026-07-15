use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TsaMode {
    Machine,
    Qualified,
}

impl TsaMode {
    fn from_db(s: &str) -> Self {
        match s {
            "qualified" => TsaMode::Qualified,
            _ => TsaMode::Machine,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountCapabilities {
    pub plan_name: String,
    pub tsa_mode: TsaMode,
    pub server_backup: bool,
    pub history_recovery: bool,
    pub identity_enabled: bool,
    pub monthly_commits_limit: Option<i32>,
    pub monthly_tsa_limit: Option<i32>,
}

impl AccountCapabilities {
    /// true, если можно зафиксировать ещё одно событие в этом месяце.
    /// `current_commits` — уже совершённые в этом месяце коммиты (usage_monthly.server_commits).
    pub fn can_commit(&self, current_commits: i32) -> bool {
        match self.monthly_commits_limit {
            Some(limit) => current_commits < limit,
            None => true,
        }
    }

    /// true, если можно поставить ещё один TSA-штамп в этом месяце.
    /// `current_tsa_requests` — уже совершённые в этом месяце TSA-запросы (usage_monthly.tsa_requests).
    pub fn can_use_tsa(&self, current_tsa_requests: i32) -> bool {
        match self.monthly_tsa_limit {
            Some(limit) => current_tsa_requests < limit,
            None => true,
        }
    }

    /// true, если запрошенный tsa_mode тарифа реально доступен прямо сейчас.
    /// Пока подключён только machine-режим (freetsa.org) — qualified TSA
    /// ещё не имеет реального провайдера.
    pub fn tsa_available(&self) -> bool {
        self.tsa_mode == TsaMode::Machine
    }
}

pub async fn get_account_capabilities(
    pool: &PgPool,
    account_id: Uuid,
) -> Result<AccountCapabilities, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT
            tp.name AS plan_name,
            tp.tsa_mode,
            tp.server_backup,
            tp.history_recovery,
            tp.identity_enabled,
            tp.monthly_commits_limit,
            tp.monthly_tsa_limit
        FROM accounts a
        JOIN tariff_plans tp ON tp.plan_id = a.tariff_plan_id
        WHERE a.account_id = $1
        "#,
        account_id
    )
    .fetch_one(pool)
    .await?;

    Ok(AccountCapabilities {
        plan_name: row.plan_name,
        tsa_mode: TsaMode::from_db(&row.tsa_mode),
        server_backup: row.server_backup,
        history_recovery: row.history_recovery,
        identity_enabled: row.identity_enabled,
        monthly_commits_limit: row.monthly_commits_limit,
        monthly_tsa_limit: row.monthly_tsa_limit,
    })
}
