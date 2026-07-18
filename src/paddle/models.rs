//! Paddle Billing webhook payload models (Stage 8.2b).

use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PaddleWebhookEvent {
    pub event_id: String,
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub data: PaddleEventData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaddleEventData {
    pub id: Option<String>,
    pub customer_id: Option<String>,
    pub customer: Option<PaddleCustomerRef>,
    pub current_billing_period: Option<PaddleBillingPeriod>,
    pub items: Option<Vec<PaddleSubscriptionItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaddleCustomerRef {
    pub id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaddleBillingPeriod {
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaddleSubscriptionItem {
    pub price: Option<PaddlePriceRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaddlePriceRef {
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct TariffPlanRow {
    pub plan_id: uuid::Uuid,
    pub name: String,
    pub priority: i32,
}

impl PaddleWebhookEvent {
    pub fn normalized_event_type(&self) -> String {
        self.event_type.replace('.', "_").to_lowercase()
    }

    pub fn customer_id(&self) -> Option<&str> {
        self.data
            .customer_id
            .as_deref()
            .or_else(|| self.data.customer.as_ref()?.id.as_deref())
    }

    pub fn subscription_id(&self) -> Option<&str> {
        self.data.id.as_deref()
    }

    pub fn price_id(&self) -> Option<&str> {
        self.data
            .items
            .as_ref()?
            .first()?
            .price
            .as_ref()
            .map(|p| p.id.as_str())
    }

    pub fn period_end(&self) -> Option<DateTime<Utc>> {
        self.data
            .current_billing_period
            .as_ref()
            .map(|p| p.ends_at)
    }
}

pub fn is_upgrade(old_plan: &TariffPlanRow, new_plan: &TariffPlanRow) -> bool {
    new_plan.priority > old_plan.priority
}

pub fn is_downgrade(old_plan: &TariffPlanRow, new_plan: &TariffPlanRow) -> bool {
    new_plan.priority < old_plan.priority
}
