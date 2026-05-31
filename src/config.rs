use ipnet::IpNet;
use std::env;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub mongodb_uri: String,
    pub mongodb_db: String,
    pub bind_addr: String,
    pub app_env: AppEnvironment,
    pub admin_api_key: Option<String>,
    pub cors_allowed_origins: Vec<String>,
    pub trust_proxy: bool,
    pub trusted_proxy_cidrs: Vec<IpNet>,
    pub redis_url: Option<String>,
    pub require_redis: bool,
    pub api_key_hash_pepper: String,
    pub admin_cookie_secure: bool,
    pub cache_max_entries: usize,
    pub rate_limit_max_keys: usize,
    pub admin_json_limit_bytes: usize,
    pub request_logging: bool,
    pub response_compression: bool,
}

impl AppConfig {
    pub fn from_env() -> Arc<Self> {
        let app_env = env::var("APP_ENV")
            .ok()
            .map(|value| AppEnvironment::from(value.as_str()))
            .unwrap_or(AppEnvironment::Development);
        let admin_cookie_secure = env::var("ADMIN_COOKIE_SECURE")
            .ok()
            .map(|value| truthy(&value))
            .unwrap_or_else(|| app_env.is_production());

        Arc::new(Self {
            mongodb_uri: env::var("MONGODB_URI")
                .unwrap_or_else(|_| "mongodb://localhost:27017".to_string()),
            mongodb_db: env::var("MONGODB_DB").unwrap_or_else(|_| "skyblock".to_string()),
            bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:22417".to_string()),
            app_env,
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
                .map(|value| truthy(&value))
                .unwrap_or(false),
            trusted_proxy_cidrs: env::var("TRUSTED_PROXY_CIDRS")
                .ok()
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|cidr| !cidr.is_empty())
                        .filter_map(|cidr| cidr.parse::<IpNet>().ok())
                        .collect()
                })
                .unwrap_or_default(),
            redis_url: env::var("REDIS_URL").ok().filter(|value| !value.is_empty()),
            require_redis: env::var("REQUIRE_REDIS")
                .ok()
                .map(|value| truthy(&value))
                .unwrap_or(false),
            api_key_hash_pepper: env::var("API_KEY_HASH_PEPPER")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "development-api-key-hash-pepper".to_string()),
            admin_cookie_secure,
            cache_max_entries: parse_usize_env("CACHE_MAX_ENTRIES", 10_000),
            rate_limit_max_keys: parse_usize_env("RATE_LIMIT_MAX_KEYS", 50_000),
            admin_json_limit_bytes: parse_usize_env("ADMIN_JSON_LIMIT_BYTES", 16 * 1024),
            request_logging: env::var("REQUEST_LOGGING")
                .ok()
                .map(|value| truthy(&value))
                .unwrap_or_else(|| !app_env.is_production()),
            response_compression: env::var("RESPONSE_COMPRESSION")
                .ok()
                .map(|value| truthy(&value))
                .unwrap_or_else(|| !app_env.is_production()),
        })
    }

    pub fn redis_required(&self) -> bool {
        self.require_redis || self.app_env.is_production()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnvironment {
    Development,
    Production,
}

impl AppEnvironment {
    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

impl From<&str> for AppEnvironment {
    fn from(value: &str) -> Self {
        if value.eq_ignore_ascii_case("production") || value.eq_ignore_ascii_case("prod") {
            Self::Production
        } else {
            Self::Development
        }
    }
}

fn parse_usize_env(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
}
