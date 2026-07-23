use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub dev_mode: bool,
    /// Deployment environment label (`development` / `production`).
    pub environment: String,
    /// Filesystem path to the server Ed25519 signing key.
    pub signing_key_path: String,
    pub trust_proxy_headers: bool,
    pub paddle_webhook_secret: String,
    pub paddle_api_key: String,
    pub paddle_api_base_url: String,
    /// Public client-side token for Paddle.js (never confuse with `paddle_api_key`).
    pub paddle_client_token: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let environment = env::var("ENVIRONMENT")
            .ok()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "development".into());

        let dev_mode = env::var("DEV_MODE")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
            || env::var("APP_ENV")
                .map(|v| v.eq_ignore_ascii_case("development"))
                .unwrap_or(false);

        if dev_mode && environment == "production" {
            panic!("DEV_MODE cannot be enabled in production environment");
        }

        let signing_key_from_env = env::var("SIGNING_KEY_PATH")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        if environment == "production" && signing_key_from_env.is_none() {
            panic!("SIGNING_KEY_PATH must be set in production environment");
        }

        let signing_key_path = signing_key_from_env.unwrap_or_else(|| "signing_key.bin".into());

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
            environment,
            signing_key_path,
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
            environment: "development".into(),
            signing_key_path: "signing_key.bin".into(),
            trust_proxy_headers: false,
            paddle_webhook_secret: "test-paddle-webhook-secret".into(),
            paddle_api_key: "test-paddle-api-key".into(),
            paddle_api_base_url: "https://api.paddle.com".into(),
            paddle_client_token: "test_paddle_client_token".into(),
        }
    }

    /// Absolute-ish path for logging (CWD-joined when relative).
    pub fn signing_key_path_display(&self) -> PathBuf {
        let path = PathBuf::from(&self.signing_key_path);
        if path.is_absolute() {
            path
        } else {
            env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| PathBuf::from(&self.signing_key_path))
        }
    }
}
