use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BazaarLatest {
    pub product_id: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub buy_volume: i64,
    pub sell_volume: i64,
    pub buy_orders: i64,
    pub sell_orders: i64,
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl From<BazaarData> for BazaarLatest {
    fn from(data: BazaarData) -> Self {
        Self {
            product_id: data.product_id,
            buy_price: data.buy_price,
            sell_price: data.sell_price,
            buy_volume: data.buy_volume,
            sell_volume: data.sell_volume,
            buy_orders: data.buy_orders,
            sell_orders: data.sell_orders,
            timestamp: data.timestamp,
            updated_at: Utc::now(),
        }
    }
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
pub struct ProblemResponse {
    pub success: bool,
    pub error: ProblemDetails,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProblemDetails {
    pub code: String,
    pub message: String,
    pub status: u16,
}

impl ProblemResponse {
    pub fn new(code: impl Into<String>, message: impl Into<String>, status: u16) -> Self {
        Self {
            success: false,
            error: ProblemDetails {
                code: code.into(),
                message: message.into(),
                status,
            },
            timestamp: Utc::now(),
        }
    }
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
    #[serde(rename = "cursor")]
    pub _cursor: Option<String>,
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
    pub sort_by: Option<String>,    // "price", "volume", "timestamp"
    pub sort_order: Option<String>, // "asc", "desc"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BazaarCandle {
    pub product_id: String,
    pub interval: String,
    pub metric: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub period_start: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub period_end: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub value_sum: f64,
    pub volume: i64,
    pub buy_volume_sum: i64,
    pub sell_volume_sum: i64,
    pub sample_count: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CandlePoint {
    pub t: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub samples: i64,
}

#[derive(Debug, Serialize)]
pub struct SeriesPoint {
    pub t: DateTime<Utc>,
    pub value: f64,
    pub samples: i64,
}

#[derive(Debug, Deserialize)]
pub struct ChartQuery {
    pub interval: Option<String>,
    pub range: Option<String>,
    pub metric: Option<String>,
    pub stat: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct LatestQuery {
    pub ids: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccessPolicy {
    #[serde(rename = "_id")]
    pub id: String,
    pub anonymous_public_enabled: bool,
    pub anonymous_rate_limit_per_minute: u32,
    pub default_user_rate_limit_per_minute: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AccessPolicyView {
    pub id: String,
    pub anonymous_public_enabled: bool,
    pub anonymous_rate_limit_per_minute: u32,
    pub default_user_rate_limit_per_minute: u32,
    pub updated_at: DateTime<Utc>,
}

impl From<AccessPolicy> for AccessPolicyView {
    fn from(policy: AccessPolicy) -> Self {
        Self {
            id: policy.id,
            anonymous_public_enabled: policy.anonymous_public_enabled,
            anonymous_rate_limit_per_minute: policy.anonymous_rate_limit_per_minute,
            default_user_rate_limit_per_minute: policy.default_user_rate_limit_per_minute,
            updated_at: policy.updated_at,
        }
    }
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            id: "public_api".to_string(),
            anonymous_public_enabled: true,
            anonymous_rate_limit_per_minute: 120,
            default_user_rate_limit_per_minute: 600,
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiKeyRecord {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub name: String,
    pub owner_email: Option<String>,
    pub key_prefix: String,
    pub key_hash: String,
    pub scopes: Vec<String>,
    pub rate_limit_per_minute: u32,
    pub daily_quota: Option<u32>,
    pub status: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime_optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime_optional")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeySummary {
    pub id: String,
    pub name: String,
    pub owner_email: Option<String>,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub rate_limit_per_minute: u32,
    pub daily_quota: Option<u32>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<ApiKeyRecord> for ApiKeySummary {
    fn from(record: ApiKeyRecord) -> Self {
        Self {
            id: record.id.map(|id| id.to_hex()).unwrap_or_default(),
            name: record.name,
            owner_email: record.owner_email,
            key_prefix: record.key_prefix,
            scopes: record.scopes,
            rate_limit_per_minute: record.rate_limit_per_minute,
            daily_quota: record.daily_quota,
            status: record.status,
            created_at: record.created_at,
            updated_at: record.updated_at,
            last_used_at: record.last_used_at,
            expires_at: record.expires_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CreatedApiKey {
    pub key: String,
    pub record: ApiKeySummary,
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
    pub interval_minutes: i32,    // Kaç dakikalık interval: 1, 60, 1440, 10080, 43200
}

// Veri yaşam döngüsü için configuration
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DataLifecycleConfig {
    pub minutely_retention_hours: i64, // Dakikalık veri ne kadar süre saklanacak (varsayılan: 24 saat)
    pub hourly_retention_days: i64,    // Saatlik veri ne kadar süre saklanacak (varsayılan: 7 gün)
    pub daily_retention_days: i64,     // Günlük veri ne kadar süre saklanacak (varsayılan: 30 gün)
    pub weekly_retention_days: i64, // Haftalık veri ne kadar süre saklanacak (varsayılan: 365 gün)
    pub compression_enabled: bool,
}

impl Default for DataLifecycleConfig {
    fn default() -> Self {
        Self {
            minutely_retention_hours: 24, // 1 gün sonra dakikalık → saatlik
            hourly_retention_days: 7,     // 1 hafta sonra saatlik → günlük
            daily_retention_days: 30,     // 1 ay sonra günlük → haftalık
            weekly_retention_days: 365,   // 1 yıl sonra haftalık → aylık
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
#[allow(dead_code)]
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
