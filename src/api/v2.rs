use crate::db;
use crate::models::{ApiResponse, BazaarCandle, CandlePoint, ChartQuery, LatestQuery, SeriesPoint};
use crate::security;
use crate::state::AppState;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use chrono::{DateTime, Duration, Utc};
use mongodb::bson::doc;
use serde_json::json;
use std::time::Duration as StdDuration;

const BAZAAR_READ_SCOPE: &str = "bazaar:read";

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
    match state.db.run_command(doc! { "ping": 1 }).await {
        Ok(_) => HttpResponse::Ok().json(json!({
            "success": true,
            "data": {
                "mongo": "ok",
                "redis": if state.config.redis_url.is_some() { "configured" } else { "not_configured" }
            },
            "error": null,
            "timestamp": Utc::now()
        })),
        Err(e) => security::problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "readiness_failed",
            &format!("MongoDB readiness check failed: {}", e),
        ),
    }
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
            "/api/v2/skyblock/bazaar/products/{product_id}/series": { "get": { "summary": "Get chart series" } }
        }
    }))
}

#[get("/api/v2/skyblock/bazaar/products")]
pub async fn list_products(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    let auth = match security::authorize_public(&req, state.get_ref(), BAZAAR_READ_SCOPE).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    let cache_key = "v2:products".to_string();
    if let Some(value) = state.cache.get(&cache_key).await {
        return security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit);
    }

    match db::list_products_v2(&state.db).await {
        Ok(products) => {
            let value =
                serde_json::to_value(ApiResponse::success(products)).unwrap_or_else(|_| json!({}));
            state
                .cache
                .set(cache_key, value.clone(), StdDuration::from_secs(15))
                .await;
            security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit)
        }
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "products_query_failed",
            &format!("Products could not be loaded: {}", e),
        ),
    }
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
    let cache_key = format!("v2:latest-many:{}", ids.join(","));
    if let Some(value) = state.cache.get(&cache_key).await {
        return security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit);
    }

    match db::get_latest_many_v2(&state.db, &ids).await {
        Ok(data) => {
            let value =
                serde_json::to_value(ApiResponse::success(data)).unwrap_or_else(|_| json!({}));
            state
                .cache
                .set(cache_key, value.clone(), StdDuration::from_secs(15))
                .await;
            security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit)
        }
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "latest_query_failed",
            &format!("Latest data could not be loaded: {}", e),
        ),
    }
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

    let cache_key = format!("v2:latest:{}", product_id);
    if let Some(value) = state.cache.get(&cache_key).await {
        return security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit);
    }

    match db::get_latest_bazaar_data_v2(&state.db, &product_id).await {
        Ok(Some(data)) => {
            let value =
                serde_json::to_value(ApiResponse::success(data)).unwrap_or_else(|_| json!({}));
            state
                .cache
                .set(cache_key, value.clone(), StdDuration::from_secs(15))
                .await;
            security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit)
        }
        Ok(None) => security::problem(
            StatusCode::NOT_FOUND,
            "product_not_found",
            "Product not found",
        ),
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "latest_query_failed",
            &format!("Latest data could not be loaded: {}", e),
        ),
    }
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
        "v2:candles:{}:{}:{}:{}:{}:{}",
        product_id,
        window.interval,
        window.metric,
        window.start.timestamp(),
        window.end.timestamp(),
        window.limit
    );
    if let Some(value) = state.cache.get(&cache_key).await {
        return security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit);
    }

    match db::get_candles(
        &state.db,
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
                .map(candle_point)
                .collect::<Vec<_>>();
            let value =
                serde_json::to_value(ApiResponse::success(points)).unwrap_or_else(|_| json!({}));
            state
                .cache
                .set(cache_key, value.clone(), StdDuration::from_secs(30))
                .await;
            security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit)
        }
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "candles_query_failed",
            &format!("Candles could not be loaded: {}", e),
        ),
    }
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
    let stat = query.stat.as_deref().unwrap_or("close");
    if !["open", "high", "low", "close", "avg", "volume"].contains(&stat) {
        return security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_stat",
            "stat must be one of: open, high, low, close, avg, volume",
        );
    }

    let cache_key = format!(
        "v2:series:{}:{}:{}:{}:{}:{}:{}",
        product_id,
        window.interval,
        window.metric,
        stat,
        window.start.timestamp(),
        window.end.timestamp(),
        window.limit
    );
    if let Some(value) = state.cache.get(&cache_key).await {
        return security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit);
    }

    match db::get_candles(
        &state.db,
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
                .map(|candle| series_point(candle, stat))
                .collect::<Vec<_>>();
            let value =
                serde_json::to_value(ApiResponse::success(points)).unwrap_or_else(|_| json!({}));
            state
                .cache
                .set(cache_key, value.clone(), StdDuration::from_secs(30))
                .await;
            security::with_rate_limit_headers(HttpResponse::Ok().json(value), &auth.rate_limit)
        }
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "series_query_failed",
            &format!("Series could not be loaded: {}", e),
        ),
    }
}

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
    let duration = match unit {
        "s" => Duration::seconds(amount),
        "m" => Duration::minutes(amount),
        "h" => Duration::hours(amount),
        "d" => Duration::days(amount),
        "w" => Duration::weeks(amount),
        "mo" => Duration::days(amount * 30),
        "y" => Duration::days(amount * 365),
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

fn candle_point(candle: BazaarCandle) -> CandlePoint {
    CandlePoint {
        t: candle.period_start,
        period_end: candle.period_end,
        open: candle.open,
        high: candle.high,
        low: candle.low,
        close: candle.close,
        volume: candle.volume,
        samples: candle.sample_count,
    }
}

fn series_point(candle: BazaarCandle, stat: &str) -> SeriesPoint {
    let value = match stat {
        "open" => candle.open,
        "high" => candle.high,
        "low" => candle.low,
        "avg" => {
            if candle.sample_count > 0 {
                candle.value_sum / candle.sample_count as f64
            } else {
                candle.close
            }
        }
        "volume" => candle.volume as f64,
        _ => candle.close,
    };

    SeriesPoint {
        t: candle.period_start,
        value,
        samples: candle.sample_count,
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
    use super::{parse_range, valid_product_id};

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
}
