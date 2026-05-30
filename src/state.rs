use crate::config::AppConfig;
use crate::shared_store::SharedSecurityStore;
use mongodb::Database;
use std::collections::HashMap;
use std::sync::Arc;
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
            hypixel_last_modified: Arc::new(RwLock::new(None)),
            hypixel_last_updated: Arc::new(RwLock::new(None)),
        }
    }
}
