use askama::Template;

use crate::service::tariff::PlanPreview;

#[derive(Template)]
#[template(path = "dashboard/index.html")]
pub struct DashboardIndexTemplate {
    pub email: String,
    pub plan_display: String,
    pub subscription_status: String,
    pub trust_level: String,
    pub usage_summary: String,
    pub percentage: String,
    /// True when the account has not yet recorded any server commits this period
    /// (and has no usage row) — used for first-run onboarding only.
    pub show_onboarding: bool,
    pub can_upgrade: bool,
    pub available_plans: Vec<PlanPreview>,
    pub paddle_client_token: String,
    pub paddle_environment: String,
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
    pub available_plans: Vec<PlanPreview>,
    pub paddle_client_token: String,
    pub paddle_environment: String,
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
#[template(path = "dashboard/register.html")]
pub struct RegisterTemplate;

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

pub fn trust_level_label(plan_name: &str) -> &'static str {
    match plan_name {
        "identity" => "IDENTITY",
        "vault" => "VAULT",
        "legal" => "ENHANCED",
        _ => "BASIC",
    }
}

/// Human-readable plan label for dashboard presentation (no billing changes).
pub fn format_plan_label(plan_name: &str, plan_display_name: &str) -> String {
    if plan_name == "free" {
        "Free plan".to_string()
    } else {
        plan_display_name.to_string()
    }
}

/// Human-readable subscription status for dashboard presentation.
pub fn format_subscription_status_label(status: &str) -> String {
    match status {
        "none" => "No subscription".to_string(),
        "active" => "Active".to_string(),
        "past_due" => "Past due".to_string(),
        "canceled" | "cancelled" => "Canceled".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_plan_label_is_user_friendly() {
        assert_eq!(format_plan_label("free", "Free"), "Free plan");
        assert_eq!(format_plan_label("free", "Бесплатно"), "Free plan");
        assert_eq!(format_plan_label("legal", "Legal"), "Legal");
    }

    #[test]
    fn none_subscription_is_not_raw_none() {
        assert_eq!(format_subscription_status_label("none"), "No subscription");
        assert_ne!(format_subscription_status_label("none"), "none");
        assert_ne!(format_subscription_status_label("none"), "нет");
        assert_eq!(format_subscription_status_label("active"), "Active");
    }
}
