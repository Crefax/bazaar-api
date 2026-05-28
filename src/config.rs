use std::env;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub mongodb_uri: String,
    pub mongodb_db: String,
    pub bind_addr: String,
    pub admin_api_key: Option<String>,
    pub cors_allowed_origins: Vec<String>,
    pub trust_proxy: bool,
    pub redis_url: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Arc<Self> {
        Arc::new(Self {
            mongodb_uri: env::var("MONGODB_URI")
                .unwrap_or_else(|_| "mongodb://localhost:27017".to_string()),
            mongodb_db: env::var("MONGODB_DB").unwrap_or_else(|_| "skyblock".to_string()),
            bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:22417".to_string()),
            admin_api_key: env::var("ADMIN_API_KEY")
                .ok()
                .filter(|value| !value.is_empty()),
            cors_allowed_origins: env::var("CORS_ALLOWED_ORIGINS")
                .ok()
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|origin| !origin.is_empty())
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            trust_proxy: env::var("TRUST_PROXY")
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false),
            redis_url: env::var("REDIS_URL").ok().filter(|value| !value.is_empty()),
        })
    }
}
