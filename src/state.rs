use crate::config::AppConfig;
use crate::models::AccessPolicy;
use mongodb::Database;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone, PartialEq)]
pub struct ProductSnapshot {
    pub buy_price: f64,
    pub sell_price: f64,
    pub buy_volume: i64,
    pub sell_volume: i64,
    pub buy_orders: i64,
    pub sell_orders: i64,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub config: Arc<AppConfig>,
    pub http_client: reqwest::Client,
    pub rate_limiter: Arc<MemoryRateLimiter>,
    pub cache: Arc<MemoryCache>,
    pub access_policy_cache: Arc<AccessPolicyCache>,
    pub admin_sessions: Arc<AdminSessionStore>,
    pub last_snapshot: Arc<RwLock<HashMap<String, ProductSnapshot>>>,
}

impl AppState {
    pub fn new(db: Database, config: Arc<AppConfig>) -> Self {
        if config.redis_url.is_some() {
            println!(
                "Redis URL configured; using in-process cache/rate-limit fallback in this build"
            );
        }

        Self {
            db,
            config,
            http_client: reqwest::Client::new(),
            rate_limiter: Arc::new(MemoryRateLimiter::default()),
            cache: Arc::new(MemoryCache::default()),
            access_policy_cache: Arc::new(AccessPolicyCache::new(Duration::from_secs(30))),
            admin_sessions: Arc::new(AdminSessionStore::default()),
            last_snapshot: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone)]
struct AccessPolicyCacheEntry {
    policy: AccessPolicy,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct AccessPolicyCache {
    ttl: Duration,
    entry: RwLock<Option<AccessPolicyCacheEntry>>,
}

impl AccessPolicyCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entry: RwLock::new(None),
        }
    }

    pub async fn get(&self) -> Option<AccessPolicy> {
        let entry = self.entry.read().await;
        entry
            .as_ref()
            .filter(|cached| cached.expires_at > Instant::now())
            .map(|cached| cached.policy.clone())
    }

    pub async fn set(&self, policy: AccessPolicy) {
        let mut entry = self.entry.write().await;
        *entry = Some(AccessPolicyCacheEntry {
            policy,
            expires_at: Instant::now() + self.ttl,
        });
    }

    #[allow(dead_code)]
    pub async fn invalidate(&self) {
        let mut entry = self.entry.write().await;
        *entry = None;
    }
}

#[derive(Debug, Clone)]
pub struct RateLimitOutcome {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub retry_after_seconds: u64,
}

#[derive(Debug)]
struct RateWindow {
    started_at: Instant,
    count: u32,
}

#[derive(Debug, Default)]
pub struct MemoryRateLimiter {
    windows: Mutex<HashMap<String, RateWindow>>,
}

impl MemoryRateLimiter {
    pub async fn check(&self, key: &str, limit: u32, window: Duration) -> RateLimitOutcome {
        let effective_limit = limit.max(1);
        let now = Instant::now();
        let mut windows = self.windows.lock().await;
        let window_state = windows.entry(key.to_string()).or_insert(RateWindow {
            started_at: now,
            count: 0,
        });

        if now.duration_since(window_state.started_at) >= window {
            window_state.started_at = now;
            window_state.count = 0;
        }

        let elapsed = now.duration_since(window_state.started_at);
        let retry_after_seconds = window
            .checked_sub(elapsed)
            .unwrap_or_else(|| Duration::from_secs(0))
            .as_secs()
            .max(1);

        if window_state.count >= effective_limit {
            return RateLimitOutcome {
                allowed: false,
                limit: effective_limit,
                remaining: 0,
                retry_after_seconds,
            };
        }

        window_state.count += 1;
        RateLimitOutcome {
            allowed: true,
            limit: effective_limit,
            remaining: effective_limit.saturating_sub(window_state.count),
            retry_after_seconds,
        }
    }
}

#[derive(Debug)]
struct CacheEntry {
    expires_at: Instant,
    value: Value,
}

#[derive(Debug, Default)]
pub struct MemoryCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl MemoryCache {
    pub async fn get(&self, key: &str) -> Option<Value> {
        let now = Instant::now();
        let mut entries = self.entries.lock().await;
        match entries.get(key) {
            Some(entry) if entry.expires_at > now => Some(entry.value.clone()),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    pub async fn set(&self, key: impl Into<String>, value: Value, ttl: Duration) {
        let mut entries = self.entries.lock().await;
        entries.insert(
            key.into(),
            CacheEntry {
                expires_at: Instant::now() + ttl,
                value,
            },
        );
    }

    pub async fn remove_prefix(&self, prefix: &str) {
        let mut entries = self.entries.lock().await;
        entries.retain(|key, _| !key.starts_with(prefix));
    }
}

#[derive(Debug)]
struct AdminSession {
    expires_at: Instant,
}

#[derive(Debug, Default)]
pub struct AdminSessionStore {
    sessions: Mutex<HashMap<String, AdminSession>>,
}

impl AdminSessionStore {
    pub async fn create(&self, ttl: Duration) -> String {
        let token = format!("adm_{}", uuid::Uuid::new_v4().simple());
        let mut sessions = self.sessions.lock().await;
        sessions.insert(
            token.clone(),
            AdminSession {
                expires_at: Instant::now() + ttl,
            },
        );
        token
    }

    pub async fn verify(&self, token: &str) -> bool {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().await;
        match sessions.get(token) {
            Some(session) if session.expires_at > now => true,
            Some(_) => {
                sessions.remove(token);
                false
            }
            None => false,
        }
    }

    pub async fn revoke(&self, token: &str) {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(token);
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessPolicyCache, MemoryRateLimiter};
    use crate::models::AccessPolicy;
    use std::time::Duration;

    #[tokio::test]
    async fn memory_rate_limiter_blocks_after_limit() {
        let limiter = MemoryRateLimiter::default();

        assert!(
            limiter
                .check("client", 2, Duration::from_secs(60))
                .await
                .allowed
        );
        assert!(
            limiter
                .check("client", 2, Duration::from_secs(60))
                .await
                .allowed
        );
        assert!(
            !limiter
                .check("client", 2, Duration::from_secs(60))
                .await
                .allowed
        );
    }

    #[tokio::test]
    async fn access_policy_cache_expires_entries() {
        let cache = AccessPolicyCache::new(Duration::from_millis(10));
        cache.set(AccessPolicy::default()).await;

        assert!(cache.get().await.is_some());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(cache.get().await.is_none());
    }
}
