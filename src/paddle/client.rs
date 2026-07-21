//! HTTP client for Paddle Billing API (Stage 8.3.2).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::config::AppConfig;

#[derive(Debug, Clone)]
pub enum PaddleClientError {
    Timeout,
    NetworkError(String),
    ApiError { status: u16, message: String },
    InvalidResponse,
}

#[derive(Debug, Clone)]
pub struct CheckoutSession {
    pub checkout_url: String,
    pub transaction_id: String,
}

#[async_trait]
pub trait PaddleClient: Send + Sync {
    async fn create_customer(&self, email: &str) -> Result<String, PaddleClientError>;
    async fn create_checkout(
        &self,
        customer_id: &str,
        price_id: &str,
    ) -> Result<CheckoutSession, PaddleClientError>;
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
            .request(
                method,
                format!("{}{path}", self.base_url.trim_end_matches('/')),
            )
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
    }

    async fn find_customer_id_by_email(&self, email: &str) -> Result<String, PaddleClientError> {
        let response = self
            .auth_request(reqwest::Method::GET, "/customers")
            .query(&[("email", email)])
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| PaddleClientError::NetworkError(err.to_string()))?;
        if !status.is_success() {
            let message = format_paddle_error_message(status.as_u16(), &body);
            tracing::error!(
                status = status.as_u16(),
                body = %body,
                message = %message,
                "paddle api error"
            );
            return Err(PaddleClientError::ApiError {
                status: status.as_u16(),
                message,
            });
        }

        let json: serde_json::Value =
            serde_json::from_str(&body).map_err(|_| PaddleClientError::InvalidResponse)?;
        json.pointer("/data/0/id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or(PaddleClientError::InvalidResponse)
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

        match parse_customer_response(response).await {
            Ok(id) => Ok(id),
            Err(PaddleClientError::ApiError { status, message })
                if status == 409 || message.contains("conflicts with customer") =>
            {
                self.find_customer_id_by_email(email).await
            }
            Err(err) => Err(err),
        }
    }

    async fn create_checkout(
        &self,
        customer_id: &str,
        price_id: &str,
    ) -> Result<CheckoutSession, PaddleClientError> {
        let response = self
            .auth_request(reqwest::Method::POST, "/transactions")
            .json(&json!({
                "customer_id": customer_id,
                "items": [{ "price_id": price_id, "quantity": 1 }],
                "collection_mode": "automatic"
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
    last_checkout: Mutex<Option<(String, String)>>,
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

    pub fn last_checkout(&self) -> Option<(String, String)> {
        self.last_checkout.lock().ok()?.clone()
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
    ) -> Result<CheckoutSession, PaddleClientError> {
        if let Ok(mut last) = self.last_checkout.lock() {
            *last = Some((customer_id.to_string(), price_id.to_string()));
        }
        if self.simulate_timeout.load(Ordering::SeqCst) {
            return Err(PaddleClientError::Timeout);
        }
        if self.fail_checkout.load(Ordering::SeqCst) {
            return Err(PaddleClientError::ApiError {
                status: 503,
                message: "checkout unavailable".into(),
            });
        }
        let transaction_id = format!("txn_mock_{price_id}");
        Ok(CheckoutSession {
            checkout_url: format!(
                "https://paddle.example/checkout/{customer_id}/{price_id}?_ptxn={transaction_id}"
            ),
            transaction_id,
        })
    }
}

fn map_reqwest_error(err: reqwest::Error) -> PaddleClientError {
    if err.is_timeout() {
        PaddleClientError::Timeout
    } else {
        PaddleClientError::NetworkError(err.to_string())
    }
}

fn format_paddle_error_message(status: u16, body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let code = json.pointer("/error/code").and_then(|value| value.as_str());
        let detail = json
            .pointer("/error/detail")
            .and_then(|value| value.as_str());
        match (code, detail) {
            (Some(code), Some(detail)) => {
                return format!("status={status} code={code} detail={detail} body={body}");
            }
            (Some(code), None) => {
                return format!("status={status} code={code} body={body}");
            }
            (None, Some(detail)) => {
                return format!("status={status} detail={detail} body={body}");
            }
            (None, None) => {}
        }
    }
    format!("status={status} body={body}")
}

async fn parse_customer_response(response: reqwest::Response) -> Result<String, PaddleClientError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| PaddleClientError::NetworkError(err.to_string()))?;
    if !status.is_success() {
        let message = format_paddle_error_message(status.as_u16(), &body);
        tracing::error!(
            status = status.as_u16(),
            body = %body,
            message = %message,
            "paddle api error"
        );
        return Err(PaddleClientError::ApiError {
            status: status.as_u16(),
            message,
        });
    }
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| PaddleClientError::InvalidResponse)?;
    json.pointer("/data/id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or(PaddleClientError::InvalidResponse)
}

fn extract_checkout_url(json: &serde_json::Value) -> Option<String> {
    const CANDIDATES: &[&str] = &[
        "/data/checkout/url",
        "/data/details/checkout/url",
        "/checkout/url",
    ];
    for path in CANDIDATES {
        if let Some(url) = json.pointer(path).and_then(|value| value.as_str()) {
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }
    None
}

async fn parse_checkout_response(
    response: reqwest::Response,
) -> Result<CheckoutSession, PaddleClientError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| PaddleClientError::NetworkError(err.to_string()))?;
    if !status.is_success() {
        let message = format_paddle_error_message(status.as_u16(), &body);
        tracing::error!(
            status = status.as_u16(),
            body = %body,
            message = %message,
            "paddle api error"
        );
        return Err(PaddleClientError::ApiError {
            status: status.as_u16(),
            message,
        });
    }
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| PaddleClientError::InvalidResponse)?;
    let transaction_id = json
        .pointer("/data/id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let checkout_url = extract_checkout_url(&json);
    match (checkout_url, transaction_id) {
        (Some(checkout_url), Some(transaction_id)) => Ok(CheckoutSession {
            checkout_url,
            transaction_id,
        }),
        _ => {
            tracing::error!(
                status = status.as_u16(),
                body = %body,
                "paddle api error"
            );
            Err(PaddleClientError::InvalidResponse)
        }
    }
}

pub fn paddle_client_error_is_unavailable(err: &PaddleClientError) -> bool {
    match err {
        PaddleClientError::Timeout | PaddleClientError::NetworkError(_) => true,
        PaddleClientError::ApiError { status, .. } => *status >= 500,
        PaddleClientError::InvalidResponse => false,
    }
}
