use crate::config::AppConfig;
use crate::models::AccessPolicy;
use chrono::{DateTime, Days, Utc};
use redis::aio::ConnectionManager;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

const REDIS_KEY_PREFIX: &str = "bazaar-api:";

#[derive(Debug, Clone)]
pub struct RateLimitOutcome {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub retry_after_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct DailyQuotaOutcome {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub reset_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AdminSessionTokens {
    pub session_token: String,
    pub csrf_token: String,
}

#[derive(Debug, Clone)]
pub struct StoreReadiness {
    pub backend: &'static str,
    pub redis_required: bool,
    pub redis_available: bool,
    pub ready: bool,
}

#[derive(Debug, Clone)]
pub struct SecurityStoreError {
    message: String,
}

impl SecurityStoreError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SecurityStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SecurityStoreError {}

type StoreResult<T> = Result<T, SecurityStoreError>;

#[derive(Clone)]
pub struct SharedSecurityStore {
    redis: Option<RedisSecurityStore>,
    memory: Arc<MemorySecurityStore>,
    redis_required: bool,
    redis_init_error: Option<String>,
}

impl SharedSecurityStore {
    pub async fn new(config: &AppConfig) -> Self {
        let memory = Arc::new(MemorySecurityStore::new(
            config.cache_max_entries,
            config.rate_limit_max_keys,
        ));
        let redis_required = config.redis_required();

        if let Some(redis_url) = &config.redis_url {
            match RedisSecurityStore::connect(redis_url).await {
                Ok(redis) => {
                    return Self {
                        redis: Some(redis),
                        memory,
                        redis_required,
                        redis_init_error: None,
                    };
                }
                Err(error) => {
                    eprintln!("Redis connection failed: {}", error);
                    return Self {
                        redis: None,
                        memory,
                        redis_required,
                        redis_init_error: Some(error.to_string()),
                    };
                }
            }
        }

        Self {
            redis: None,
            memory,
            redis_required,
            redis_init_error: None,
        }
    }

    pub async fn readiness(&self) -> StoreReadiness {
        let redis_available = match &self.redis {
            Some(redis) => redis.ping().await.is_ok(),
            None => false,
        };
        StoreReadiness {
            backend: if self.redis.is_some() {
                "redis"
            } else {
                "memory"
            },
            redis_required: self.redis_required,
            redis_available,
            ready: !self.redis_required || redis_available,
        }
    }

    pub fn redis_init_error(&self) -> Option<&str> {
        self.redis_init_error.as_deref()
    }

    pub async fn check_rate_limit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> StoreResult<RateLimitOutcome> {
        if let Some(redis) = &self.redis {
            match redis.check_rate_limit(key, limit, window).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => {
                    eprintln!("Redis rate limit failed, falling back to memory: {}", error)
                }
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        self.memory.check_rate_limit(key, limit, window).await
    }

    pub async fn check_daily_quota(
        &self,
        key: &str,
        limit: u32,
        now: DateTime<Utc>,
    ) -> StoreResult<DailyQuotaOutcome> {
        if let Some(redis) = &self.redis {
            match redis.check_daily_quota(key, limit, now).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => eprintln!(
                    "Redis daily quota failed, falling back to memory: {}",
                    error
                ),
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        self.memory.check_daily_quota(key, limit, now).await
    }

    pub async fn cache_get(&self, key: &str) -> Option<Value> {
        if let Some(redis) = &self.redis {
            match redis.cache_get(key).await {
                Ok(value) => return value,
                Err(error) if self.redis_required => {
                    eprintln!("Redis cache get failed while required: {}", error);
                    return None;
                }
                Err(error) => {
                    eprintln!("Redis cache get failed, falling back to memory: {}", error)
                }
            }
        }

        self.memory.cache_get(key).await
    }

    pub async fn cache_set(&self, key: impl Into<String>, value: Value, ttl: Duration) {
        let key = key.into();
        if let Some(redis) = &self.redis {
            match redis.cache_set(&key, &value, ttl).await {
                Ok(()) => return,
                Err(error) if self.redis_required => {
                    eprintln!("Redis cache set failed while required: {}", error);
                    return;
                }
                Err(error) => {
                    eprintln!("Redis cache set failed, falling back to memory: {}", error)
                }
            }
        }

        self.memory.cache_set(key, value, ttl).await;
    }

    pub async fn cache_remove_prefix(&self, prefix: &str) {
        if let Some(redis) = &self.redis {
            match redis.cache_remove_prefix(prefix).await {
                Ok(()) => return,
                Err(error) if self.redis_required => {
                    eprintln!("Redis cache remove failed while required: {}", error);
                    return;
                }
                Err(error) => eprintln!(
                    "Redis cache remove failed, falling back to memory: {}",
                    error
                ),
            }
        }

        self.memory.cache_remove_prefix(prefix).await;
    }

    pub async fn get_access_policy(&self) -> StoreResult<Option<AccessPolicy>> {
        if let Some(redis) = &self.redis {
            match redis.get_access_policy().await {
                Ok(policy) => return Ok(policy),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => eprintln!(
                    "Redis policy cache failed, falling back to memory: {}",
                    error
                ),
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        Ok(self.memory.get_access_policy().await)
    }

    pub async fn set_access_policy(&self, policy: AccessPolicy, ttl: Duration) -> StoreResult<()> {
        if let Some(redis) = &self.redis {
            match redis.set_access_policy(&policy, ttl).await {
                Ok(()) => return Ok(()),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => {
                    eprintln!("Redis policy set failed, falling back to memory: {}", error)
                }
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        self.memory.set_access_policy(policy, ttl).await;
        Ok(())
    }

    pub async fn create_admin_session(&self, ttl: Duration) -> StoreResult<AdminSessionTokens> {
        let tokens = AdminSessionTokens {
            session_token: format!("adm_{}", uuid::Uuid::new_v4().simple()),
            csrf_token: format!("csrf_{}", uuid::Uuid::new_v4().simple()),
        };

        if let Some(redis) = &self.redis {
            match redis.create_admin_session(&tokens, ttl).await {
                Ok(()) => return Ok(tokens),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => eprintln!(
                    "Redis session create failed, falling back to memory: {}",
                    error
                ),
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        self.memory.create_admin_session(&tokens, ttl).await;
        Ok(tokens)
    }

    pub async fn verify_admin_session(&self, token: &str) -> StoreResult<Option<String>> {
        if let Some(redis) = &self.redis {
            match redis.verify_admin_session(token).await {
                Ok(csrf) => return Ok(csrf),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => eprintln!(
                    "Redis session verify failed, falling back to memory: {}",
                    error
                ),
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        Ok(self.memory.verify_admin_session(token).await)
    }

    pub async fn revoke_admin_session(&self, token: &str) -> StoreResult<()> {
        if let Some(redis) = &self.redis {
            match redis.revoke_admin_session(token).await {
                Ok(()) => return Ok(()),
                Err(error) if self.redis_required => return Err(error),
                Err(error) => eprintln!(
                    "Redis session revoke failed, falling back to memory: {}",
                    error
                ),
            }
        } else if self.redis_required {
            return Err(SecurityStoreError::new(
                "Redis is required but not available",
            ));
        }

        self.memory.revoke_admin_session(token).await;
        Ok(())
    }
}

#[derive(Clone)]
struct RedisSecurityStore {
    manager: ConnectionManager,
}

impl RedisSecurityStore {
    async fn connect(redis_url: &str) -> StoreResult<Self> {
        let client = redis::Client::open(redis_url)
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        let manager = client
            .get_connection_manager()
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        Ok(Self { manager })
    }

    async fn ping(&self) -> StoreResult<()> {
        let mut conn = self.manager.clone();
        redis::cmd("PING")
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))
    }

    async fn check_rate_limit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> StoreResult<RateLimitOutcome> {
        let key = redis_key(&format!("rate:{}", key));
        let mut conn = self.manager.clone();
        let count = redis::cmd("INCR")
            .arg(&key)
            .query_async::<u32>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        if count == 1 {
            let _: () = redis::cmd("EXPIRE")
                .arg(&key)
                .arg(window.as_secs().max(1))
                .query_async(&mut conn)
                .await
                .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        }
        let ttl = redis::cmd("TTL")
            .arg(&key)
            .query_async::<i64>(&mut conn)
            .await
            .unwrap_or(window.as_secs() as i64);
        Ok(rate_outcome(count, limit, ttl.max(1) as u64))
    }

    async fn check_daily_quota(
        &self,
        key: &str,
        limit: u32,
        now: DateTime<Utc>,
    ) -> StoreResult<DailyQuotaOutcome> {
        let reset_at = next_utc_midnight(now);
        let key = redis_key(&format!("quota:{}:{}", key, now.format("%Y%m%d")));
        let mut conn = self.manager.clone();
        let count = redis::cmd("INCR")
            .arg(&key)
            .query_async::<u32>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        if count == 1 {
            let _: () = redis::cmd("EXPIRE")
                .arg(&key)
                .arg(seconds_until(reset_at, now))
                .query_async(&mut conn)
                .await
                .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        }
        Ok(quota_outcome(count, limit, reset_at))
    }

    async fn cache_get(&self, key: &str) -> StoreResult<Option<Value>> {
        let mut conn = self.manager.clone();
        let raw = redis::cmd("GET")
            .arg(redis_key(&format!("cache:{}", key)))
            .query_async::<Option<String>>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        Ok(raw.and_then(|value| serde_json::from_str(&value).ok()))
    }

    async fn cache_set(&self, key: &str, value: &Value, ttl: Duration) -> StoreResult<()> {
        let raw = serde_json::to_string(value)
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        let mut conn = self.manager.clone();
        redis::cmd("SETEX")
            .arg(redis_key(&format!("cache:{}", key)))
            .arg(ttl.as_secs().max(1))
            .arg(raw)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))
    }

    async fn cache_remove_prefix(&self, prefix: &str) -> StoreResult<()> {
        let pattern = redis_key(&format!("cache:{}*", prefix));
        let mut conn = self.manager.clone();
        let keys = redis::cmd("KEYS")
            .arg(pattern)
            .query_async::<Vec<String>>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        if !keys.is_empty() {
            redis::cmd("DEL")
                .arg(keys)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        }
        Ok(())
    }

    async fn get_access_policy(&self) -> StoreResult<Option<AccessPolicy>> {
        let mut conn = self.manager.clone();
        let raw = redis::cmd("GET")
            .arg(redis_key("policy:public_api"))
            .query_async::<Option<String>>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        Ok(raw.and_then(|value| serde_json::from_str(&value).ok()))
    }

    async fn set_access_policy(&self, policy: &AccessPolicy, ttl: Duration) -> StoreResult<()> {
        let raw = serde_json::to_string(policy)
            .map_err(|error| SecurityStoreError::new(error.to_string()))?;
        let mut conn = self.manager.clone();
        redis::cmd("SETEX")
            .arg(redis_key("policy:public_api"))
            .arg(ttl.as_secs().max(1))
            .arg(raw)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))
    }

    async fn create_admin_session(
        &self,
        tokens: &AdminSessionTokens,
        ttl: Duration,
    ) -> StoreResult<()> {
        let mut conn = self.manager.clone();
        redis::cmd("SETEX")
            .arg(redis_key(&format!(
                "admin-session:{}",
                tokens.session_token
            )))
            .arg(ttl.as_secs().max(1))
            .arg(&tokens.csrf_token)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))
    }

    async fn verify_admin_session(&self, token: &str) -> StoreResult<Option<String>> {
        let mut conn = self.manager.clone();
        redis::cmd("GET")
            .arg(redis_key(&format!("admin-session:{}", token)))
            .query_async::<Option<String>>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))
    }

    async fn revoke_admin_session(&self, token: &str) -> StoreResult<()> {
        let mut conn = self.manager.clone();
        redis::cmd("DEL")
            .arg(redis_key(&format!("admin-session:{}", token)))
            .query_async::<()>(&mut conn)
            .await
            .map_err(|error| SecurityStoreError::new(error.to_string()))
    }
}

#[derive(Debug)]
struct MemorySecurityStore {
    max_cache_entries: usize,
    max_rate_keys: usize,
    rate_windows: Mutex<HashMap<String, MemoryRateWindow>>,
    daily_quotas: Mutex<HashMap<String, MemoryDailyQuota>>,
    cache_entries: Mutex<HashMap<String, MemoryCacheEntry>>,
    policy: RwLock<Option<MemoryAccessPolicyEntry>>,
    admin_sessions: Mutex<HashMap<String, MemoryAdminSession>>,
}

impl MemorySecurityStore {
    fn new(max_cache_entries: usize, max_rate_keys: usize) -> Self {
        Self {
            max_cache_entries,
            max_rate_keys,
            rate_windows: Mutex::new(HashMap::new()),
            daily_quotas: Mutex::new(HashMap::new()),
            cache_entries: Mutex::new(HashMap::new()),
            policy: RwLock::new(None),
            admin_sessions: Mutex::new(HashMap::new()),
        }
    }

    async fn check_rate_limit(
        &self,
        key: &str,
        limit: u32,
        window: Duration,
    ) -> StoreResult<RateLimitOutcome> {
        let now = Instant::now();
        let mut windows = self.rate_windows.lock().await;
        windows.retain(|_, value| now.duration_since(value.started_at) < value.window);

        if !windows.contains_key(key) && windows.len() >= self.max_rate_keys {
            return Err(SecurityStoreError::new("rate limit key capacity reached"));
        }

        let window_state = windows
            .entry(key.to_string())
            .or_insert_with(|| MemoryRateWindow {
                started_at: now,
                window,
                count: 0,
            });

        if now.duration_since(window_state.started_at) >= window_state.window {
            window_state.started_at = now;
            window_state.window = window;
            window_state.count = 0;
        }

        let retry_after_seconds = window_state
            .window
            .checked_sub(now.duration_since(window_state.started_at))
            .unwrap_or_else(|| Duration::from_secs(0))
            .as_secs()
            .max(1);
        if window_state.count >= limit.max(1) {
            return Ok(RateLimitOutcome {
                allowed: false,
                limit: limit.max(1),
                remaining: 0,
                retry_after_seconds,
            });
        }

        window_state.count += 1;
        Ok(RateLimitOutcome {
            allowed: true,
            limit: limit.max(1),
            remaining: limit.max(1).saturating_sub(window_state.count),
            retry_after_seconds,
        })
    }

    async fn check_daily_quota(
        &self,
        key: &str,
        limit: u32,
        now: DateTime<Utc>,
    ) -> StoreResult<DailyQuotaOutcome> {
        let reset_at = next_utc_midnight(now);
        let key = format!("{}:{}", key, now.format("%Y%m%d"));
        let mut quotas = self.daily_quotas.lock().await;
        quotas.retain(|_, value| value.reset_at > now);
        if !quotas.contains_key(&key) && quotas.len() >= self.max_rate_keys {
            return Err(SecurityStoreError::new("quota key capacity reached"));
        }

        let quota = quotas
            .entry(key)
            .or_insert(MemoryDailyQuota { count: 0, reset_at });
        if quota.count >= limit.max(1) {
            return Ok(DailyQuotaOutcome {
                allowed: false,
                limit: limit.max(1),
                remaining: 0,
                reset_at,
            });
        }

        quota.count += 1;
        Ok(quota_outcome(quota.count, limit, reset_at))
    }

    async fn cache_get(&self, key: &str) -> Option<Value> {
        let now = Instant::now();
        let mut entries = self.cache_entries.lock().await;
        match entries.get(key) {
            Some(entry) if entry.expires_at > now => Some(entry.value.clone()),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    async fn cache_set(&self, key: String, value: Value, ttl: Duration) {
        let now = Instant::now();
        let mut entries = self.cache_entries.lock().await;
        entries.retain(|_, entry| entry.expires_at > now);
        if !entries.contains_key(&key) && entries.len() >= self.max_cache_entries {
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(key, _)| key.clone())
            {
                entries.remove(&oldest_key);
            }
        }
        entries.insert(
            key,
            MemoryCacheEntry {
                inserted_at: now,
                expires_at: now + ttl,
                value,
            },
        );
    }

    async fn cache_remove_prefix(&self, prefix: &str) {
        let mut entries = self.cache_entries.lock().await;
        entries.retain(|key, _| !key.starts_with(prefix));
    }

    async fn get_access_policy(&self) -> Option<AccessPolicy> {
        let entry = self.policy.read().await;
        entry
            .as_ref()
            .filter(|cached| cached.expires_at > Instant::now())
            .map(|cached| cached.policy.clone())
    }

    async fn set_access_policy(&self, policy: AccessPolicy, ttl: Duration) {
        let mut entry = self.policy.write().await;
        *entry = Some(MemoryAccessPolicyEntry {
            policy,
            expires_at: Instant::now() + ttl,
        });
    }

    async fn create_admin_session(&self, tokens: &AdminSessionTokens, ttl: Duration) {
        let mut sessions = self.admin_sessions.lock().await;
        sessions.insert(
            tokens.session_token.clone(),
            MemoryAdminSession {
                csrf_token: tokens.csrf_token.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
    }

    async fn verify_admin_session(&self, token: &str) -> Option<String> {
        let now = Instant::now();
        let mut sessions = self.admin_sessions.lock().await;
        match sessions.get(token) {
            Some(session) if session.expires_at > now => Some(session.csrf_token.clone()),
            Some(_) => {
                sessions.remove(token);
                None
            }
            None => None,
        }
    }

    async fn revoke_admin_session(&self, token: &str) {
        let mut sessions = self.admin_sessions.lock().await;
        sessions.remove(token);
    }
}

#[derive(Debug)]
struct MemoryRateWindow {
    started_at: Instant,
    window: Duration,
    count: u32,
}

#[derive(Debug)]
struct MemoryDailyQuota {
    count: u32,
    reset_at: DateTime<Utc>,
}

#[derive(Debug)]
struct MemoryCacheEntry {
    inserted_at: Instant,
    expires_at: Instant,
    value: Value,
}

#[derive(Debug, Clone)]
struct MemoryAccessPolicyEntry {
    policy: AccessPolicy,
    expires_at: Instant,
}

#[derive(Debug)]
struct MemoryAdminSession {
    csrf_token: String,
    expires_at: Instant,
}

fn redis_key(suffix: &str) -> String {
    format!("{}{}", REDIS_KEY_PREFIX, suffix)
}

fn rate_outcome(count: u32, limit: u32, retry_after_seconds: u64) -> RateLimitOutcome {
    let limit = limit.max(1);
    RateLimitOutcome {
        allowed: count <= limit,
        limit,
        remaining: limit.saturating_sub(count),
        retry_after_seconds,
    }
}

fn quota_outcome(count: u32, limit: u32, reset_at: DateTime<Utc>) -> DailyQuotaOutcome {
    let limit = limit.max(1);
    DailyQuotaOutcome {
        allowed: count <= limit,
        limit,
        remaining: limit.saturating_sub(count),
        reset_at,
    }
}

fn next_utc_midnight(now: DateTime<Utc>) -> DateTime<Utc> {
    let tomorrow = now
        .date_naive()
        .checked_add_days(Days::new(1))
        .unwrap_or_else(|| now.date_naive());
    let reset = tomorrow
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| naive.and_local_timezone(Utc).single());
    reset.unwrap_or_else(|| now + chrono::Duration::days(1))
}

fn seconds_until(target: DateTime<Utc>, now: DateTime<Utc>) -> u64 {
    (target - now).num_seconds().max(1) as u64
}

#[cfg(test)]
mod tests {
    use super::SharedSecurityStore;
    use crate::config::{AppConfig, AppEnvironment};
    use std::sync::Arc;
    use std::time::Duration;

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
        })
    }

    #[tokio::test]
    async fn memory_rate_limiter_blocks_after_limit() {
        let store = SharedSecurityStore::new(&test_config()).await;

        assert!(
            store
                .check_rate_limit("client", 2, Duration::from_secs(60))
                .await
                .unwrap()
                .allowed
        );
        assert!(
            store
                .check_rate_limit("client", 2, Duration::from_secs(60))
                .await
                .unwrap()
                .allowed
        );
        assert!(
            !store
                .check_rate_limit("client", 2, Duration::from_secs(60))
                .await
                .unwrap()
                .allowed
        );
    }

    #[tokio::test]
    async fn memory_daily_quota_blocks_after_limit() {
        let store = SharedSecurityStore::new(&test_config()).await;
        let now = chrono::Utc::now();

        assert!(
            store
                .check_daily_quota("key", 1, now)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            !store
                .check_daily_quota("key", 1, now)
                .await
                .unwrap()
                .allowed
        );
    }

    #[tokio::test]
    async fn admin_session_verifies_csrf() {
        let store = SharedSecurityStore::new(&test_config()).await;
        let tokens = store
            .create_admin_session(Duration::from_secs(60))
            .await
            .unwrap();

        assert_eq!(
            store
                .verify_admin_session(&tokens.session_token)
                .await
                .unwrap(),
            Some(tokens.csrf_token)
        );
    }
}
