use actix_web::{web, HttpResponse, Responder, get};
use mongodb::Database;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::db;
use serde::Deserialize;

#[get("/api/skyblock/bazaar/{product_id}")]
pub async fn get_bazaar_data(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
) -> impl Responder {
    let db = db.lock().await;
    match db::get_latest_bazaar_data(&db, &product_id).await {
        Ok(Some(data)) => HttpResponse::Ok().json(data),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
    }
}

#[get("/api/skyblock/bazaar/{product_id}/history")]
pub async fn get_bazaar_data_history(
    db: web::Data<Arc<Mutex<Database>>>,
    product_id: web::Path<String>,
    query: web::Query<TimeframeQuery>,
) -> impl Responder {
    let db = db.lock().await;
    let start_time = chrono::Utc::now() - chrono::Duration::hours(query.hours.unwrap_or(24) as i64);
    let end_time = chrono::Utc::now();

    match db::get_bazaar_data_by_timeframe(&db, &product_id, start_time, end_time).await {
        Ok(data) => HttpResponse::Ok().json(data),
        Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
    }
}

#[derive(Deserialize)]
pub struct TimeframeQuery {
    hours: Option<u32>,
} 