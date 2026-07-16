use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub dev_mode: bool,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let dev_mode = env::var("DEV_MODE")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
            || env::var("APP_ENV")
                .map(|v| v.eq_ignore_ascii_case("development"))
                .unwrap_or(false);

        Self { dev_mode }
    }
}
