use crate::db;
use crate::models::{
    ApiResponse, BazaarCandle, CandleMetric, CandlePoint, ChartQuery, LatestQuery, SeriesPoint,
};
use crate::security;
use crate::state::AppState;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use mongodb::bson::doc;
use serde::Serialize;
use serde_json::json;
use std::future::Future;
use std::time::Duration as StdDuration;

const BAZAAR_READ_SCOPE: &str = "bazaar:read";
const PUBLIC_CACHE_VERSION: &str = "v3";

#[get("/health")]
pub async fn health() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "success": true,
        "data": {
            "status": "ok"
        },
        "error": null,
        "timestamp": Utc::now()
    }))
}

#[get("/ready")]
pub async fn ready(state: web::Data<AppState>) -> impl Responder {
    let mongo_ok = match state.db.run_command(doc! { "ping": 1 }).await {
        Ok(_) => true,
        Err(error) => {
            eprintln!("MongoDB readiness failed: {}", error);
            false
        }
    };
    let store = state.security_store.readiness().await;
    let ready = mongo_ok && store.ready;
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let data = if state.config.app_env.is_production() {
        json!({ "status": if ready { "ok" } else { "degraded" } })
    } else {
        json!({
            "status": if ready { "ok" } else { "degraded" },
            "mongo": if mongo_ok { "ok" } else { "unavailable" },
            "security_store": store.backend,
            "redis_required": store.redis_required,
            "redis_available": store.redis_available,
        })
    };

    HttpResponse::build(status).json(json!({
        "success": ready,
        "data": data,
        "error": null,
        "timestamp": Utc::now()
    }))
}

#[get("/api/v2/openapi.json")]
pub async fn openapi() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Bazaar API",
            "version": "2.0.0"
        },
        "paths": {
            "/api/v2/skyblock/bazaar/products": { "get": { "summary": "List products" } },
            "/api/v2/skyblock/bazaar/products/latest": { "get": { "summary": "Get latest data for many products" } },
            "/api/v2/skyblock/bazaar/products/{product_id}/latest": { "get": { "summary": "Get latest product data" } },
            "/api/v2/skyblock/bazaar/products/{product_id}/candles": { "get": { "summary": "Get OHLCV candles" } },
            "/api/v2/skyblock/bazaar/products/{product_id}/series": { "get": { "summary": "Get chart series" } },
            "/api/v2/admin/api-keys": { "get": { "summary": "List API keys" }, "post": { "summary": "Create API key" } },
            "/api/v2/admin/access-policy": { "get": { "summary": "Read public access policy" }, "patch": { "summary": "Update public access policy" } },
            "/api/v2/admin/compression/stats": { "get": { "summary": "Read compression stats" } },
            "/api/v2/admin/compression/logs": { "get": { "summary": "Read compression logs" } }
        },
        "components": {
            "securitySchemes": {
                "userApiKey": { "type": "apiKey", "in": "header", "name": "X-API-Key" },
                "adminApiKey": { "type": "apiKey", "in": "header", "name": "X-Admin-Api-Key" }
            }
        }
    }))
}

#[get("/api/v2/skyblock/bazaar/products")]
pub async fn list_products(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    let auth = match security::authorize_public(&req, state.get_ref(), BAZAAR_READ_SCOPE).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    let app_state = state.get_ref().clone();
    let cache_key = products_cache_key();
    let raw = match cached_raw_response(
        app_state.clone(),
        cache_key,
        StdDuration::from_secs(15),
        move || {
            let app_state = app_state.clone();
            async move {
                match db::list_products_v2(&app_state.db).await {
                    Ok(products) => Ok(success_json_bytes(products)),
                    Err(error) => {
                        eprintln!("Products query failed: {}", error);
                        Err(RawCacheLoadError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "products_query_failed",
                            "Products could not be loaded",
                        ))
                    }
                }
            }
        },
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => return error.into_response(),
    };
    security::with_auth_headers(raw_json_response(raw), &auth)
}

#[get("/api/v2/skyblock/bazaar/products/latest")]
pub async fn latest_many(
    req: HttpRequest,
    state: web::Data<AppState>,
    query: web::Query<LatestQuery>,
) -> impl Responder {
    let auth = match security::authorize_public(&req, state.get_ref(), BAZAAR_READ_SCOPE).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    let ids = parse_ids(query.ids.as_deref());
    let app_state = state.get_ref().clone();
    let cache_key = latest_many_cache_key(&ids);
    let raw = match cached_raw_response(
        app_state.clone(),
        cache_key,
        StdDuration::from_secs(15),
        move || {
            let app_state = app_state.clone();
            let ids = ids.clone();
            async move {
                match db::get_latest_many_v2(&app_state.db, &ids).await {
                    Ok(data) => Ok(success_json_bytes(data)),
                    Err(error) => {
                        eprintln!("Latest many query failed: {}", error);
                        Err(RawCacheLoadError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "latest_query_failed",
                            "Latest data could not be loaded",
                        ))
                    }
                }
            }
        },
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => return error.into_response(),
    };
    security::with_auth_headers(raw_json_response(raw), &auth)
}

#[get("/api/v2/skyblock/bazaar/products/{product_id}/latest")]
pub async fn latest_one(
    req: HttpRequest,
    state: web::Data<AppState>,
    product_id: web::Path<String>,
) -> impl Responder {
    let auth = match security::authorize_public(&req, state.get_ref(), BAZAAR_READ_SCOPE).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    let product_id = product_id.into_inner();
    if !valid_product_id(&product_id) {
        return security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_product_id",
            "Invalid product id",
        );
    }

    let app_state = state.get_ref().clone();
    let cache_key = latest_one_cache_key(&product_id);
    let raw = match cached_raw_response(
        app_state.clone(),
        cache_key,
        StdDuration::from_secs(15),
        move || {
            let app_state = app_state.clone();
            let product_id = product_id.clone();
            async move {
                match db::get_latest_bazaar_data_v2(&app_state.db, &product_id).await {
                    Ok(Some(data)) => Ok(success_json_bytes(data)),
                    Ok(None) => Err(RawCacheLoadError::new(
                        StatusCode::NOT_FOUND,
                        "product_not_found",
                        "Product not found",
                    )),
                    Err(error) => {
                        eprintln!("Latest one query failed: {}", error);
                        Err(RawCacheLoadError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "latest_query_failed",
                            "Latest data could not be loaded",
                        ))
                    }
                }
            }
        },
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => return error.into_response(),
    };
    security::with_auth_headers(raw_json_response(raw), &auth)
}

#[get("/api/v2/skyblock/bazaar/products/{product_id}/candles")]
pub async fn candles(
    req: HttpRequest,
    state: web::Data<AppState>,
    product_id: web::Path<String>,
    query: web::Query<ChartQuery>,
) -> impl Responder {
    let auth = match security::authorize_public(&req, state.get_ref(), BAZAAR_READ_SCOPE).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    let product_id = product_id.into_inner();
    let window = match parse_chart_window(&product_id, &query) {
        Ok(window) => window,
        Err(response) => return response,
    };

    let cache_key = format!(
        "{PUBLIC_CACHE_VERSION}:candles:{}:{}:{}:{}:{}:{}",
        product_id,
        window.interval,
        window.metric,
        window.start.timestamp(),
        window.end.timestamp(),
        window.limit
    );
    let app_state = state.get_ref().clone();
    let raw = match cached_raw_response(
        app_state.clone(),
        cache_key,
        StdDuration::from_secs(30),
        move || {
            let app_state = app_state.clone();
            let product_id = product_id.clone();
            let window = window.clone();
            async move {
                match db::get_candles(
                    &app_state.db,
                    &product_id,
                    &window.interval,
                    &window.metric,
                    window.start,
                    window.end,
                    window.limit,
                )
                .await
                {
                    Ok(candle_rows) => {
                        let points = candle_rows
                            .into_iter()
                            .map(|candle| candle_point(candle, &window.metric))
                            .collect::<Vec<_>>();
                        Ok(success_json_bytes(points))
                    }
                    Err(error) => {
                        eprintln!("Candles query failed: {}", error);
                        Err(RawCacheLoadError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "candles_query_failed",
                            "Candles could not be loaded",
                        ))
                    }
                }
            }
        },
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => return error.into_response(),
    };
    security::with_auth_headers(raw_json_response(raw), &auth)
}

#[get("/api/v2/skyblock/bazaar/products/{product_id}/series")]
pub async fn series(
    req: HttpRequest,
    state: web::Data<AppState>,
    product_id: web::Path<String>,
    query: web::Query<ChartQuery>,
) -> impl Responder {
    let auth = match security::authorize_public(&req, state.get_ref(), BAZAAR_READ_SCOPE).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    let product_id = product_id.into_inner();
    let window = match parse_chart_window(&product_id, &query) {
        Ok(window) => window,
        Err(response) => return response,
    };
    let stat = query.stat.as_deref().unwrap_or("close").to_string();
    if !["open", "high", "low", "close", "avg", "volume"].contains(&stat.as_str()) {
        return security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_stat",
            "stat must be one of: open, high, low, close, avg, volume",
        );
    }

    let cache_key = format!(
        "{PUBLIC_CACHE_VERSION}:series:{}:{}:{}:{}:{}:{}:{}",
        product_id,
        window.interval,
        window.metric,
        stat,
        window.start.timestamp(),
        window.end.timestamp(),
        window.limit
    );
    let app_state = state.get_ref().clone();
    let raw = match cached_raw_response(
        app_state.clone(),
        cache_key,
        StdDuration::from_secs(30),
        move || {
            let app_state = app_state.clone();
            let product_id = product_id.clone();
            let window = window.clone();
            let stat = stat.clone();
            async move {
                match db::get_candles(
                    &app_state.db,
                    &product_id,
                    &window.interval,
                    &window.metric,
                    window.start,
                    window.end,
                    window.limit,
                )
                .await
                {
                    Ok(candle_rows) => {
                        let points = candle_rows
                            .into_iter()
                            .map(|candle| series_point(candle, &window.metric, &stat))
                            .collect::<Vec<_>>();
                        Ok(success_json_bytes(points))
                    }
                    Err(error) => {
                        eprintln!("Series query failed: {}", error);
                        Err(RawCacheLoadError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "series_query_failed",
                            "Series could not be loaded",
                        ))
                    }
                }
            }
        },
    )
    .await
    {
        Ok(raw) => raw,
        Err(error) => return error.into_response(),
    };
    security::with_auth_headers(raw_json_response(raw), &auth)
}

async fn cached_raw_response<F, Fut>(
    state: AppState,
    cache_key: String,
    ttl: StdDuration,
    load: F,
) -> Result<Bytes, RawCacheLoadError>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Bytes, RawCacheLoadError>> + Send + 'static,
{
    if let Some(hit) = state.security_store.raw_cache_get(&cache_key).await {
        if hit.is_stale() {
            spawn_raw_cache_refresh(state, cache_key, ttl, load);
        }
        return Ok(hit.value);
    }

    let lock_key = format!("cache-fill:{}", cache_key);
    let lock_token = match state
        .security_store
        .acquire_lock(&lock_key, StdDuration::from_secs(5))
        .await
    {
        Ok(token) => token,
        Err(error) => {
            eprintln!("Response cache fill lock failed: {}", error);
            None
        }
    };

    if lock_token.is_none() {
        if let Some(hit) = wait_for_raw_cached_response(&state, &cache_key).await {
            return Ok(hit.value);
        }
    }

    let result = load().await;
    if let Ok(value) = &result {
        state
            .security_store
            .raw_cache_set(cache_key.clone(), value.clone(), ttl)
            .await;
    }

    if let Some(token) = lock_token {
        if let Err(error) = state.security_store.release_lock(&lock_key, &token).await {
            eprintln!("Response cache fill lock release failed: {}", error);
        }
    }

    result
}

fn spawn_raw_cache_refresh<F, Fut>(state: AppState, cache_key: String, ttl: StdDuration, load: F)
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<Bytes, RawCacheLoadError>> + Send + 'static,
{
    tokio::spawn(async move {
        let lock_key = format!("cache-fill:{}", cache_key);
        let token = match state
            .security_store
            .acquire_lock(&lock_key, StdDuration::from_secs(5))
            .await
        {
            Ok(Some(token)) => token,
            Ok(None) => return,
            Err(error) => {
                eprintln!("Response cache refresh lock failed: {}", error);
                return;
            }
        };

        if let Some(value) = state.security_store.raw_cache_get_l2(&cache_key).await {
            state
                .security_store
                .raw_cache_set(cache_key.clone(), value, ttl)
                .await;
            let _ = state.security_store.release_lock(&lock_key, &token).await;
            return;
        }

        if let Ok(value) = load().await {
            state
                .security_store
                .raw_cache_set(cache_key.clone(), value, ttl)
                .await;
        }

        if let Err(error) = state.security_store.release_lock(&lock_key, &token).await {
            eprintln!("Response cache refresh lock release failed: {}", error);
        }
    });
}

async fn wait_for_raw_cached_response(
    state: &AppState,
    cache_key: &str,
) -> Option<crate::shared_store::RawCacheHit> {
    for _ in 0..20 {
        tokio::time::sleep(StdDuration::from_millis(25)).await;
        if let Some(hit) = state.security_store.raw_cache_get(cache_key).await {
            return Some(hit);
        }
    }
    None
}

pub fn products_cache_key() -> String {
    format!("{PUBLIC_CACHE_VERSION}:products")
}

pub fn latest_many_cache_key(ids: &[String]) -> String {
    format!("{PUBLIC_CACHE_VERSION}:latest-many:{}", ids.join(","))
}

pub fn latest_one_cache_key(product_id: &str) -> String {
    format!("{PUBLIC_CACHE_VERSION}:latest:{}", product_id)
}

pub fn success_json<T: Serialize>(data: T) -> String {
    serde_json::to_string(&ApiResponse::success(data)).unwrap_or_else(|_| "{}".to_string())
}

pub fn success_json_bytes<T: Serialize>(data: T) -> Bytes {
    Bytes::from(success_json(data).into_bytes())
}

fn raw_json_response(raw: Bytes) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header(("Content-Type", "application/json"))
        .body(raw)
}

#[derive(Debug, Clone, Copy)]
struct RawCacheLoadError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl RawCacheLoadError {
    fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    fn into_response(self) -> HttpResponse {
        security::problem(self.status, self.code, self.message)
    }
}

#[derive(Clone)]
struct ChartWindow {
    interval: String,
    metric: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    limit: u32,
}

fn parse_chart_window(product_id: &str, query: &ChartQuery) -> Result<ChartWindow, HttpResponse> {
    if !valid_product_id(product_id) {
        return Err(security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_product_id",
            "Invalid product id",
        ));
    }

    let interval = query.interval.as_deref().unwrap_or("1m");
    if db::interval_seconds(interval).is_none() {
        return Err(security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_interval",
            "interval must be one of: 15s, 1m, 5m, 15m, 1h, 1d, 1w, 1mo",
        ));
    }

    let metric = query.metric.as_deref().unwrap_or("mid_price");
    if !["buy_price", "sell_price", "mid_price", "spread"].contains(&metric) {
        return Err(security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_metric",
            "metric must be one of: buy_price, sell_price, mid_price, spread",
        ));
    }

    let end = match &query.end {
        Some(value) => parse_datetime(value)?,
        None => Utc::now(),
    };
    let start = match &query.start {
        Some(value) => parse_datetime(value)?,
        None => end - parse_range(query.range.as_deref().unwrap_or("1d"))?,
    };

    if start >= end {
        return Err(security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_time_window",
            "start must be before end",
        ));
    }

    let range = end - start;
    if range > max_range_for_interval(interval) {
        return Err(security::problem(
            StatusCode::BAD_REQUEST,
            "range_too_large",
            "Requested range is too large for the selected interval",
        ));
    }

    Ok(ChartWindow {
        interval: interval.to_string(),
        metric: metric.to_string(),
        start,
        end,
        limit: query.limit.unwrap_or(2000).clamp(1, 5000),
    })
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, HttpResponse> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| {
            security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_datetime",
                "Dates must use RFC3339/ISO 8601 format",
            )
        })
}

fn parse_range(value: &str) -> Result<Duration, HttpResponse> {
    let split_at = value.find(|ch: char| !ch.is_ascii_digit()).ok_or_else(|| {
        security::problem(StatusCode::BAD_REQUEST, "invalid_range", "Invalid range")
    })?;
    let (amount, unit) = value.split_at(split_at);
    let amount = amount.parse::<i64>().map_err(|_| {
        security::problem(StatusCode::BAD_REQUEST, "invalid_range", "Invalid range")
    })?;
    if amount <= 0 {
        return Err(security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_range",
            "Range amount must be positive",
        ));
    }
    let duration = match unit {
        "s" => Duration::seconds(amount),
        "m" => Duration::minutes(amount),
        "h" => Duration::hours(amount),
        "d" => Duration::days(amount),
        "w" => Duration::weeks(amount),
        "mo" => Duration::days(amount.checked_mul(30).ok_or_else(|| {
            security::problem(StatusCode::BAD_REQUEST, "invalid_range", "Invalid range")
        })?),
        "y" => Duration::days(amount.checked_mul(365).ok_or_else(|| {
            security::problem(StatusCode::BAD_REQUEST, "invalid_range", "Invalid range")
        })?),
        _ => {
            return Err(security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_range",
                "Invalid range unit",
            ));
        }
    };
    Ok(duration)
}

fn max_range_for_interval(interval: &str) -> Duration {
    match interval {
        "15s" => Duration::hours(24),
        "1m" => Duration::days(30),
        "5m" | "15m" => Duration::days(180),
        "1h" => Duration::days(365 * 5),
        "1d" | "1w" | "1mo" => Duration::days(365 * 100),
        _ => Duration::days(1),
    }
}

fn candle_point(candle: BazaarCandle, metric: &str) -> CandlePoint {
    let metric = candle_metric(&candle, metric);
    CandlePoint {
        t: candle.period_start,
        period_end: candle.period_end,
        open: metric.open,
        high: metric.high,
        low: metric.low,
        close: metric.close,
        volume: candle.volume,
        samples: metric.sample_count,
    }
}

fn series_point(candle: BazaarCandle, metric: &str, stat: &str) -> SeriesPoint {
    let metric = candle_metric(&candle, metric);
    let value = match stat {
        "open" => metric.open,
        "high" => metric.high,
        "low" => metric.low,
        "avg" => {
            if metric.sample_count > 0 {
                metric.value_sum / metric.sample_count as f64
            } else {
                metric.close
            }
        }
        "volume" => candle.volume as f64,
        _ => metric.close,
    };

    SeriesPoint {
        t: candle.period_start,
        value,
        samples: metric.sample_count,
    }
}

fn candle_metric<'a>(candle: &'a BazaarCandle, metric: &str) -> &'a CandleMetric {
    match metric {
        "buy_price" => &candle.buy_price,
        "sell_price" => &candle.sell_price,
        "spread" => &candle.spread,
        _ => &candle.mid_price,
    }
}

fn parse_ids(ids: Option<&str>) -> Vec<String> {
    ids.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty() && valid_product_id(id))
        .take(200)
        .map(ToOwned::to_owned)
        .collect()
}

fn valid_product_id(product_id: &str) -> bool {
    !product_id.is_empty()
        && product_id.len() <= 128
        && product_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':'))
}

#[cfg(test)]
mod tests {
    use super::{parse_range, products_cache_key, success_json, valid_product_id};

    #[test]
    fn product_id_validation_is_strict_but_hypixel_friendly() {
        assert!(valid_product_id("ENCHANTED_WHEAT"));
        assert!(valid_product_id("INK_SACK:3"));
        assert!(!valid_product_id("../secret"));
    }

    #[test]
    fn range_parser_accepts_chart_units() {
        assert_eq!(parse_range("1d").unwrap().num_hours(), 24);
        assert_eq!(parse_range("2w").unwrap().num_days(), 14);
        assert!(parse_range("bad").is_err());
    }

    #[test]
    fn raw_success_json_preserves_api_shape() {
        let raw = success_json(vec!["WHEAT".to_string()]);
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["data"][0], "WHEAT");
        assert_eq!(products_cache_key(), "v3:products");
    }
}
