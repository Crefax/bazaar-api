use actix_web::{web, HttpResponse, Responder, get};
use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::db;
use crate::models::{ApiResponse, PaginationQuery, TimeframeQuery, FilterQuery};
use chrono::{DateTime, Utc, Duration};

// API v1 - Mevcut endpoint (en son veri)
#[get("/api/v1/skyblock/bazaar/{product_id}")]
pub async fn get_bazaar_data_v1(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
) -> impl Responder {
    let db = db.lock().await;
    match db::get_latest_bazaar_data(&db, &product_id).await {
        Ok(Some(data)) => HttpResponse::Ok().json(ApiResponse::success(data)),
        Ok(None) => HttpResponse::NotFound().json(ApiResponse::<()>::error("Product not found".to_string())),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!("Database error: {}", e))),
    }
}

// API v1 - Pagination destekli geçmiş veriler
#[get("/api/v1/skyblock/bazaar/{product_id}/history")]
pub async fn get_bazaar_data_history_v1(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
    pagination: web::Query<PaginationQuery>,
    timeframe: web::Query<TimeframeQuery>,
    filters: web::Query<FilterQuery>,
) -> impl Responder {
    let db = db.lock().await;
    
    // Sayfa ve limit değerlerini al (varsayılan değerler)
    let page = pagination.page.unwrap_or(1).max(1);
    let limit = pagination.limit.unwrap_or(50).min(1000).max(1); // Max 1000, min 1
    
    // Zaman aralığını hesapla
    let (start_time, end_time) = match (&timeframe.start_date, &timeframe.end_date) {
        (Some(start), Some(end)) => {
            match (start.parse::<DateTime<Utc>>(), end.parse::<DateTime<Utc>>()) {
                (Ok(s), Ok(e)) => (Some(s), Some(e)),
                _ => {
                    return HttpResponse::BadRequest()
                        .json(ApiResponse::<()>::error("Invalid date format. Use ISO 8601 format.".to_string()));
                }
            }
        }
        _ => {
            let hours = timeframe.hours.or(timeframe.days.map(|d| d * 24)).unwrap_or(24);
            let start = Utc::now() - Duration::hours(hours as i64);
            let end = Utc::now();
            (Some(start), Some(end))
        }
    };
    
    match db::get_bazaar_data_paginated(
        &db,
        &product_id,
        page,
        limit,
        start_time,
        end_time,
        filters.min_price,
        filters.max_price,
        filters.sort_by.clone(),
        filters.sort_order.clone(),
    ).await {
        Ok((data, pagination_info)) => {
            HttpResponse::Ok().json(ApiResponse::success_with_pagination(data, pagination_info))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<()>::error(format!("Database error: {}", e))),
    }
}

// API v1 - Agregasyon destekli özetlenmiş veriler (grafikler için)
#[get("/api/v1/skyblock/bazaar/{product_id}/summary")]
pub async fn get_bazaar_data_summary_v1(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
    query: web::Query<AggregationQuery>,
) -> impl Responder {
    let db = db.lock().await;
    
    // Agregasyon tipini kontrol et
    let aggregation_type = query.aggregation.as_deref().unwrap_or("hourly");
    if !["hourly", "daily", "weekly", "monthly"].contains(&aggregation_type) {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<()>::error("Invalid aggregation type. Use: hourly, daily, weekly, monthly".to_string()));
    }
    
    // Zaman aralığını hesapla
    let (start_time, end_time) = match (&query.start_date, &query.end_date) {
        (Some(start), Some(end)) => {
            match (start.parse::<DateTime<Utc>>(), end.parse::<DateTime<Utc>>()) {
                (Ok(s), Ok(e)) => (s, e),
                _ => {
                    return HttpResponse::BadRequest()
                        .json(ApiResponse::<()>::error("Invalid date format. Use ISO 8601 format.".to_string()));
                }
            }
        }
        _ => {
            // Varsayılan zaman aralıkları
            let (duration, _default_aggregation) = match aggregation_type {
                "hourly" => (Duration::days(1), "hourly"),   // Son 1 gün için saatlik
                "daily" => (Duration::days(30), "daily"),    // Son 30 gün için günlük
                "weekly" => (Duration::days(90), "weekly"),  // Son 90 gün için haftalık
                "monthly" => (Duration::days(365), "monthly"), // Son 1 yıl için aylık
                _ => (Duration::days(1), "hourly"),
            };
            let end = Utc::now();
            let start = end - duration;
            (start, end)
        }
    };
    
    // Akıllı veri getirme - compressed data varsa kullan, yoksa real-time aggregation
    match db::get_bazaar_data_smart(&db, &product_id, aggregation_type, start_time, end_time).await {
        Ok(data) => HttpResponse::Ok().json(ApiResponse::success(data)),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<()>::error(format!("Database error: {}", e))),
    }
}

// Geriye dönük uyumluluk için eski endpoint'ler
#[get("/api/skyblock/bazaar/{product_id}")]
pub async fn get_bazaar_data(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
) -> impl Responder {
    let db_ref = db.lock().await;
    match db::get_latest_bazaar_data(&db_ref, &product_id).await {
        Ok(Some(data)) => HttpResponse::Ok().json(data),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
    }
}

#[get("/api/skyblock/bazaar/{product_id}/history")]
pub async fn get_bazaar_data_history(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
    query: web::Query<LegacyTimeframeQuery>,
) -> impl Responder {
    let db = db.lock().await;
    let start_time = Utc::now() - Duration::hours(query.hours.unwrap_or(24) as i64);
    let end_time = Utc::now();

    match db::get_bazaar_data_by_timeframe(&db, &product_id, start_time, end_time).await {
        Ok(data) => HttpResponse::Ok().json(data),
        Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
    }
}

// Query structs
#[derive(serde::Deserialize)]
pub struct AggregationQuery {
    pub aggregation: Option<String>, // "hourly", "daily", "weekly", "monthly"
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct LegacyTimeframeQuery {
    pub hours: Option<u32>,
}

// === COMPRESSION TRACKING ENDPOINTS ===

// Compression statistics endpoint
#[get("/api/v1/admin/compression/stats")]
pub async fn get_compression_stats(
    db: web::Data<Arc<Mutex<Database>>>,
) -> impl Responder {
    let db = db.lock().await;
    
    match db::get_compression_stats(&db).await {
        Ok(stats) => HttpResponse::Ok().json(ApiResponse::success(stats)),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<()>::error(format!("Database error: {}", e))),
    }
}

// Compression logs endpoint
#[get("/api/v1/admin/compression/logs")]
pub async fn get_compression_logs_api(
    db: web::Data<Arc<Mutex<Database>>>,
    query: web::Query<CompressionLogsQuery>,
) -> impl Responder {
    let db = db.lock().await;
    
    match db::get_compression_logs(
        &db, 
        query.product_id.as_deref(), 
        query.compression_type.as_deref(),
        query.limit
    ).await {
        Ok(logs) => HttpResponse::Ok().json(ApiResponse::success(logs)),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<()>::error(format!("Database error: {}", e))),
    }
}

#[derive(serde::Deserialize)]
pub struct CompressionLogsQuery {
    pub product_id: Option<String>,
    pub compression_type: Option<String>, // "minutely_to_hourly", "hourly_to_daily", etc.
    pub limit: Option<i64>,
} 