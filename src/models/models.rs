use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct BazaarData {
    pub product_id: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub buy_volume: i64,
    pub sell_volume: i64,
    pub buy_orders: i64,
    pub sell_orders: i64,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
} 