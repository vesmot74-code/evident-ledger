use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub dev_mode: bool,
    pub trust_proxy_headers: bool,
    pub paddle_webhook_secret: String,
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

        Self {
            dev_mode,
            trust_proxy_headers,
            paddle_webhook_secret,
        }
    }

    #[allow(dead_code)]
    pub fn test_defaults() -> Self {
        Self {
            dev_mode: true,
            trust_proxy_headers: false,
            paddle_webhook_secret: "test-paddle-webhook-secret".into(),
        }
    }
}
