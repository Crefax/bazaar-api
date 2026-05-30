#![allow(non_snake_case)]

use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use mongodb::Client;

mod api;
mod config;
mod db;
mod models;
mod security;
mod shared_store;
mod state;
mod tracker;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = config::AppConfig::from_env();

    let client = Client::with_uri_str(&config.mongodb_uri)
        .await
        .expect("Failed to create MongoDB client");
    let db = client.database(&config.mongodb_db);

    db::ensure_indexes(&db)
        .await
        .expect("Failed to create required MongoDB indexes");

    let state = state::AppState::new(db, config.clone()).await;

    let tracker_state = state.clone();
    tokio::spawn(async move {
        tracker::start_tracker(tracker_state).await;
    });

    let rollup_state = state.clone();
    tokio::spawn(async move {
        tracker::start_rollup_scheduler(rollup_state).await;
    });

    let bind_addr = config.bind_addr.clone();
    println!("Bazaar API listening on {}", bind_addr);

    HttpServer::new(move || {
        let public_cors = build_public_cors(&state.config);
        let mut default_headers = middleware::DefaultHeaders::new()
            .add(("X-API-Version", "2.0"))
            .add(("X-Content-Type-Options", "nosniff"))
            .add(("X-Frame-Options", "DENY"))
            .add(("Referrer-Policy", "no-referrer"))
            .add((
                "Permissions-Policy",
                "geolocation=(), microphone=(), camera=()",
            ));
        if state.config.app_env.is_production() {
            default_headers = default_headers.add((
                "Strict-Transport-Security",
                "max-age=31536000; includeSubDomains",
            ));
        }

        App::new()
            .app_data(web::Data::new(state.clone()))
            .app_data(web::JsonConfig::default().limit(state.config.admin_json_limit_bytes))
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::default())
            .wrap(middleware::NormalizePath::trim())
            .wrap(default_headers)
            .service(
                web::scope("")
                    .wrap(public_cors)
                    .service(api::health)
                    .service(api::ready)
                    .service(api::openapi)
                    .service(api::list_products)
                    .service(api::latest_many)
                    .service(api::latest_one)
                    .service(api::candles)
                    .service(api::series),
            )
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
            .service(api::get_compression_stats_v2)
            .service(api::get_compression_logs_v2)
    })
    .bind(bind_addr)?
    .run()
    .await
}

fn build_public_cors(config: &config::AppConfig) -> Cors {
    let mut cors = Cors::default()
        .allow_any_method()
        .allow_any_header()
        .max_age(3600);

    if config.cors_allowed_origins.is_empty() && !config.app_env.is_production() {
        cors = cors.allow_any_origin();
    } else {
        for origin in &config.cors_allowed_origins {
            cors = cors.allowed_origin(origin);
        }
    }

    cors
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, test};
    use mongodb::Client;
    use std::sync::Arc;

    fn test_config() -> Arc<config::AppConfig> {
        Arc::new(config::AppConfig {
            mongodb_uri: "mongodb://localhost:27017".to_string(),
            mongodb_db: "test".to_string(),
            bind_addr: "127.0.0.1:0".to_string(),
            app_env: config::AppEnvironment::Development,
            admin_api_key: Some("admin".to_string()),
            cors_allowed_origins: vec![],
            trust_proxy: false,
            trusted_proxy_cidrs: vec![],
            redis_url: None,
            require_redis: false,
            api_key_hash_pepper: "test-pepper".to_string(),
            admin_cookie_secure: false,
            cache_max_entries: 100,
            rate_limit_max_keys: 100,
            admin_json_limit_bytes: 16 * 1024,
        })
    }

    #[actix_web::test]
    async fn legacy_v1_route_is_not_registered() {
        let config = test_config();
        let client = Client::with_uri_str(&config.mongodb_uri).await.unwrap();
        let state = state::AppState::new(client.database(&config.mongodb_db), config).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .service(api::health)
                .service(api::latest_one),
        )
        .await;

        let request = test::TestRequest::get()
            .uri("/api/v1/skyblock/bazaar/WHEAT")
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn admin_compression_requires_admin_auth() {
        let config = test_config();
        let client = Client::with_uri_str(&config.mongodb_uri).await.unwrap();
        let state = state::AppState::new(client.database(&config.mongodb_db), config).await;
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .service(api::get_compression_stats_v2),
        )
        .await;

        let request = test::TestRequest::get()
            .uri("/api/v2/admin/compression/stats")
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), actix_web::http::StatusCode::UNAUTHORIZED);
    }
}
