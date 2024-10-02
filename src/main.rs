#![allow(non_snake_case)]
use actix_web::{web, App, HttpServer};
use mongodb::Client;

mod api;
mod db;
mod models;
mod utils;

struct AppState {
    client: Client,
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_uri = "mongodb://localhost:27017";
    let client_options = mongodb::options::ClientOptions::parse(client_uri).await?;
    let client = Client::with_options(client_options)?;

    db::create_indexes(&client).await;

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
