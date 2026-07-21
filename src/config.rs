use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub dev_mode: bool,
    pub trust_proxy_headers: bool,
    pub paddle_webhook_secret: String,
    pub paddle_api_key: String,
    pub paddle_api_base_url: String,
    /// Public client-side token for Paddle.js (never confuse with `paddle_api_key`).
    pub paddle_client_token: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let dev_mode = env::var("DEV_MODE")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
            || env::var("APP_ENV")
                .map(|v| v.eq_ignore_ascii_case("development"))
                .unwrap_or(false);

        let trust_proxy_headers = env::var("TRUST_PROXY_HEADERS")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);

        let paddle_webhook_secret = env::var("PADDLE_WEBHOOK_SECRET").unwrap_or_else(|_| {
            #[cfg(test)]
            {
                return "test-paddle-webhook-secret".into();
            }
            #[cfg(not(test))]
            {
                panic!("PADDLE_WEBHOOK_SECRET must be set");
            }
        });

        let paddle_api_key = env::var("PADDLE_API_KEY").unwrap_or_else(|_| {
            #[cfg(test)]
            {
                return "test-paddle-api-key".into();
            }
            #[cfg(not(test))]
            {
                panic!("PADDLE_API_KEY must be set");
            }
        });

        let paddle_api_base_url =
            env::var("PADDLE_API_BASE_URL").unwrap_or_else(|_| "https://api.paddle.com".into());

        let paddle_client_token = env::var("PADDLE_CLIENT_TOKEN").unwrap_or_else(|_| {
            #[cfg(test)]
            {
                return "test_paddle_client_token".into();
            }
            #[cfg(not(test))]
            {
                panic!("PADDLE_CLIENT_TOKEN must be set");
            }
        });

        Self {
            dev_mode,
            trust_proxy_headers,
            paddle_webhook_secret,
            paddle_api_key,
            paddle_api_base_url,
            paddle_client_token,
        }
    }

    pub fn paddle_environment(&self) -> &'static str {
        if self.paddle_api_base_url.contains("sandbox") {
            "sandbox"
        } else {
            "production"
        }
    }

    #[allow(dead_code)]
    pub fn test_defaults() -> Self {
        Self {
            dev_mode: true,
            trust_proxy_headers: false,
            paddle_webhook_secret: "test-paddle-webhook-secret".into(),
            paddle_api_key: "test-paddle-api-key".into(),
            paddle_api_base_url: "https://api.paddle.com".into(),
            paddle_client_token: "test_paddle_client_token".into(),
        }
    }
}
