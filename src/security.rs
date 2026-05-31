use crate::config::AppConfig;
use crate::db;
use crate::models::{AccessPolicy, ApiKeyRecord, ProblemResponse};
use crate::shared_store::{DailyQuotaOutcome, RateLimitOutcome};
use crate::state::AppState;
use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, HttpResponse};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

pub const USER_API_KEY_HEADER: &str = "X-API-Key";
pub const ADMIN_API_KEY_HEADER: &str = "X-Admin-Api-Key";
pub const ADMIN_CSRF_HEADER: &str = "X-CSRF-Token";
pub const ADMIN_SESSION_COOKIE: &str = "bazaar_admin_session";
pub const SECURE_ADMIN_SESSION_COOKIE: &str = "__Host-bazaar_admin_session";

const ACCESS_POLICY_CACHE_TTL: Duration = Duration::from_secs(30);
const API_KEY_RECORD_CACHE_TTL: Duration = Duration::from_secs(30);
const API_KEY_LAST_USED_TOUCH_TTL: Duration = Duration::from_secs(60);
const ADMIN_LOGIN_MINUTE_LIMIT: u32 = 5;
const ADMIN_LOGIN_HOUR_LIMIT: u32 = 50;
const ADMIN_API_RATE_LIMIT: u32 = 120;

#[derive(Debug, Clone)]
pub struct RequestAuth {
    pub rate_limit: RateLimitOutcome,
    pub daily_quota: Option<DailyQuotaOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyHashStatus {
    Current,
    Legacy,
    Invalid,
}

pub fn hash_api_key(key: &str, config: &AppConfig) -> String {
    let mut mac = HmacSha256::new_from_slice(config.api_key_hash_pepper.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(key.as_bytes());
    format!("hmac-sha256:{}", hex::encode(mac.finalize().into_bytes()))
}

pub fn legacy_hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn verify_api_key_hash(
    stored_hash: &str,
    raw_key: &str,
    config: &AppConfig,
) -> ApiKeyHashStatus {
    let current_hash = hash_api_key(raw_key, config);
    if constant_time_eq(stored_hash, &current_hash) {
        return ApiKeyHashStatus::Current;
    }

    let legacy_hash = legacy_hash_api_key(raw_key);
    if !stored_hash.starts_with("hmac-sha256:") && constant_time_eq(stored_hash, &legacy_hash) {
        return ApiKeyHashStatus::Legacy;
    }

    ApiKeyHashStatus::Invalid
}

pub fn generate_user_api_key(config: &AppConfig) -> (String, String, String) {
    let prefix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(10)
        .collect::<String>();
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let key = format!("bzusr_{}_{}", prefix, secret);
    let hash = hash_api_key(&key, config);
    (key, prefix, hash)
}

pub fn parse_api_key_prefix(key: &str) -> Option<&str> {
    let mut parts = key.split('_');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("bzusr"), Some(prefix), Some(secret), None)
            if prefix.len() == 10
                && prefix.chars().all(|ch| ch.is_ascii_hexdigit())
                && !secret.is_empty() =>
        {
            Some(prefix)
        }
        _ => None,
    }
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

pub fn problem(status: StatusCode, code: &str, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(ProblemResponse::new(code, message, status.as_u16()))
}

pub fn with_auth_headers(mut response: HttpResponse, auth: &RequestAuth) -> HttpResponse {
    add_rate_limit_headers(&mut response, &auth.rate_limit);
    if let Some(quota) = &auth.daily_quota {
        add_daily_quota_headers(&mut response, quota);
    }
    response
}

pub fn with_rate_limit_headers(
    mut response: HttpResponse,
    rate_limit: &RateLimitOutcome,
) -> HttpResponse {
    add_rate_limit_headers(&mut response, rate_limit);
    response
}

pub fn with_daily_quota_headers(
    mut response: HttpResponse,
    quota: &DailyQuotaOutcome,
) -> HttpResponse {
    add_daily_quota_headers(&mut response, quota);
    response
}

pub async fn authorize_public(
    req: &HttpRequest,
    state: &AppState,
    required_scope: &str,
) -> Result<RequestAuth, HttpResponse> {
    let policy = load_access_policy(state).await?;

    if let Some(raw_key) = header_value(req, USER_API_KEY_HEADER) {
        if state
            .config
            .admin_api_key
            .as_deref()
            .is_some_and(|admin_key| constant_time_eq(admin_key, raw_key))
        {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "admin_key_not_allowed",
                "Admin key cannot be used as a public API key",
            ));
        }

        let prefix = parse_api_key_prefix(raw_key).ok_or_else(|| {
            problem(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "Invalid API key format",
            )
        })?;
        let record = load_api_key_record(state, prefix).await?;

        let id = record.id.as_ref().map(|id| id.to_hex()).unwrap_or_default();
        match verify_api_key_hash(&record.key_hash, raw_key, &state.config) {
            ApiKeyHashStatus::Current => {}
            ApiKeyHashStatus::Legacy => {
                let new_hash = hash_api_key(raw_key, &state.config);
                if let Err(error) = db::update_api_key_hash(&state.db, &id, new_hash).await {
                    eprintln!("API key hash migration failed for {}: {}", id, error);
                } else {
                    state
                        .security_store
                        .cache_remove_prefix(&api_key_record_cache_key(prefix))
                        .await;
                }
            }
            ApiKeyHashStatus::Invalid => {
                return Err(problem(
                    StatusCode::UNAUTHORIZED,
                    "invalid_api_key",
                    "Invalid API key",
                ));
            }
        }

        if record.status != "active" {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "api_key_disabled",
                "API key is not active",
            ));
        }
        if record
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "api_key_expired",
                "API key has expired",
            ));
        }
        if !record.scopes.iter().any(|scope| scope == required_scope) {
            return Err(problem(
                StatusCode::FORBIDDEN,
                "scope_denied",
                "API key does not have the required scope",
            ));
        }

        let rate_limit = check_rate_limit_or_503(
            state,
            &format!("api-key:{}", id),
            record.rate_limit_per_minute,
            Duration::from_secs(60),
        )
        .await?;
        if !rate_limit.allowed {
            return Err(with_rate_limit_headers(
                problem(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "Rate limit exceeded",
                ),
                &rate_limit,
            ));
        }

        let daily_quota = if let Some(limit) = record.daily_quota {
            let quota = check_daily_quota_or_503(state, &format!("api-key:{}", id), limit).await?;
            if !quota.allowed {
                return Err(with_daily_quota_headers(
                    problem(
                        StatusCode::TOO_MANY_REQUESTS,
                        "quota_exceeded",
                        "Daily quota exceeded",
                    ),
                    &quota,
                ));
            }
            Some(quota)
        } else {
            None
        };

        schedule_api_key_last_used_touch(state, &id).await;
        return Ok(RequestAuth {
            rate_limit,
            daily_quota,
        });
    }

    if !policy.anonymous_public_enabled {
        return Err(problem(
            StatusCode::UNAUTHORIZED,
            "api_key_required",
            "Anonymous public API access is disabled",
        ));
    }

    let ip = request_ip(req, state);
    let rate_limit = check_rate_limit_or_503(
        state,
        &format!("anonymous:{}", ip),
        policy.anonymous_rate_limit_per_minute,
        Duration::from_secs(60),
    )
    .await?;

    if !rate_limit.allowed {
        return Err(with_rate_limit_headers(
            problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Rate limit exceeded",
            ),
            &rate_limit,
        ));
    }

    Ok(RequestAuth {
        rate_limit,
        daily_quota: None,
    })
}

async fn load_api_key_record(state: &AppState, prefix: &str) -> Result<ApiKeyRecord, HttpResponse> {
    let cache_key = api_key_record_cache_key(prefix);
    if let Some(record) = cached_api_key_record(state, &cache_key).await {
        return Ok(record);
    }

    let lock_key = format!("api-key-record-fill:{}", prefix);
    let lock_token = match state
        .security_store
        .acquire_lock(&lock_key, Duration::from_secs(5))
        .await
    {
        Ok(token) => token,
        Err(error) => {
            eprintln!("API key record cache fill lock failed: {}", error);
            None
        }
    };

    if lock_token.is_none() {
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if let Some(record) = cached_api_key_record(state, &cache_key).await {
                return Ok(record);
            }
        }
    }

    let result = match db::find_api_key_by_prefix(&state.db, prefix).await {
        Ok(Some(record)) => Ok(record),
        Ok(None) => Err(problem(
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "Invalid API key",
        )),
        Err(error) => {
            eprintln!("API key lookup failed: {}", error);
            Err(problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_key_lookup_error",
                "API key could not be verified",
            ))
        }
    };

    if let Some(token) = lock_token {
        if let Err(error) = state.security_store.release_lock(&lock_key, &token).await {
            eprintln!("API key record cache fill lock release failed: {}", error);
        }
    }

    if let Ok(record) = &result
        && let Ok(value) = serde_json::to_value(record)
    {
        state
            .security_store
            .cache_set(cache_key, value, API_KEY_RECORD_CACHE_TTL)
            .await;
    }

    result
}

async fn cached_api_key_record(state: &AppState, cache_key: &str) -> Option<ApiKeyRecord> {
    match state.security_store.cache_get(cache_key).await {
        Some(value) => match serde_json::from_value::<ApiKeyRecord>(value) {
            Ok(record) => Some(record),
            Err(error) => {
                eprintln!("Cached API key record parse failed: {}", error);
                None
            }
        },
        None => None,
    }
}

async fn schedule_api_key_last_used_touch(state: &AppState, id: &str) {
    let now = Instant::now();
    let mut timestamps = state.api_key_last_used_timestamps.lock().await;
    if !should_touch_api_key_last_used(&mut timestamps, id, now) {
        return;
    }
    drop(timestamps);

    let db = state.db.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        if let Err(error) = db::touch_api_key_last_used(&db, &id).await {
            eprintln!("API key last_used_at update failed for {}: {}", id, error);
        }
    });
}

fn should_touch_api_key_last_used(
    timestamps: &mut HashMap<String, Instant>,
    id: &str,
    now: Instant,
) -> bool {
    let retention = Duration::from_secs(API_KEY_LAST_USED_TOUCH_TTL.as_secs().saturating_mul(10));
    timestamps.retain(|_, last_touch| now.duration_since(*last_touch) <= retention);

    if timestamps
        .get(id)
        .is_some_and(|last_touch| now.duration_since(*last_touch) < API_KEY_LAST_USED_TOUCH_TTL)
    {
        return false;
    }

    timestamps.insert(id.to_string(), now);
    true
}

pub fn api_key_record_cache_key(prefix: &str) -> String {
    format!("api-key-record:{}", prefix)
}

pub async fn check_admin_login_rate_limit(
    req: &HttpRequest,
    state: &AppState,
) -> Result<(), HttpResponse> {
    let ip = request_ip(req, state);
    let minute = check_rate_limit_or_503(
        state,
        &format!("admin-login-minute:{}", ip),
        ADMIN_LOGIN_MINUTE_LIMIT,
        Duration::from_secs(60),
    )
    .await?;
    if !minute.allowed {
        return Err(with_rate_limit_headers(
            problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many admin login attempts",
            ),
            &minute,
        ));
    }

    let hour = check_rate_limit_or_503(
        state,
        &format!("admin-login-hour:{}", ip),
        ADMIN_LOGIN_HOUR_LIMIT,
        Duration::from_secs(60 * 60),
    )
    .await?;
    if !hour.allowed {
        return Err(with_rate_limit_headers(
            problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many admin login attempts",
            ),
            &hour,
        ));
    }

    Ok(())
}

pub async fn authorize_admin(req: &HttpRequest, state: &AppState) -> Result<(), HttpResponse> {
    let configured_key = state.config.admin_api_key.as_deref().ok_or_else(|| {
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_auth_not_configured",
            "ADMIN_API_KEY is not configured",
        )
    })?;

    if let Some(raw_key) = header_value(req, ADMIN_API_KEY_HEADER) {
        let ip = request_ip(req, state);
        let attempt_limit = check_rate_limit_or_503(
            state,
            &format!("admin-api-key-attempt:{}", ip),
            ADMIN_API_RATE_LIMIT,
            Duration::from_secs(60),
        )
        .await?;
        if !attempt_limit.allowed {
            return Err(with_rate_limit_headers(
                problem(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "Rate limit exceeded",
                ),
                &attempt_limit,
            ));
        }

        if constant_time_eq(configured_key, raw_key) {
            let key_limit = check_rate_limit_or_503(
                state,
                "admin-api-key:primary",
                ADMIN_API_RATE_LIMIT,
                Duration::from_secs(60),
            )
            .await?;
            if !key_limit.allowed {
                return Err(with_rate_limit_headers(
                    problem(
                        StatusCode::TOO_MANY_REQUESTS,
                        "rate_limited",
                        "Rate limit exceeded",
                    ),
                    &key_limit,
                ));
            }
            return Ok(());
        }
        return Err(problem(
            StatusCode::FORBIDDEN,
            "invalid_admin_key",
            "Invalid admin API key",
        ));
    }

    let cookie_name = admin_session_cookie_name(&state.config);
    if let Some(cookie) = req.cookie(cookie_name) {
        let csrf_token = state
            .security_store
            .verify_admin_session(cookie.value())
            .await
            .map_err(|error| {
                eprintln!("Admin session verification failed: {}", error);
                security_store_unavailable()
            })?;
        if let Some(csrf_token) = csrf_token {
            let rate_limit = check_rate_limit_or_503(
                state,
                &format!("admin-session:{}", stable_hash(cookie.value())),
                ADMIN_API_RATE_LIMIT,
                Duration::from_secs(60),
            )
            .await?;
            if !rate_limit.allowed {
                return Err(with_rate_limit_headers(
                    problem(
                        StatusCode::TOO_MANY_REQUESTS,
                        "rate_limited",
                        "Rate limit exceeded",
                    ),
                    &rate_limit,
                ));
            }

            if state_changing(req.method()) {
                let provided = header_value(req, ADMIN_CSRF_HEADER).unwrap_or("");
                if !constant_time_eq(&csrf_token, provided) {
                    return Err(problem(
                        StatusCode::FORBIDDEN,
                        "csrf_required",
                        "A valid CSRF token is required",
                    ));
                }
            }

            return Ok(());
        }
    }

    Err(problem(
        StatusCode::UNAUTHORIZED,
        "admin_auth_required",
        "Admin authentication required",
    ))
}

pub fn verify_admin_key(raw_key: &str, state: &AppState) -> Result<(), HttpResponse> {
    let configured_key = state.config.admin_api_key.as_deref().ok_or_else(|| {
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_auth_not_configured",
            "ADMIN_API_KEY is not configured",
        )
    })?;

    if constant_time_eq(configured_key, raw_key) {
        Ok(())
    } else {
        Err(problem(
            StatusCode::FORBIDDEN,
            "invalid_admin_key",
            "Invalid admin API key",
        ))
    }
}

pub fn admin_session_cookie_name(config: &AppConfig) -> &'static str {
    if config.admin_cookie_secure {
        SECURE_ADMIN_SESSION_COOKIE
    } else {
        ADMIN_SESSION_COOKIE
    }
}

pub fn header_value<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
    req.headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

pub fn request_ip(req: &HttpRequest, state: &AppState) -> String {
    let peer_ip = req.peer_addr().map(|addr| addr.ip());
    if state.config.trust_proxy && peer_ip.is_some_and(|ip| trusted_proxy(ip, &state.config)) {
        if let Some(forwarded_for) = header_value(req, "X-Forwarded-For") {
            if let Some(first_ip) = forwarded_for.split(',').next() {
                let ip = first_ip.trim();
                if !ip.is_empty() && ip.parse::<IpAddr>().is_ok() {
                    return ip.to_string();
                }
            }
        }
    }

    peer_ip
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn load_access_policy(state: &AppState) -> Result<AccessPolicy, HttpResponse> {
    match state.security_store.get_access_policy().await {
        Ok(Some(policy)) => Ok(policy),
        Ok(None) => {
            let policy = db::get_access_policy(&state.db).await.map_err(|error| {
                eprintln!("Access policy load failed: {}", error);
                problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "access_policy_error",
                    "Access policy could not be loaded",
                )
            })?;
            state
                .security_store
                .set_access_policy(policy.clone(), ACCESS_POLICY_CACHE_TTL)
                .await
                .map_err(|error| {
                    eprintln!("Access policy cache set failed: {}", error);
                    security_store_unavailable()
                })?;
            Ok(policy)
        }
        Err(error) => {
            eprintln!("Access policy cache failed: {}", error);
            Err(security_store_unavailable())
        }
    }
}

async fn check_rate_limit_or_503(
    state: &AppState,
    key: &str,
    limit: u32,
    window: Duration,
) -> Result<RateLimitOutcome, HttpResponse> {
    state
        .security_store
        .check_rate_limit(key, limit, window)
        .await
        .map_err(|error| {
            eprintln!("Rate limit check failed: {}", error);
            security_store_unavailable()
        })
}

async fn check_daily_quota_or_503(
    state: &AppState,
    key: &str,
    limit: u32,
) -> Result<DailyQuotaOutcome, HttpResponse> {
    state
        .security_store
        .check_daily_quota(key, limit, Utc::now())
        .await
        .map_err(|error| {
            eprintln!("Daily quota check failed: {}", error);
            security_store_unavailable()
        })
}

fn security_store_unavailable() -> HttpResponse {
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "security_store_unavailable",
        "Security store is unavailable",
    )
}

fn add_rate_limit_headers(response: &mut HttpResponse, rate_limit: &RateLimitOutcome) {
    if let Ok(value) = HeaderValue::from_str(&rate_limit.limit.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-ratelimit-limit"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&rate_limit.remaining.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-ratelimit-remaining"), value);
    }
    if !rate_limit.allowed {
        if let Ok(value) = HeaderValue::from_str(&rate_limit.retry_after_seconds.to_string()) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("retry-after"), value);
        }
    }
}

fn add_daily_quota_headers(response: &mut HttpResponse, quota: &DailyQuotaOutcome) {
    if let Ok(value) = HeaderValue::from_str(&quota.limit.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-dailyquota-limit"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&quota.remaining.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-dailyquota-remaining"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&quota.reset_at.timestamp().to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-dailyquota-reset"), value);
    }
}

fn state_changing(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    )
}

fn trusted_proxy(ip: IpAddr, config: &AppConfig) -> bool {
    config
        .trusted_proxy_cidrs
        .iter()
        .any(|network| network.contains(&ip))
}

fn stable_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{
        ApiKeyHashStatus, constant_time_eq, generate_user_api_key, legacy_hash_api_key,
        parse_api_key_prefix, should_touch_api_key_last_used, verify_api_key_hash,
    };
    use crate::config::{AppConfig, AppEnvironment};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig {
            mongodb_uri: "mongodb://localhost:27017".to_string(),
            mongodb_db: "test".to_string(),
            bind_addr: "127.0.0.1:0".to_string(),
            app_env: AppEnvironment::Development,
            admin_api_key: Some("admin".to_string()),
            cors_allowed_origins: vec![],
            trust_proxy: false,
            trusted_proxy_cidrs: vec![],
            redis_url: None,
            require_redis: false,
            api_key_hash_pepper: "test-pepper".to_string(),
            admin_cookie_secure: false,
            cache_max_entries: 10,
            rate_limit_max_keys: 10,
            admin_json_limit_bytes: 16 * 1024,
            request_logging: false,
            response_compression: false,
        })
    }

    #[test]
    fn generated_user_key_can_be_hashed_and_parsed() {
        let config = test_config();
        let (key, prefix, hash) = generate_user_api_key(&config);

        assert_eq!(parse_api_key_prefix(&key), Some(prefix.as_str()));
        assert_eq!(
            verify_api_key_hash(&hash, &key, &config),
            ApiKeyHashStatus::Current
        );
    }

    #[test]
    fn legacy_key_hash_can_still_verify() {
        let config = test_config();
        let key = "bzusr_012345abcd_secret";
        let legacy = legacy_hash_api_key(key);

        assert_eq!(
            verify_api_key_hash(&legacy, key, &config),
            ApiKeyHashStatus::Legacy
        );
    }

    #[test]
    fn malformed_key_prefix_is_rejected() {
        assert_eq!(parse_api_key_prefix("bzusr_short_secret"), None);
        assert_eq!(parse_api_key_prefix("bzusr_012345abcd_secret_extra"), None);
    }

    #[test]
    fn constant_time_comparison_works() {
        assert!(constant_time_eq("same", "same"));
        assert!(!constant_time_eq("same", "nope"));
    }

    #[test]
    fn last_used_touch_is_locally_throttled() {
        let mut timestamps = HashMap::new();
        let now = Instant::now();

        assert!(should_touch_api_key_last_used(
            &mut timestamps,
            "key-id",
            now
        ));
        assert!(!should_touch_api_key_last_used(
            &mut timestamps,
            "key-id",
            now + Duration::from_secs(30)
        ));
        assert!(should_touch_api_key_last_used(
            &mut timestamps,
            "key-id",
            now + Duration::from_secs(60)
        ));
    }
}
