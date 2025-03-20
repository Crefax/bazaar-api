#![allow(non_snake_case)]
use actix_web::{web, App, HttpServer};
use mongodb::Client;
use std::sync::Arc;
use tokio::sync::Mutex;

mod api;
mod tracker;
mod db;
mod models;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize MongoDB connection
    let client = Client::with_uri_str("mongodb://localhost:27017")
        .await
        .expect("Failed to create MongoDB client");
    let db = client.database("skyblock");
    let db = Arc::new(Mutex::new(db));

    // Start the tracker
    let db_clone = db.clone();
    tokio::spawn(async move {
        tracker::start_tracker(db_clone).await;
    });

    // Start the API server
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(db.clone()))
            .service(api::get_bazaar_data)
            .service(api::get_bazaar_data_history)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
