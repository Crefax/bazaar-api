use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
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

// Modern API Response wrapper
#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
    pub pagination: Option<PaginationInfo>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PaginationInfo {
    pub page: u32,
    pub limit: u32,
    pub total_items: u64,
    pub total_pages: u32,
    pub has_next: bool,
    pub has_previous: bool,
    pub next_cursor: Option<String>,
    pub previous_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TimeframeQuery {
    pub hours: Option<u32>,
    pub days: Option<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FilterQuery {
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub min_volume: Option<i64>,
    pub max_volume: Option<i64>,
    pub sort_by: Option<String>, // "price", "volume", "timestamp"
    pub sort_order: Option<String>, // "asc", "desc"
}

// Aggregated data models
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BazaarAggregatedData {
    pub product_id: String,
    pub avg_buy_price: f64,
    pub avg_sell_price: f64,
    pub min_buy_price: f64,
    pub max_buy_price: f64,
    pub min_sell_price: f64,
    pub max_sell_price: f64,
    pub total_buy_volume: i64,
    pub total_sell_volume: i64,
    pub avg_buy_volume: f64,
    pub avg_sell_volume: f64,
    pub data_points: i64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub aggregation_type: String, // "minutely", "hourly", "daily", "weekly", "monthly"
    pub interval_minutes: i32, // Kaç dakikalık interval: 1, 60, 1440, 10080, 43200
}

// Veri yaşam döngüsü için configuration
#[derive(Debug, Clone)]
pub struct DataLifecycleConfig {
    pub minutely_retention_hours: i64,    // Dakikalık veri ne kadar süre saklanacak (varsayılan: 24 saat)
    pub hourly_retention_days: i64,       // Saatlik veri ne kadar süre saklanacak (varsayılan: 7 gün)  
    pub daily_retention_days: i64,        // Günlük veri ne kadar süre saklanacak (varsayılan: 30 gün)
    pub weekly_retention_days: i64,       // Haftalık veri ne kadar süre saklanacak (varsayılan: 365 gün)
    pub compression_enabled: bool,
}

impl Default for DataLifecycleConfig {
    fn default() -> Self {
        Self {
            minutely_retention_hours: 24,   // 1 gün sonra dakikalık → saatlik
            hourly_retention_days: 7,       // 1 hafta sonra saatlik → günlük
            daily_retention_days: 30,       // 1 ay sonra günlük → haftalık
            weekly_retention_days: 365,     // 1 yıl sonra haftalık → aylık
            compression_enabled: true,
        }
    }
}

// Compression state tracking per product
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompressionState {
    pub product_id: String,
    pub last_minutely_compression: DateTime<Utc>,
    pub last_hourly_compression: DateTime<Utc>,
    pub last_daily_compression: DateTime<Utc>,
    pub last_weekly_compression: DateTime<Utc>,
    pub compression_in_progress: bool,
    pub current_operation: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Compression log entry for detailed tracking
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CompressionLog {
    pub id: Option<mongodb::bson::oid::ObjectId>,
    pub product_id: String,
    pub compression_type: String, // "minutely_to_hourly", "hourly_to_daily", etc.
    pub source_period_start: DateTime<Utc>,
    pub source_period_end: DateTime<Utc>,
    pub compressed_period_start: DateTime<Utc>,
    pub compressed_period_end: DateTime<Utc>,
    pub source_records_count: i64,
    pub compressed_records_count: i64,
    pub bytes_saved: Option<i64>,
    pub compression_duration_ms: i64,
    pub status: String, // "success", "failed", "partial"
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

// Sistem durumu tracking
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemState {
    pub last_compression_cycle: DateTime<Utc>,
    pub compression_in_progress: bool,
    pub current_operation: Option<String>,
    pub processed_products: Vec<String>,
    pub failed_products: Vec<String>,
    pub compression_errors: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            pagination: None,
            timestamp: Utc::now(),
        }
    }

    pub fn success_with_pagination(data: T, pagination: PaginationInfo) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            pagination: Some(pagination),
            timestamp: Utc::now(),
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message),
            pagination: None,
            timestamp: Utc::now(),
        }
    }
} 