//! HTTP client for Paddle Billing API (Stage 8.3.2).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode as HttpStatus;
use serde_json::json;

use crate::config::AppConfig;

#[derive(Debug, Clone)]
pub enum PaddleClientError {
    Timeout,
    NetworkError(String),
    ApiError { status: u16, message: String },
    InvalidResponse,
}

#[async_trait]
pub trait PaddleClient: Send + Sync {
    async fn create_customer(&self, email: &str) -> Result<String, PaddleClientError>;
    async fn create_checkout(
        &self,
        customer_id: &str,
        price_id: &str,
    ) -> Result<String, PaddleClientError>;
}

pub struct HttpPaddleClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl HttpPaddleClient {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("paddle http client"),
            api_key: config.paddle_api_key.clone(),
            base_url: config.paddle_api_base_url.clone(),
        }
    }

    fn auth_request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base_url.trim_end_matches('/')))
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
    }
}

#[async_trait]
impl PaddleClient for HttpPaddleClient {
    async fn create_customer(&self, email: &str) -> Result<String, PaddleClientError> {
        let response = self
            .auth_request(reqwest::Method::POST, "/customers")
            .json(&json!({ "email": email }))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        parse_customer_response(response).await
    }

    async fn create_checkout(
        &self,
        customer_id: &str,
        price_id: &str,
    ) -> Result<String, PaddleClientError> {
        let response = self
            .auth_request(reqwest::Method::POST, "/transactions")
            .json(&json!({
                "customer_id": customer_id,
                "items": [{ "price_id": price_id, "quantity": 1 }]
            }))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        parse_checkout_response(response).await
    }
}

#[derive(Default)]
pub struct MockPaddleClient {
    create_customer_calls: AtomicUsize,
    simulate_timeout: AtomicBool,
    fail_create: AtomicBool,
    fail_checkout: AtomicBool,
    create_delay_ms: AtomicUsize,
}

impl MockPaddleClient {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn create_customer_calls(&self) -> usize {
        self.create_customer_calls.load(Ordering::SeqCst)
    }

    pub fn set_simulate_timeout(&self, value: bool) {
        self.simulate_timeout.store(value, Ordering::SeqCst);
    }

    pub fn set_fail_create(&self, value: bool) {
        self.fail_create.store(value, Ordering::SeqCst);
    }

    pub fn set_fail_checkout(&self, value: bool) {
        self.fail_checkout.store(value, Ordering::SeqCst);
    }

    pub fn set_create_delay_ms(&self, value: usize) {
        self.create_delay_ms.store(value, Ordering::SeqCst);
    }
}

#[async_trait]
impl PaddleClient for MockPaddleClient {
    async fn create_customer(&self, email: &str) -> Result<String, PaddleClientError> {
        self.create_customer_calls.fetch_add(1, Ordering::SeqCst);
        if self.simulate_timeout.load(Ordering::SeqCst) {
            return Err(PaddleClientError::Timeout);
        }
        if self.fail_create.load(Ordering::SeqCst) {
            return Err(PaddleClientError::ApiError {
                status: 503,
                message: "service unavailable".into(),
            });
        }
        let delay = self.create_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay as u64)).await;
        }
        Ok(format!("ctm_mock_{}", email.replace('@', "_at_")))
    }

    async fn create_checkout(
        &self,
        customer_id: &str,
        price_id: &str,
    ) -> Result<String, PaddleClientError> {
        if self.simulate_timeout.load(Ordering::SeqCst) {
            return Err(PaddleClientError::Timeout);
        }
        if self.fail_checkout.load(Ordering::SeqCst) {
            return Err(PaddleClientError::ApiError {
                status: 503,
                message: "checkout unavailable".into(),
            });
        }
        Ok(format!(
            "https://paddle.example/checkout/{customer_id}/{price_id}"
        ))
    }
}

fn map_reqwest_error(err: reqwest::Error) -> PaddleClientError {
    if err.is_timeout() {
        PaddleClientError::Timeout
    } else {
        PaddleClientError::NetworkError(err.to_string())
    }
}

async fn parse_customer_response(
    response: reqwest::Response,
) -> Result<String, PaddleClientError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| PaddleClientError::NetworkError(err.to_string()))?;
    if !status.is_success() {
        return Err(PaddleClientError::ApiError {
            status: status.as_u16(),
            message: body,
        });
    }
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| PaddleClientError::InvalidResponse)?;
    json.pointer("/data/id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or(PaddleClientError::InvalidResponse)
}

async fn parse_checkout_response(
    response: reqwest::Response,
) -> Result<String, PaddleClientError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| PaddleClientError::NetworkError(err.to_string()))?;
    if !status.is_success() {
        return Err(PaddleClientError::ApiError {
            status: status.as_u16(),
            message: body,
        });
    }
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|_| PaddleClientError::InvalidResponse)?;
    json.pointer("/data/checkout/url")
        .or_else(|| json.pointer("/data/details/checkout/url"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or(PaddleClientError::InvalidResponse)
}

pub fn paddle_client_error_is_unavailable(err: &PaddleClientError) -> bool {
    match err {
        PaddleClientError::Timeout | PaddleClientError::NetworkError(_) => true,
        PaddleClientError::ApiError { status, .. } => *status >= 500,
        PaddleClientError::InvalidResponse => false,
    }
}
