#![allow(non_snake_case)]

use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use mongodb::Client;

mod api;
mod config;
mod db;
mod models;
mod security;
mod state;
mod tracker;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = config::AppConfig::from_env();

    let client = Client::with_uri_str(&config.mongodb_uri)
        .await
        .expect("Failed to create MongoDB client");
    let db = client.database(&config.mongodb_db);

    if let Err(e) = db::ensure_indexes(&db).await {
        eprintln!("Warning: Failed to create indexes: {}", e);
    }

    let state = state::AppState::new(db, config.clone());

    let tracker_state = state.clone();
    tokio::spawn(async move {
        tracker::start_tracker(tracker_state).await;
    });

    let retention_state = state.clone();
    tokio::spawn(async move {
        tracker::start_retention_scheduler(retention_state).await;
    });

    let bind_addr = config.bind_addr.clone();
    println!("Bazaar API listening on {}", bind_addr);

    HttpServer::new(move || {
        let cors = build_cors(&state.config);

        App::new()
            .app_data(web::Data::new(state.clone()))
            .wrap(cors)
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
            .wrap(middleware::NormalizePath::trim())
            .wrap(
                middleware::DefaultHeaders::new()
                    .add(("X-API-Version", "2.0"))
                    .add(("X-Content-Type-Options", "nosniff"))
                    .add(("X-Frame-Options", "DENY"))
                    .add(("X-XSS-Protection", "1; mode=block")),
            )
            .service(api::health)
            .service(api::ready)
            .service(api::openapi)
            .service(api::list_products)
            .service(api::latest_many)
            .service(api::latest_one)
            .service(api::candles)
            .service(api::series)
            .service(api::admin_panel)
            .service(api::login)
            .service(api::logout)
            .service(api::list_api_keys)
            .service(api::create_api_key)
            .service(api::update_api_key)
            .service(api::rotate_api_key)
            .service(api::revoke_api_key)
            .service(api::get_access_policy)
            .service(api::update_access_policy)
            .service(api::get_bazaar_data_v1)
            .service(api::get_bazaar_data_history_v1)
            .service(api::get_bazaar_data_summary_v1)
            .service(api::get_compression_stats)
            .service(api::get_compression_logs_api)
            .service(api::get_bazaar_data)
            .service(api::get_bazaar_data_history)
    })
    .bind(bind_addr)?
    .run()
    .await
}

fn build_cors(config: &config::AppConfig) -> Cors {
    let mut cors = Cors::default()
        .allow_any_method()
        .allow_any_header()
        .max_age(3600);

    if config.cors_allowed_origins.is_empty() {
        cors = cors.allow_any_origin();
    } else {
        for origin in &config.cors_allowed_origins {
            cors = cors.allowed_origin(origin);
        }
    }

    cors
}
