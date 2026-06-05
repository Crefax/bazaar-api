use crate::config::AppConfig;
use crate::models::BazaarLatest;
use crate::shared_store::SharedSecurityStore;
use chrono::{DateTime, Utc};
use mongodb::Database;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

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
    pub security_store: SharedSecurityStore,
    pub last_snapshot: Arc<RwLock<HashMap<String, ProductSnapshot>>>,
    pub latest_cache: Arc<RwLock<HashMap<String, BazaarLatest>>>,
    verified_api_key_cache: Arc<RwLock<HashMap<String, CachedVerifiedApiKey>>>,
    raw_cache_refresh_deadlines: Arc<StdMutex<HashMap<String, Instant>>>,
    pub api_key_last_used_timestamps: Arc<RwLock<HashMap<String, Instant>>>,
    pub hypixel_last_modified: Arc<RwLock<Option<String>>>,
    pub hypixel_last_updated: Arc<RwLock<Option<i64>>>,
}

impl AppState {
    pub async fn new(db: Database, config: Arc<AppConfig>) -> Self {
        let security_store = SharedSecurityStore::new(&config).await;
        if let Some(error) = security_store.redis_init_error() {
            if config.redis_required() {
                eprintln!("Redis is required but unavailable: {}", error);
            }
        }

        let http_client = reqwest::Client::builder()
            .gzip(true)
            .build()
            .expect("failed to build HTTP client");

        Self {
            db,
            config,
            http_client,
            security_store,
            last_snapshot: Arc::new(RwLock::new(HashMap::new())),
            latest_cache: Arc::new(RwLock::new(HashMap::new())),
            verified_api_key_cache: Arc::new(RwLock::new(HashMap::new())),
            raw_cache_refresh_deadlines: Arc::new(StdMutex::new(HashMap::new())),
            api_key_last_used_timestamps: Arc::new(RwLock::new(HashMap::new())),
            hypixel_last_modified: Arc::new(RwLock::new(None)),
            hypixel_last_updated: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn cached_verified_api_key(&self, raw_key: &str) -> Option<VerifiedApiKey> {
        let now = Instant::now();
        {
            let entries = self.verified_api_key_cache.read().await;
            if let Some(entry) = entries.get(raw_key)
                && entry.expires_at > now
            {
                return Some(entry.value.clone());
            }
        }

        let mut entries = self.verified_api_key_cache.write().await;
        if entries
            .get(raw_key)
            .is_some_and(|entry| entry.expires_at <= now)
        {
            entries.remove(raw_key);
        }
        None
    }

    pub async fn cache_verified_api_key(
        &self,
        raw_key: String,
        value: VerifiedApiKey,
        ttl: Duration,
    ) {
        let mut entries = self.verified_api_key_cache.write().await;
        entries.retain(|_, entry| entry.expires_at > Instant::now());
        entries.insert(
            raw_key,
            CachedVerifiedApiKey {
                value,
                expires_at: Instant::now() + ttl,
            },
        );
    }

    pub async fn remove_verified_api_key_cache_prefix(&self, prefix: &str) {
        let mut entries = self.verified_api_key_cache.write().await;
        entries.retain(|_, entry| entry.value.key_prefix != prefix);
    }

    pub fn try_start_raw_cache_refresh(&self, cache_key: &str, cooldown: Duration) -> bool {
        let now = Instant::now();
        let mut entries = self
            .raw_cache_refresh_deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.retain(|_, deadline| *deadline > now);
        if entries
            .get(cache_key)
            .is_some_and(|deadline| *deadline > now)
        {
            return false;
        }
        entries.insert(cache_key.to_string(), now + cooldown);
        true
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedApiKey {
    pub id: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub rate_limit_per_minute: u32,
    pub daily_quota: Option<u32>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct CachedVerifiedApiKey {
    value: VerifiedApiKey,
    expires_at: Instant,
}
