#![allow(non_snake_case)]

use actix_cors::Cors;
use actix_web::{App, HttpServer, middleware, web};
use mongodb::Client;
use std::path::{Path, PathBuf};

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
    load_dotenv();
    let config = config::AppConfig::from_env();
    println!(
        "Runtime config: app_env={:?}, bind_addr={}, mongodb_db={}, admin_api_key_configured={}, redis_url_configured={}, redis_required={}, request_logging={}, response_compression={}",
        config.app_env,
        config.bind_addr,
        config.mongodb_db,
        config.admin_api_key.is_some(),
        config.redis_url.is_some(),
        config.redis_required(),
        config.request_logging,
        config.response_compression
    );

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
            .wrap(middleware::Condition::new(
                state.config.response_compression,
                middleware::Compress::default(),
            ))
            .wrap(middleware::Condition::new(
                state.config.request_logging,
                middleware::Logger::default(),
            ))
            .wrap(middleware::NormalizePath::trim())
            .wrap(default_headers)
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
    })
    .bind(bind_addr)?
    .run()
    .await
}

fn load_dotenv() {
    if let Ok(path) = dotenvy::dotenv() {
        println!("Loaded environment from {}", path.display());
        return;
    }

    for path in dotenv_candidates_from_exe() {
        if path.is_file() && dotenvy::from_path(&path).is_ok() {
            println!("Loaded environment from {}", path.display());
            return;
        }
    }

    println!("No .env file found; using process environment and defaults");
}

fn dotenv_candidates_from_exe() -> Vec<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|dir| {
            dir.ancestors()
                .take(4)
                .map(|ancestor| ancestor.join(".env"))
                .collect()
        })
        .unwrap_or_default()
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
            rate_limit_reservation_size: 128,
            admin_json_limit_bytes: 16 * 1024,
            request_logging: false,
            response_compression: false,
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
    async fn admin_panel_is_not_shadowed_by_public_scope() {
        let config = test_config();
        let app = test::init_service(
            App::new().service(api::admin_panel).service(
                web::scope("")
                    .wrap(build_public_cors(&config))
                    .service(api::health),
            ),
        )
        .await;

        let request = test::TestRequest::get().uri("/admin").to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), actix_web::http::StatusCode::OK);
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
