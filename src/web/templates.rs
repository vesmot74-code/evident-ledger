use askama::Template;

#[derive(Template)]
#[template(path = "dashboard/index.html")]
pub struct DashboardIndexTemplate {
    pub email: String,
    pub plan_display: String,
    pub usage_summary: String,
    pub percentage: String,
    pub can_upgrade: bool,
}

#[derive(Template)]
#[template(path = "dashboard/subscription.html")]
pub struct SubscriptionTemplate {
    pub plan_display: String,
    pub plan: String,
    pub subscription_status: String,
    pub current_period_end: String,
    pub pending_plan_display: String,
    pub can_upgrade: bool,
}

#[derive(Template)]
#[template(path = "dashboard/usage.html")]
pub struct UsageTemplate {
    pub period: String,
    pub usage_summary: String,
    pub percentage: String,
}

#[derive(Template)]
#[template(path = "dashboard/api_keys.html")]
pub struct ApiKeysTemplate {
    pub api_keys: Vec<ApiKeyRow>,
}

#[derive(Debug, Clone)]
pub struct ApiKeyRow {
    pub key_id: String,
    pub prefix: String,
    pub created_at: String,
    pub is_active: bool,
}

#[derive(Template)]
#[template(path = "dashboard/login.html")]
pub struct LoginTemplate;

#[derive(Template)]
#[template(path = "dashboard/api_key_created.html")]
pub struct ApiKeyCreatedTemplate {
    pub api_key: String,
}

#[derive(Template)]
#[template(path = "dashboard/api_key_revoked.html")]
pub struct ApiKeyRevokedTemplate;

#[derive(Template)]
#[template(path = "dashboard/identity_key_revoked.html")]
pub struct IdentityKeyRevokedTemplate;

#[derive(Template)]
#[template(path = "dashboard/identity_keys.html")]
pub struct IdentityKeysTemplate {
    pub keys: Vec<IdentityKeyRow>,
}

#[derive(Debug, Clone)]
pub struct IdentityKeyRow {
    pub key_id: String,
    pub fingerprint: String,
    pub status: String,
    pub created_at: String,
    pub verified_at: String,
    pub revoked_at: String,
    pub events_count: String,
}

#[derive(Template)]
#[template(path = "dashboard/identity_key_events.html")]
pub struct IdentityKeyEventsTemplate {
    pub key_id: String,
    pub key_status: String,
    pub events: Vec<IdentityKeyEventRow>,
    pub next_page_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IdentityKeyEventRow {
    pub event_id: String,
    pub chain_id: String,
    pub sequence: String,
    pub signed_at: String,
    pub signature_valid: bool,
}

pub fn format_usage_summary(server_commits: i32, monthly_limit: Option<i32>) -> String {
    match monthly_limit {
        Some(limit) => format!("{server_commits} / {limit}"),
        None => format!("{server_commits} / unlimited"),
    }
}

pub fn format_percentage(percentage: Option<i32>) -> String {
    match percentage {
        Some(value) => format!("{value}%"),
        None => "—".to_string(),
    }
}

pub fn format_optional_datetime(value: Option<chrono::DateTime<chrono::Utc>>) -> String {
    value
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "—".to_string())
}

pub fn format_optional_text(value: Option<&str>) -> String {
    value.unwrap_or("—").to_string()
}
