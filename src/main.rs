#![allow(non_snake_case)]
use actix_web::{web, App, HttpServer};
use mongodb::Client;
use tokio::time::{interval, Duration};

mod api;
mod db;
mod models;
mod utils;

struct AppState {
    client: Client,
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_uri = "mongodb://localhost:27017"; // MongoDB URI
    let interval_time = 600; // 10 minutes

    let client_options = mongodb::options::ClientOptions::parse(client_uri).await?;
    let client = Client::with_options(client_options)?;

    db::create_indexes(&client).await;

    // Reset Check Limits
    let client_clone = client.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(interval_time));
        loop {
            interval.tick().await;
            if let Err(e) = db::reset_check_limits(&client_clone).await {
                eprintln!("Error resetting control limits: {}", e);
            }
        }
    });

    // Start HTTP Server
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(AppState {
                client: client.clone(),
            }))
            .service(api::get_latest_product)
            .service(api::get_latest_field)
            .service(api::get_fields)
            .default_service(web::route().to(api::not_found))
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await?;

    Ok(())
}
