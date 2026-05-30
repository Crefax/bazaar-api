use crate::db;
use crate::models::BazaarData;
use crate::state::{AppState, ProductSnapshot};
use reqwest::StatusCode;
use reqwest::header::{ACCEPT_ENCODING, IF_MODIFIED_SINCE, LAST_MODIFIED};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::time::{Duration, interval};

const BAZAAR_URL: &str = "https://api.hypixel.net/v2/skyblock/bazaar";
const TRACKER_LOCK: &str = "bazaar:tracker";
const ROLLUP_LOCK: &str = "bazaar:rollup";

pub async fn start_tracker(state: AppState) {
    let mut interval = interval(Duration::from_secs(15));

    loop {
        interval.tick().await;
        match state
            .security_store
            .acquire_lock(TRACKER_LOCK, Duration::from_secs(45))
            .await
        {
            Ok(Some(token)) => {
                if let Err(e) = update_bazaar_data(&state).await {
                    eprintln!("Error updating bazaar data: {}", e);
                }
                if let Err(e) = state
                    .security_store
                    .release_lock(TRACKER_LOCK, &token)
                    .await
                {
                    eprintln!("Error releasing tracker lock: {}", e);
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("Error acquiring tracker lock: {}", e),
        }
    }
}

pub async fn start_rollup_scheduler(state: AppState) {
    let mut interval = interval(Duration::from_secs(60));

    loop {
        interval.tick().await;
        match state
            .security_store
            .acquire_lock(ROLLUP_LOCK, Duration::from_secs(55))
            .await
        {
            Ok(Some(token)) => {
                match db::rollup_closed_candles(&state.db).await {
                    Ok(count) if count > 0 => {
                        println!("Rollup wrote {} compressed candles", count);
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("Error rolling up chart data: {}", e),
                }
                if let Err(e) = state.security_store.release_lock(ROLLUP_LOCK, &token).await {
                    eprintln!("Error releasing rollup lock: {}", e);
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("Error acquiring rollup lock: {}", e),
        }
    }
}

async fn update_bazaar_data(state: &AppState) -> Result<(), Box<dyn std::error::Error>> {
    let mut request = state
        .http_client
        .get(BAZAAR_URL)
        .header(ACCEPT_ENCODING, "gzip");

    if let Some(last_modified) = state.hypixel_last_modified.read().await.clone() {
        request = request.header(IF_MODIFIED_SINCE, last_modified);
    }

    let response = request.send().await?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(());
    }
    let response = response.error_for_status()?;
    let last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    let parsed = response.json::<HypixelBazaarResponse>().await?;
    if !parsed.success {
        return Ok(());
    }
    if let Some(last_updated) = parsed.last_updated {
        let mut stored_last_updated = state.hypixel_last_updated.write().await;
        if stored_last_updated.is_some_and(|stored| stored == last_updated) {
            if let Some(last_modified) = last_modified {
                *state.hypixel_last_modified.write().await = Some(last_modified);
            }
            return Ok(());
        }
        *stored_last_updated = Some(last_updated);
    }
    if let Some(last_modified) = last_modified {
        *state.hypixel_last_modified.write().await = Some(last_modified);
    }

    let timestamp = chrono::Utc::now();
    let mut changed = Vec::new();
    {
        let mut last_snapshot = state.last_snapshot.write().await;
        for (product_id, product_data) in parsed.products {
            let Some(quick_status) = product_data.quick_status else {
                continue;
            };
            let snapshot = ProductSnapshot {
                buy_price: quick_status.buy_price,
                sell_price: quick_status.sell_price,
                buy_volume: quick_status.buy_volume,
                sell_volume: quick_status.sell_volume,
                buy_orders: quick_status.buy_orders,
                sell_orders: quick_status.sell_orders,
            };
            if last_snapshot
                .get(&product_id)
                .is_some_and(|previous| previous == &snapshot)
            {
                continue;
            }
            last_snapshot.insert(product_id.clone(), snapshot.clone());
            changed.push(BazaarData {
                product_id,
                buy_price: snapshot.buy_price,
                sell_price: snapshot.sell_price,
                buy_volume: snapshot.buy_volume,
                sell_volume: snapshot.sell_volume,
                buy_orders: snapshot.buy_orders,
                sell_orders: snapshot.sell_orders,
                timestamp,
                expires_at: Some(db::raw_expires_at(timestamp)),
            });
        }
    }

    if changed.is_empty() {
        return Ok(());
    }

    db::insert_bazaar_data_batch(&state.db, &changed).await?;
    db::upsert_latest_bazaar_data_batch(&state.db, &changed).await?;
    db::upsert_chart_candles_batch(&state.db, &changed).await?;
    println!("Tracker stored {} changed products", changed.len());

    Ok(())
}

#[derive(Debug, Deserialize)]
struct HypixelBazaarResponse {
    #[serde(default)]
    success: bool,
    #[serde(default, rename = "lastUpdated")]
    last_updated: Option<i64>,
    #[serde(default)]
    products: HashMap<String, HypixelProduct>,
}

#[derive(Debug, Deserialize)]
struct HypixelProduct {
    #[serde(default)]
    quick_status: Option<HypixelQuickStatus>,
}

#[derive(Debug, Deserialize)]
struct HypixelQuickStatus {
    #[serde(default, rename = "buyPrice")]
    buy_price: f64,
    #[serde(default, rename = "sellPrice")]
    sell_price: f64,
    #[serde(default, rename = "buyVolume")]
    buy_volume: i64,
    #[serde(default, rename = "sellVolume")]
    sell_volume: i64,
    #[serde(default, rename = "buyOrders")]
    buy_orders: i64,
    #[serde(default, rename = "sellOrders")]
    sell_orders: i64,
}

#[cfg(test)]
mod tests {
    use super::HypixelBazaarResponse;
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

        let mut last_snapshot = state.last_snapshot.write().await;
        assert!(
            last_snapshot
                .insert("WHEAT".to_string(), snapshot.clone())
                .is_none()
        );
        assert_eq!(last_snapshot.get("WHEAT"), Some(&snapshot));
    }

    #[test]
    fn hypixel_typed_parse_reads_only_quick_status() {
        let raw = r#"{
            "success": true,
            "lastUpdated": 123,
            "products": {
                "WHEAT": {
                    "sell_summary": [{ "amount": 1 }],
                    "buy_summary": [{ "amount": 2 }],
                    "quick_status": {
                        "buyPrice": 2.5,
                        "sellPrice": 1.5,
                        "buyVolume": 10,
                        "sellVolume": 20,
                        "buyOrders": 3,
                        "sellOrders": 4
                    }
                }
            }
        }"#;
        let parsed: HypixelBazaarResponse = serde_json::from_str(raw).unwrap();
        let quick = parsed.products["WHEAT"].quick_status.as_ref().unwrap();

        assert!(parsed.success);
        assert_eq!(parsed.last_updated, Some(123));
        assert_eq!(quick.buy_price, 2.5);
        assert_eq!(quick.sell_orders, 4);
    }
}
