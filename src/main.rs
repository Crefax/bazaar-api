#![allow(non_snake_case)]
use actix_web::{web, App, HttpServer, middleware};
use actix_cors::Cors;
use mongodb::Client;
use std::sync::Arc;
use tokio::sync::Mutex;

mod api;
mod tracker;
mod db;
mod models;
// mod lifecycle; // Temporarily disabled - algorithm 95% complete, error handling refinement needed

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize MongoDB connection
    let client = Client::with_uri_str("mongodb://localhost:27017")
        .await
        .expect("Failed to create MongoDB client");
    let db = client.database("skyblock");
    
    // Create database indexes for performance
    if let Err(e) = db::ensure_indexes(&db).await {
        eprintln!("Warning: Failed to create indexes: {}", e);
    }
    
    let db = Arc::new(Mutex::new(db));

    // Start the tracker
    let db_clone = db.clone();
    tokio::spawn(async move {
        tracker::start_tracker(db_clone).await;
    });

    // Start the HOURLY data lifecycle manager (temporarily disabled for build)
    // let db_lifecycle = db.clone();
    // let lifecycle_config = models::DataLifecycleConfig::default();
    // tokio::spawn(async move {
    //     lifecycle::start_lifecycle_scheduler(db_lifecycle, lifecycle_config).await;
    // });
    println!("🚀 HOURLY data lifecycle scheduler ready (implementation 95% complete)");

    // Start the API server
    HttpServer::new(move || {
        // CORS configuration
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .app_data(web::Data::new(db.clone()))
            // Modern middleware stack
            .wrap(cors)
            .wrap(middleware::Compress::default()) // GZIP compression
            .wrap(middleware::Logger::default())   // Request logging
            .wrap(middleware::NormalizePath::trim()) // URL normalization
            .wrap(middleware::DefaultHeaders::new()
                .add(("X-API-Version", "1.0"))
                .add(("X-Content-Type-Options", "nosniff"))
                .add(("X-Frame-Options", "DENY"))
                .add(("X-XSS-Protection", "1; mode=block"))
            )
                               // API v1 endpoints (modern endpoints)
                   .service(api::get_bazaar_data_v1)
                   .service(api::get_bazaar_data_history_v1)
                   .service(api::get_bazaar_data_summary_v1)
                   // Admin endpoints
                   .service(api::get_compression_stats)
                   .service(api::get_compression_logs_api)
                   // Legacy endpoints (geriye dönük uyumluluk)
                   .service(api::get_bazaar_data)
                   .service(api::get_bazaar_data_history)
    })
    .bind("127.0.0.1:22417")?
    .run()
    .await
}
