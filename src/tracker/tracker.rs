use crate::db;
use crate::models::BazaarData;
use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};

pub async fn start_tracker(db: Arc<Mutex<Database>>) {
    let mut interval = interval(Duration::from_secs(60)); // Update interval: 1 minute

    loop {
        interval.tick().await;
        if let Err(e) = update_bazaar_data(&db).await {
            eprintln!("Error updating bazaar data: {}", e);
        }
    }
}

async fn update_bazaar_data(db: &Arc<Mutex<Database>>) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.hypixel.net/v2/skyblock/bazaar")
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    if let Some(products) = response["products"].as_object() {
        let db = db.lock().await;
        for (product_id, product_data) in products {
            if let Some(quick_status) = product_data["quick_status"].as_object() {
                let bazaar_data = BazaarData {
                    product_id: product_id.clone(),
                    buy_price: quick_status["buyPrice"]
                        .as_f64()
                        .unwrap_or(0.0),
                    sell_price: quick_status["sellPrice"]
                        .as_f64()
                        .unwrap_or(0.0),
                    buy_volume: quick_status["buyVolume"]
                        .as_i64()
                        .unwrap_or(0),
                    sell_volume: quick_status["sellVolume"]
                        .as_i64()
                        .unwrap_or(0),
                    buy_orders: quick_status["buyOrders"]
                        .as_i64()
                        .unwrap_or(0),
                    sell_orders: quick_status["sellOrders"]
                        .as_i64()
                        .unwrap_or(0),
                    timestamp: chrono::Utc::now(),
                };

                db::insert_bazaar_data(&db, bazaar_data).await?;
            }
        }
    }

    Ok(())
} 