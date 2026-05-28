use crate::db;
use crate::models::BazaarData;
use crate::state::{AppState, ProductSnapshot};
use tokio::time::{Duration, interval};

pub async fn start_tracker(state: AppState) {
    let mut interval = interval(Duration::from_secs(15));

    loop {
        interval.tick().await;
        if let Err(e) = update_bazaar_data(&state).await {
            eprintln!("Error updating bazaar data: {}", e);
        }
    }
}

pub async fn start_retention_scheduler(state: AppState) {
    let mut interval = interval(Duration::from_secs(60 * 60));

    loop {
        interval.tick().await;
        if let Err(e) = db::cleanup_retention(&state.db).await {
            eprintln!("Error cleaning old chart data: {}", e);
        }
    }
}

async fn update_bazaar_data(state: &AppState) -> Result<(), Box<dyn std::error::Error>> {
    let response = state
        .http_client
        .get("https://api.hypixel.net/v2/skyblock/bazaar")
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;

    let Some(products) = response["products"].as_object() else {
        return Ok(());
    };

    let timestamp = chrono::Utc::now();
    let mut changed_count = 0usize;

    for (product_id, product_data) in products {
        let Some(quick_status) = product_data["quick_status"].as_object() else {
            continue;
        };

        let snapshot = ProductSnapshot {
            buy_price: quick_status["buyPrice"].as_f64().unwrap_or(0.0),
            sell_price: quick_status["sellPrice"].as_f64().unwrap_or(0.0),
            buy_volume: quick_status["buyVolume"].as_i64().unwrap_or(0),
            sell_volume: quick_status["sellVolume"].as_i64().unwrap_or(0),
            buy_orders: quick_status["buyOrders"].as_i64().unwrap_or(0),
            sell_orders: quick_status["sellOrders"].as_i64().unwrap_or(0),
        };

        if !mark_if_changed(state, product_id, snapshot.clone()).await {
            continue;
        }

        let bazaar_data = BazaarData {
            product_id: product_id.clone(),
            buy_price: snapshot.buy_price,
            sell_price: snapshot.sell_price,
            buy_volume: snapshot.buy_volume,
            sell_volume: snapshot.sell_volume,
            buy_orders: snapshot.buy_orders,
            sell_orders: snapshot.sell_orders,
            timestamp,
        };

        db::insert_bazaar_data(&state.db, bazaar_data.clone()).await?;
        db::upsert_latest_bazaar_data(&state.db, &bazaar_data).await?;
        db::upsert_chart_candles(&state.db, &bazaar_data).await?;
        changed_count += 1;
    }

    if changed_count > 0 {
        state.security_store.cache_remove_prefix("v2:latest").await;
        state.security_store.cache_remove_prefix("v2:candles").await;
        state.security_store.cache_remove_prefix("v2:series").await;
        state
            .security_store
            .cache_remove_prefix("v2:products")
            .await;
        println!("Tracker stored {} changed products", changed_count);
    }

    Ok(())
}

async fn mark_if_changed(state: &AppState, product_id: &str, snapshot: ProductSnapshot) -> bool {
    let mut last_snapshot = state.last_snapshot.write().await;
    match last_snapshot.get(product_id) {
        Some(previous) if previous == &snapshot => false,
        _ => {
            last_snapshot.insert(product_id.to_string(), snapshot);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::AppConfig;
    use crate::state::{AppState, ProductSnapshot};
    use mongodb::Client;
    use std::sync::Arc;

    #[tokio::test]
    async fn duplicate_snapshot_is_skipped() {
        let config = Arc::new(AppConfig {
            mongodb_uri: "mongodb://localhost:27017".to_string(),
            mongodb_db: "test".to_string(),
            bind_addr: "127.0.0.1:0".to_string(),
            app_env: crate::config::AppEnvironment::Development,
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
        });
        let client = Client::with_uri_str(&config.mongodb_uri).await.unwrap();
        let state = AppState::new(client.database(&config.mongodb_db), config).await;
        let snapshot = ProductSnapshot {
            buy_price: 1.0,
            sell_price: 2.0,
            buy_volume: 3,
            sell_volume: 4,
            buy_orders: 5,
            sell_orders: 6,
        };

        assert!(super::mark_if_changed(&state, "WHEAT", snapshot.clone()).await);
        assert!(!super::mark_if_changed(&state, "WHEAT", snapshot).await);
    }
}
