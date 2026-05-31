use crate::models::{
    AccessPolicy, ApiKeyRecord, BazaarAggregatedData, BazaarCandle, BazaarData, BazaarLatest,
    CandleMetric, CompressionLog, CompressionState, PaginationInfo,
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::bson::{Bson, DateTime as BsonDateTime, Document, doc, oid::ObjectId};
use mongodb::options::{IndexOptions, UpdateModifications, UpdateOneModel, WriteModel};
use std::collections::HashMap;
use std::time::Duration as StdDuration;

pub const BULK_WRITE_CHUNK_SIZE: usize = 750;

// Database initialization ve indexing
pub async fn ensure_indexes(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    require_mongodb_8(db).await?;

    // Raw data collection indexes
    let collection = db.collection::<BazaarData>("bazaar");

    // Compound index for product_id and timestamp (en önemli sorgu)
    let index_model = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1, "timestamp": -1 })
        .options(
            IndexOptions::builder()
                .name("bazaar_product_timestamp".to_string())
                .build(),
        )
        .build();

    let raw_ttl_index = mongodb::IndexModel::builder()
        .keys(doc! { "expires_at": 1 })
        .options(
            IndexOptions::builder()
                .name("bazaar_expires_at_ttl".to_string())
                .expire_after(StdDuration::from_secs(0))
                .build(),
        )
        .build();

    collection
        .create_indexes(vec![index_model, raw_ttl_index])
        .await?;

    // Latest snapshot collection indexes
    let latest_collection = db.collection::<BazaarLatest>("bazaar_latest");
    let latest_product_index = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1 })
        .options(
            IndexOptions::builder()
                .name("bazaar_latest_product_unique".to_string())
                .unique(true)
                .build(),
        )
        .build();
    latest_collection.create_index(latest_product_index).await?;

    // Materialized chart candle indexes
    let candle_collection = db.collection::<BazaarCandle>("bazaar_candles");
    let candle_unique_index = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1, "interval": 1, "period_start": 1 })
        .options(
            IndexOptions::builder()
                .name("bazaar_candles_product_interval_period_unique".to_string())
                .unique(true)
                .build(),
        )
        .build();
    let candle_rollup_index = mongodb::IndexModel::builder()
        .keys(doc! { "interval": 1, "period_start": 1 })
        .options(
            IndexOptions::builder()
                .name("bazaar_candles_interval_period".to_string())
                .build(),
        )
        .build();
    let candle_ttl_index = mongodb::IndexModel::builder()
        .keys(doc! { "expires_at": 1 })
        .options(
            IndexOptions::builder()
                .name("bazaar_candles_expires_at_ttl".to_string())
                .expire_after(StdDuration::from_secs(0))
                .build(),
        )
        .build();
    candle_collection
        .create_indexes(vec![
            candle_unique_index,
            candle_rollup_index,
            candle_ttl_index,
        ])
        .await?;

    // Compression tracking collection indexes
    let compression_state_collection = db.collection::<CompressionState>("compression_state");
    let cs_product_index = mongodb::IndexModel::builder()
        .keys(doc! { "source_interval": 1, "target_interval": 1 })
        .options(
            IndexOptions::builder()
                .name("compression_state_rollup_unique".to_string())
                .unique(true)
                .build(),
        )
        .build();
    let cs_updated_index = mongodb::IndexModel::builder()
        .keys(doc! { "updated_at": -1 })
        .options(
            IndexOptions::builder()
                .name("compression_state_updated_at".to_string())
                .build(),
        )
        .build();
    compression_state_collection
        .create_indexes(vec![cs_product_index, cs_updated_index])
        .await?;

    // Compression log collection indexes
    let compression_log_collection = db.collection::<CompressionLog>("compression_log");
    let cl_type_index = mongodb::IndexModel::builder()
        .keys(doc! { "compression_type": 1, "created_at": -1 })
        .options(
            IndexOptions::builder()
                .name("compression_log_type_created_at".to_string())
                .build(),
        )
        .build();
    compression_log_collection
        .create_index(cl_type_index)
        .await?;

    // API key and access policy indexes
    let api_key_collection = db.collection::<ApiKeyRecord>("api_keys");
    let api_key_prefix_index = mongodb::IndexModel::builder()
        .keys(doc! { "key_prefix": 1 })
        .options(
            IndexOptions::builder()
                .name("api_keys_prefix_unique".to_string())
                .unique(true)
                .build(),
        )
        .build();
    let api_key_status_index = mongodb::IndexModel::builder()
        .keys(doc! { "status": 1, "created_at": -1 })
        .options(
            IndexOptions::builder()
                .name("api_keys_status_created_at".to_string())
                .build(),
        )
        .build();
    api_key_collection
        .create_indexes(vec![api_key_prefix_index, api_key_status_index])
        .await?;

    let access_policy_collection = db.collection::<AccessPolicy>("api_settings");
    let access_policy_index = mongodb::IndexModel::builder()
        .keys(doc! { "_id": 1 })
        .options(
            IndexOptions::builder()
                .name("api_settings_id".to_string())
                .build(),
        )
        .build();
    access_policy_collection
        .create_index(access_policy_index)
        .await?;

    println!(
        "✅ Database indexes created successfully for raw, aggregated, and compression tracking data!"
    );
    Ok(())
}

async fn require_mongodb_8(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let build_info = db.run_command(doc! { "buildInfo": 1 }).await?;
    let version = build_info.get_str("version").unwrap_or("0");
    let major = version
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if major < 8 {
        return Err(format!("MongoDB 8.0+ is required for bulkWrite; detected {version}").into());
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn insert_bazaar_data(
    db: &Database,
    data: BazaarData,
) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar");
    collection.insert_one(data).await?;
    Ok(())
}

pub async fn insert_bazaar_data_batch(
    db: &Database,
    data: &[BazaarData],
) -> Result<(), Box<dyn std::error::Error>> {
    if data.is_empty() {
        return Ok(());
    }

    let collection = db.collection::<BazaarData>("bazaar");
    for chunk in data.chunks(BULK_WRITE_CHUNK_SIZE) {
        collection.insert_many(chunk).ordered(false).await?;
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn upsert_latest_bazaar_data(
    db: &Database,
    data: &BazaarData,
) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarLatest>("bazaar_latest");
    let now = Utc::now();
    collection
        .update_one(
            doc! { "product_id": &data.product_id },
            doc! {
                "$set": {
                    "product_id": &data.product_id,
                    "buy_price": data.buy_price,
                    "sell_price": data.sell_price,
                    "buy_volume": data.buy_volume,
                    "sell_volume": data.sell_volume,
                    "buy_orders": data.buy_orders,
                    "sell_orders": data.sell_orders,
                    "timestamp": data.timestamp.timestamp_millis(),
                    "updated_at": bson_datetime(now),
                }
            },
        )
        .upsert(true)
        .await?;
    Ok(())
}

pub async fn upsert_latest_bazaar_data_batch(
    db: &Database,
    data: &[BazaarData],
) -> Result<(), Box<dyn std::error::Error>> {
    if data.is_empty() {
        return Ok(());
    }

    let collection = db.collection::<BazaarLatest>("bazaar_latest");
    let now = Utc::now();
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        models.push(
            UpdateOneModel::builder()
                .namespace(collection.namespace())
                .filter(doc! { "product_id": &item.product_id })
                .update(UpdateModifications::from(doc! {
                    "$set": {
                        "product_id": &item.product_id,
                        "buy_price": item.buy_price,
                        "sell_price": item.sell_price,
                        "buy_volume": item.buy_volume,
                        "sell_volume": item.sell_volume,
                        "buy_orders": item.buy_orders,
                        "sell_orders": item.sell_orders,
                        "timestamp": item.timestamp.timestamp_millis(),
                        "updated_at": bson_datetime(now),
                    }
                }))
                .upsert(true)
                .build()
                .into(),
        );
    }
    execute_bulk_write(db, models).await
}

pub async fn get_latest_bazaar_data_v2(
    db: &Database,
    product_id: &str,
) -> Result<Option<BazaarLatest>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarLatest>("bazaar_latest");
    if let Some(latest) = collection
        .find_one(doc! { "product_id": product_id })
        .await?
    {
        return Ok(Some(latest));
    }

    Ok(get_latest_bazaar_data(db, product_id)
        .await?
        .map(BazaarLatest::from))
}

pub async fn list_products_v2(db: &Database) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let latest_collection = db.collection::<BazaarLatest>("bazaar_latest");
    let mut latest_cursor = latest_collection
        .find(doc! {})
        .sort(doc! { "product_id": 1 })
        .await?;
    let mut products = Vec::new();
    while let Some(item) = latest_cursor.try_next().await? {
        products.push(item.product_id);
    }

    if !products.is_empty() {
        return Ok(products);
    }

    let collection = db.collection::<Document>("bazaar");
    let mut cursor = collection
        .aggregate(vec![
            doc! { "$group": { "_id": "$product_id" } },
            doc! { "$sort": { "_id": 1 } },
        ])
        .await?;
    while let Some(doc) = cursor.try_next().await? {
        if let Ok(product_id) = doc.get_str("_id") {
            products.push(product_id.to_string());
        }
    }
    Ok(products)
}

pub async fn get_latest_many_v2(
    db: &Database,
    ids: &[String],
) -> Result<Vec<BazaarLatest>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarLatest>("bazaar_latest");
    let filter = if ids.is_empty() {
        doc! {}
    } else {
        doc! { "product_id": { "$in": ids } }
    };
    let mut cursor = collection
        .find(filter)
        .sort(doc! { "product_id": 1 })
        .limit(if ids.is_empty() {
            1000
        } else {
            ids.len() as i64
        })
        .await?;
    let mut results = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        results.push(item);
    }
    Ok(results)
}

#[allow(dead_code)]
pub async fn upsert_chart_candles(
    db: &Database,
    data: &BazaarData,
) -> Result<(), Box<dyn std::error::Error>> {
    upsert_chart_candles_batch(db, std::slice::from_ref(data)).await
}

pub async fn upsert_chart_candles_batch(
    db: &Database,
    data: &[BazaarData],
) -> Result<(), Box<dyn std::error::Error>> {
    if data.is_empty() {
        return Ok(());
    }

    let collection = db.collection::<BazaarCandle>("bazaar_candles");
    let now = Utc::now();
    let mut models = Vec::with_capacity(data.len());
    for item in data {
        let Some((period_start, period_end)) = candle_period(item.timestamp, "15s") else {
            continue;
        };
        models.push(candle_update_model(
            &collection,
            item,
            "15s",
            period_start,
            period_end,
            now,
        ));
    }
    execute_bulk_write(db, models).await
}

fn candle_update_model(
    collection: &mongodb::Collection<BazaarCandle>,
    data: &BazaarData,
    interval: &str,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    now: DateTime<Utc>,
) -> WriteModel {
    let buy_price = data.buy_price;
    let sell_price = data.sell_price;
    let mid_price = (data.buy_price + data.sell_price) / 2.0;
    let spread = (data.buy_price - data.sell_price).abs();
    let volume = data.buy_volume.saturating_add(data.sell_volume);
    let expires_at = retention_expires_at(interval, period_end);
    let mut set_doc = doc! {
        "buy_price.close": buy_price,
        "sell_price.close": sell_price,
        "mid_price.close": mid_price,
        "spread.close": spread,
        "updated_at": bson_datetime(now),
    };
    if let Some(expires_at) = expires_at {
        set_doc.insert("expires_at", bson_datetime(expires_at));
    }

    UpdateOneModel::builder()
        .namespace(collection.namespace())
        .filter(doc! {
            "product_id": &data.product_id,
            "interval": interval,
            "period_start": bson_datetime(period_start),
        })
        .update(UpdateModifications::from(doc! {
            "$setOnInsert": {
                "product_id": &data.product_id,
                "interval": interval,
                "period_start": bson_datetime(period_start),
                "period_end": bson_datetime(period_end),
                "buy_price.open": buy_price,
                "sell_price.open": sell_price,
                "mid_price.open": mid_price,
                "spread.open": spread,
            },
            "$max": {
                "buy_price.high": buy_price,
                "sell_price.high": sell_price,
                "mid_price.high": mid_price,
                "spread.high": spread,
            },
            "$min": {
                "buy_price.low": buy_price,
                "sell_price.low": sell_price,
                "mid_price.low": mid_price,
                "spread.low": spread,
            },
            "$set": set_doc,
            "$inc": {
                "buy_price.value_sum": buy_price,
                "sell_price.value_sum": sell_price,
                "mid_price.value_sum": mid_price,
                "spread.value_sum": spread,
                "buy_price.sample_count": 1_i64,
                "sell_price.sample_count": 1_i64,
                "mid_price.sample_count": 1_i64,
                "spread.sample_count": 1_i64,
                "volume": volume,
                "buy_volume_sum": data.buy_volume,
                "sell_volume_sum": data.sell_volume,
            }
        }))
        .upsert(true)
        .build()
        .into()
}

async fn execute_bulk_write(
    db: &Database,
    models: Vec<WriteModel>,
) -> Result<(), Box<dyn std::error::Error>> {
    if models.is_empty() {
        return Ok(());
    }

    let client = db.client().clone();
    for chunk in models.chunks(BULK_WRITE_CHUNK_SIZE) {
        client.bulk_write(chunk.to_vec()).ordered(false).await?;
    }
    Ok(())
}

pub async fn get_candles(
    db: &Database,
    product_id: &str,
    interval: &str,
    _metric: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<BazaarCandle>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarCandle>("bazaar_candles");
    let mut cursor = collection
        .find(doc! {
            "product_id": product_id,
            "interval": interval,
            "period_start": {
                "$gte": bson_datetime(start),
                "$lte": bson_datetime(end),
            }
        })
        .sort(doc! { "period_start": 1 })
        .limit(limit.min(5000) as i64)
        .await?;
    let mut candles = Vec::new();
    while let Some(candle) = cursor.try_next().await? {
        candles.push(candle);
    }
    Ok(candles)
}

pub async fn rollup_closed_candles(db: &Database) -> Result<usize, Box<dyn std::error::Error>> {
    let specs = [
        ("15s", "1m"),
        ("1m", "5m"),
        ("5m", "15m"),
        ("15m", "1h"),
        ("1h", "1d"),
        ("1d", "1w"),
        ("1d", "1mo"),
    ];
    let mut total_written = 0usize;
    for (source_interval, target_interval) in specs {
        total_written += rollup_spec(db, source_interval, target_interval).await?;
    }
    Ok(total_written)
}

async fn rollup_spec(
    db: &Database,
    source_interval: &str,
    target_interval: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let Some(target_seconds) = interval_seconds(target_interval) else {
        return Ok(0);
    };
    let Some(current_target_start) = candle_period(Utc::now(), target_interval).map(|(s, _)| s)
    else {
        return Ok(0);
    };

    let state = get_rollup_state(db, source_interval, target_interval).await?;
    let mut next_start =
        if let Some(last_processed) = state.and_then(|state| state.last_processed_period_start) {
            last_processed + Duration::seconds(target_seconds)
        } else if let Some(oldest) = oldest_source_period(db, source_interval).await? {
            candle_period(oldest, target_interval)
                .map(|(start, _)| start)
                .unwrap_or(oldest)
        } else {
            return Ok(0);
        };

    let started_at = std::time::Instant::now();
    let mut first_processed = None;
    let mut last_processed = None;
    let mut source_records_count = 0_i64;
    let mut compressed_records_count = 0_i64;
    let mut written = 0usize;
    let mut rollup_buffer = Vec::new();

    for _ in 0..240 {
        if next_start >= current_target_start {
            break;
        }
        let period_end = next_start + Duration::seconds(target_seconds);
        let source_rows = load_source_candles(db, source_interval, next_start, period_end).await?;
        source_records_count += source_rows.len() as i64;
        let rollups = build_rollup_candles(&source_rows, target_interval, next_start, period_end);
        compressed_records_count += rollups.len() as i64;
        written += rollups.len();
        rollup_buffer.extend(rollups);
        if rollup_buffer.len() >= BULK_WRITE_CHUNK_SIZE {
            upsert_rollup_candles(db, &rollup_buffer).await?;
            rollup_buffer.clear();
        }
        first_processed.get_or_insert(next_start);
        last_processed = Some(next_start);
        next_start = period_end;
    }

    if let (Some(first), Some(last)) = (first_processed, last_processed) {
        if !rollup_buffer.is_empty() {
            upsert_rollup_candles(db, &rollup_buffer).await?;
        }
        upsert_rollup_state(db, source_interval, target_interval, Some(last), false).await?;

        let log = CompressionLog {
            id: None,
            product_id: "*".to_string(),
            compression_type: format!("{source_interval}_to_{target_interval}"),
            source_period_start: first,
            source_period_end: last + Duration::seconds(target_seconds),
            compressed_period_start: first,
            compressed_period_end: last + Duration::seconds(target_seconds),
            source_records_count,
            compressed_records_count,
            bytes_saved: None,
            compression_duration_ms: started_at.elapsed().as_millis() as i64,
            status: "success".to_string(),
            error_message: None,
            created_at: Utc::now(),
        };
        log_compression(db, &log).await?;
    }

    Ok(written)
}

async fn oldest_source_period(
    db: &Database,
    source_interval: &str,
) -> Result<Option<DateTime<Utc>>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarCandle>("bazaar_candles");
    Ok(collection
        .find_one(doc! { "interval": source_interval })
        .sort(doc! { "period_start": 1 })
        .await?
        .map(|candle| candle.period_start))
}

async fn load_source_candles(
    db: &Database,
    source_interval: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<BazaarCandle>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarCandle>("bazaar_candles");
    let mut cursor = collection
        .find(doc! {
            "interval": source_interval,
            "period_start": {
                "$gte": bson_datetime(start),
                "$lt": bson_datetime(end),
            }
        })
        .sort(doc! { "period_start": 1, "product_id": 1 })
        .await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    Ok(rows)
}

fn build_rollup_candles(
    source_rows: &[BazaarCandle],
    target_interval: &str,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Vec<BazaarCandle> {
    let mut by_product: HashMap<String, BazaarCandle> = HashMap::new();
    let now = Utc::now();
    for row in source_rows {
        by_product
            .entry(row.product_id.clone())
            .and_modify(|target| merge_candle(target, row))
            .or_insert_with(|| BazaarCandle {
                product_id: row.product_id.clone(),
                interval: target_interval.to_string(),
                period_start,
                period_end,
                buy_price: row.buy_price.clone(),
                sell_price: row.sell_price.clone(),
                mid_price: row.mid_price.clone(),
                spread: row.spread.clone(),
                volume: row.volume,
                buy_volume_sum: row.buy_volume_sum,
                sell_volume_sum: row.sell_volume_sum,
                updated_at: now,
                expires_at: retention_expires_at(target_interval, period_end),
            });
    }
    by_product.into_values().collect()
}

fn merge_candle(target: &mut BazaarCandle, source: &BazaarCandle) {
    merge_metric(&mut target.buy_price, &source.buy_price);
    merge_metric(&mut target.sell_price, &source.sell_price);
    merge_metric(&mut target.mid_price, &source.mid_price);
    merge_metric(&mut target.spread, &source.spread);
    target.volume = target.volume.saturating_add(source.volume);
    target.buy_volume_sum = target.buy_volume_sum.saturating_add(source.buy_volume_sum);
    target.sell_volume_sum = target
        .sell_volume_sum
        .saturating_add(source.sell_volume_sum);
    target.updated_at = Utc::now();
}

fn merge_metric(target: &mut CandleMetric, source: &CandleMetric) {
    target.high = target.high.max(source.high);
    target.low = target.low.min(source.low);
    target.close = source.close;
    target.value_sum += source.value_sum;
    target.sample_count += source.sample_count;
}

async fn upsert_rollup_candles(
    db: &Database,
    candles: &[BazaarCandle],
) -> Result<(), Box<dyn std::error::Error>> {
    if candles.is_empty() {
        return Ok(());
    }

    let collection = db.collection::<BazaarCandle>("bazaar_candles");
    let mut models = Vec::with_capacity(candles.len());
    for candle in candles {
        let set_doc = mongodb::bson::to_document(candle)?;
        let update = if candle.expires_at.is_some() {
            doc! { "$set": set_doc }
        } else {
            doc! {
                "$set": set_doc,
                "$unset": { "expires_at": "" },
            }
        };
        models.push(
            UpdateOneModel::builder()
                .namespace(collection.namespace())
                .filter(doc! {
                    "product_id": &candle.product_id,
                    "interval": &candle.interval,
                    "period_start": bson_datetime(candle.period_start),
                })
                .update(UpdateModifications::from(update))
                .upsert(true)
                .build()
                .into(),
        );
    }
    execute_bulk_write(db, models).await
}

#[allow(dead_code)]
pub async fn cleanup_retention(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let _ = db;
    Ok(())
}

pub async fn get_access_policy(db: &Database) -> Result<AccessPolicy, Box<dyn std::error::Error>> {
    let collection = db.collection::<AccessPolicy>("api_settings");
    if let Some(policy) = collection.find_one(doc! { "_id": "public_api" }).await? {
        return Ok(policy);
    }

    let policy = AccessPolicy::default();
    collection
        .update_one(
            doc! { "_id": "public_api" },
            doc! {
                "$setOnInsert": {
                    "anonymous_public_enabled": policy.anonymous_public_enabled,
                    "anonymous_rate_limit_per_minute": policy.anonymous_rate_limit_per_minute,
                    "default_user_rate_limit_per_minute": policy.default_user_rate_limit_per_minute,
                    "updated_at": bson_datetime(policy.updated_at),
                }
            },
        )
        .upsert(true)
        .await?;
    Ok(policy)
}

pub async fn update_access_policy(
    db: &Database,
    anonymous_public_enabled: Option<bool>,
    anonymous_rate_limit_per_minute: Option<u32>,
    default_user_rate_limit_per_minute: Option<u32>,
) -> Result<AccessPolicy, Box<dyn std::error::Error>> {
    let mut set_doc = doc! { "updated_at": bson_datetime(Utc::now()) };
    if let Some(value) = anonymous_public_enabled {
        set_doc.insert("anonymous_public_enabled", value);
    }
    if let Some(value) = anonymous_rate_limit_per_minute {
        set_doc.insert("anonymous_rate_limit_per_minute", value.max(1));
    }
    if let Some(value) = default_user_rate_limit_per_minute {
        set_doc.insert("default_user_rate_limit_per_minute", value.max(1));
    }

    let collection = db.collection::<AccessPolicy>("api_settings");
    collection
        .update_one(doc! { "_id": "public_api" }, doc! { "$set": set_doc })
        .upsert(true)
        .await?;
    get_access_policy(db).await
}

pub async fn insert_api_key(
    db: &Database,
    mut record: ApiKeyRecord,
) -> Result<ApiKeyRecord, Box<dyn std::error::Error>> {
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    let id = ObjectId::new();
    record.id = Some(id);
    collection.insert_one(record.clone()).await?;
    Ok(record)
}

pub async fn list_api_keys(db: &Database) -> Result<Vec<ApiKeyRecord>, Box<dyn std::error::Error>> {
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    let mut cursor = collection
        .find(doc! {})
        .sort(doc! { "created_at": -1 })
        .await?;
    let mut records = Vec::new();
    while let Some(record) = cursor.try_next().await? {
        records.push(record);
    }
    Ok(records)
}

pub async fn find_api_key_by_prefix(
    db: &Database,
    prefix: &str,
) -> Result<Option<ApiKeyRecord>, Box<dyn std::error::Error>> {
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    Ok(collection.find_one(doc! { "key_prefix": prefix }).await?)
}

pub async fn find_api_key_by_id(
    db: &Database,
    id: &str,
) -> Result<Option<ApiKeyRecord>, Box<dyn std::error::Error>> {
    let object_id = ObjectId::parse_str(id)?;
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    Ok(collection.find_one(doc! { "_id": object_id }).await?)
}

pub async fn touch_api_key_last_used(
    db: &Database,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    if let Ok(object_id) = ObjectId::parse_str(id) {
        collection
            .update_one(
                doc! { "_id": object_id },
                doc! { "$set": { "last_used_at": bson_datetime(Utc::now()) } },
            )
            .await?;
    }
    Ok(())
}

pub async fn update_api_key_hash(
    db: &Database,
    id: &str,
    key_hash: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    let object_id = ObjectId::parse_str(id)?;
    collection
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "key_hash": key_hash,
                    "updated_at": bson_datetime(Utc::now()),
                }
            },
        )
        .await?;
    Ok(())
}

pub async fn update_api_key(
    db: &Database,
    id: &str,
    update: Document,
) -> Result<Option<ApiKeyRecord>, Box<dyn std::error::Error>> {
    let object_id = ObjectId::parse_str(id)?;
    let collection = db.collection::<ApiKeyRecord>("api_keys");
    collection
        .update_one(doc! { "_id": object_id }, doc! { "$set": update })
        .await?;
    Ok(collection.find_one(doc! { "_id": object_id }).await?)
}

pub async fn revoke_api_key(
    db: &Database,
    id: &str,
) -> Result<Option<ApiKeyRecord>, Box<dyn std::error::Error>> {
    let update = doc! {
        "status": "revoked",
        "updated_at": bson_datetime(Utc::now()),
    };
    update_api_key(db, id, update).await
}

pub async fn rotate_api_key(
    db: &Database,
    id: &str,
    key_prefix: String,
    key_hash: String,
) -> Result<Option<ApiKeyRecord>, Box<dyn std::error::Error>> {
    update_api_key(
        db,
        id,
        doc! {
            "key_prefix": key_prefix,
            "key_hash": key_hash,
            "status": "active",
            "updated_at": bson_datetime(Utc::now()),
        },
    )
    .await
}

pub async fn get_latest_bazaar_data(
    db: &Database,
    product_id: &str,
) -> Result<Option<BazaarData>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar");
    let filter = doc! { "product_id": product_id };

    let mut cursor = collection
        .find(filter)
        .sort(doc! { "timestamp": -1 })
        .limit(1)
        .await?;
    if let Some(data) = cursor.try_next().await? {
        Ok(Some(data))
    } else {
        Ok(None)
    }
}

// Pagination desteği ile veri getirme
#[allow(dead_code)]
pub async fn get_bazaar_data_paginated(
    db: &Database,
    product_id: &str,
    page: u32,
    limit: u32,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    min_price: Option<f64>,
    max_price: Option<f64>,
    min_volume: Option<i64>,
    max_volume: Option<i64>,
    sort_by: Option<String>,
    sort_order: Option<String>,
) -> Result<(Vec<BazaarData>, PaginationInfo), Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar");

    // Filter oluştur
    let mut filter = doc! { "product_id": product_id };

    // Tarih filtresi
    if let (Some(start), Some(end)) = (start_time, end_time) {
        filter.insert(
            "timestamp",
            doc! {
                "$gte": start.timestamp_millis(),
                "$lte": end.timestamp_millis()
            },
        );
    }

    // Fiyat ve hacim filtreleri
    let mut price_filter = Document::new();
    if let Some(min_p) = min_price {
        price_filter.insert("$gte", min_p);
    }
    if let Some(max_p) = max_price {
        price_filter.insert("$lte", max_p);
    }
    if !price_filter.is_empty() {
        filter.insert("buy_price", price_filter);
    }

    let mut volume_filter = Document::new();
    if let Some(min_v) = min_volume {
        volume_filter.insert("$gte", min_v);
    }
    if let Some(max_v) = max_volume {
        volume_filter.insert("$lte", max_v);
    }
    if !volume_filter.is_empty() {
        filter.insert("buy_volume", volume_filter);
    }

    // Toplam kayıt sayısını al
    let total_items = collection.count_documents(filter.clone()).await?;

    // Sort kriterini belirle
    let sort_field = match sort_by.as_deref().unwrap_or("timestamp") {
        "timestamp" => "timestamp",
        "price" | "buy_price" => "buy_price",
        "sell_price" => "sell_price",
        "volume" | "buy_volume" => "buy_volume",
        "sell_volume" => "sell_volume",
        _ => "timestamp",
    };
    let sort_direction = if sort_order.unwrap_or_else(|| "desc".to_string()) == "asc" {
        1
    } else {
        -1
    };

    // Pagination için skip ve limit hesapla
    let skip = (page.saturating_sub(1)) * limit;

    let mut cursor = collection
        .find(filter)
        .sort(doc! { sort_field: sort_direction })
        .skip(skip as u64)
        .limit(limit as i64)
        .await?;
    let mut results = Vec::new();

    while let Some(data) = cursor.try_next().await? {
        results.push(data);
    }

    // Pagination bilgisini oluştur
    let total_pages = ((total_items as f64) / (limit as f64)).ceil() as u32;
    let pagination = PaginationInfo {
        page,
        limit,
        total_items,
        total_pages,
        has_next: page < total_pages,
        has_previous: page > 1,
        next_cursor: None, // Basit pagination için şimdilik None
        previous_cursor: None,
    };

    Ok((results, pagination))
}

// Agregasyon ile özetlenmiş veriler (saatlik, günlük, haftalık, aylık)
#[allow(dead_code)]
pub async fn get_bazaar_data_aggregated(
    db: &Database,
    product_id: &str,
    aggregation_type: &str, // "hourly", "daily", "weekly", "monthly"
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
) -> Result<Vec<BazaarAggregatedData>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar");

    // Agregasyon için zaman formatı
    let time_format = match aggregation_type {
        "hourly" => "%Y-%m-%d %H:00:00",
        "daily" => "%Y-%m-%d",
        "weekly" => "%Y-%U", // Yıl-hafta
        "monthly" => "%Y-%m",
        _ => "%Y-%m-%d %H:00:00",
    };

    let pipeline = vec![
        doc! {
            "$match": {
                "product_id": product_id,
                "timestamp": {
                    "$gte": start_time.timestamp_millis(),
                    "$lte": end_time.timestamp_millis()
                }
            }
        },
        doc! {
            "$group": {
                "_id": {
                    "product_id": "$product_id",
                    "period": {
                        "$dateToString": {
                            "format": time_format,
                            "date": {
                                "$toDate": "$timestamp"
                            }
                        }
                    }
                },
                "avg_buy_price": { "$avg": "$buy_price" },
                "avg_sell_price": { "$avg": "$sell_price" },
                "min_buy_price": { "$min": "$buy_price" },
                "max_buy_price": { "$max": "$buy_price" },
                "min_sell_price": { "$min": "$sell_price" },
                "max_sell_price": { "$max": "$sell_price" },
                "total_buy_volume": { "$sum": "$buy_volume" },
                "total_sell_volume": { "$sum": "$sell_volume" },
                "avg_buy_volume": { "$avg": "$buy_volume" },
                "avg_sell_volume": { "$avg": "$sell_volume" },
                "data_points": { "$sum": 1 },
                "period_start": { "$min": { "$toDate": "$timestamp" } },
                "period_end": { "$max": { "$toDate": "$timestamp" } }
            }
        },
        doc! {
            "$sort": { "_id.period": 1 }
        },
    ];

    let mut cursor = collection.aggregate(pipeline).await?;
    let mut results = Vec::new();

    while let Some(doc) = cursor.try_next().await? {
        if let Ok(aggregated) = mongodb::bson::from_document::<Document>(doc) {
            let result = BazaarAggregatedData {
                product_id: product_id.to_string(),
                avg_buy_price: aggregated.get_f64("avg_buy_price").unwrap_or(0.0),
                avg_sell_price: aggregated.get_f64("avg_sell_price").unwrap_or(0.0),
                min_buy_price: aggregated.get_f64("min_buy_price").unwrap_or(0.0),
                max_buy_price: aggregated.get_f64("max_buy_price").unwrap_or(0.0),
                min_sell_price: aggregated.get_f64("min_sell_price").unwrap_or(0.0),
                max_sell_price: aggregated.get_f64("max_sell_price").unwrap_or(0.0),
                total_buy_volume: aggregated.get_i64("total_buy_volume").unwrap_or(0),
                total_sell_volume: aggregated.get_i64("total_sell_volume").unwrap_or(0),
                avg_buy_volume: aggregated.get_f64("avg_buy_volume").unwrap_or(0.0),
                avg_sell_volume: aggregated.get_f64("avg_sell_volume").unwrap_or(0.0),
                data_points: aggregated.get_i64("data_points").unwrap_or(0),
                period_start: aggregated
                    .get_datetime("period_start")
                    .ok()
                    .and_then(|dt| DateTime::from_timestamp(dt.timestamp_millis() / 1000, 0))
                    .unwrap_or(start_time),
                period_end: aggregated
                    .get_datetime("period_end")
                    .ok()
                    .and_then(|dt| DateTime::from_timestamp(dt.timestamp_millis() / 1000, 0))
                    .unwrap_or(end_time),
                aggregation_type: aggregation_type.to_string(),
                interval_minutes: match aggregation_type {
                    "hourly" => 60,
                    "daily" => 1440,
                    "weekly" => 10080,
                    "monthly" => 43200,
                    _ => 60,
                },
            };
            results.push(result);
        }
    }

    Ok(results)
}

// Akıllı veri getirme - compressed data'dan veya raw data'dan otomatik seçim
#[allow(dead_code)]
pub async fn get_bazaar_data_smart(
    db: &Database,
    product_id: &str,
    aggregation_type: &str,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
) -> Result<Vec<BazaarAggregatedData>, Box<dyn std::error::Error>> {
    let aggregated_collection = db.collection::<BazaarAggregatedData>("bazaar_aggregated");

    // Önce compressed data'da ara
    let filter = doc! {
        "product_id": product_id,
        "aggregation_type": aggregation_type,
        "period_start": { "$gte": start_time.timestamp_millis() },
        "period_end": { "$lte": end_time.timestamp_millis() }
    };

    let mut cursor = aggregated_collection
        .find(filter)
        .sort(doc! { "period_start": 1 })
        .await?;
    let mut results = Vec::new();

    while let Some(data) = cursor.try_next().await? {
        results.push(data);
    }

    // Eğer compressed data yoksa, raw data'dan real-time aggregation yap
    if results.is_empty() {
        println!("📊 No compressed data found, falling back to real-time aggregation");
        return get_bazaar_data_aggregated(db, product_id, aggregation_type, start_time, end_time)
            .await;
    }

    Ok(results)
}

// Eski fonksiyon - geriye dönük uyumluluk için
#[allow(dead_code)]
pub async fn get_bazaar_data_by_timeframe(
    db: &Database,
    product_id: &str,
    start_time: chrono::DateTime<chrono::Utc>,
    end_time: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<BazaarData>, Box<dyn std::error::Error>> {
    let (results, _) = get_bazaar_data_paginated(
        db,
        product_id,
        1,
        1000, // Maksimum 1000 kayıt
        Some(start_time),
        Some(end_time),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    Ok(results)
}

// === COMPRESSION TRACKING FUNCTIONS ===

async fn get_rollup_state(
    db: &Database,
    source_interval: &str,
    target_interval: &str,
) -> Result<Option<CompressionState>, Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionState>("compression_state");
    Ok(collection
        .find_one(doc! { "_id": rollup_state_id(source_interval, target_interval) })
        .await?)
}

async fn upsert_rollup_state(
    db: &Database,
    source_interval: &str,
    target_interval: &str,
    last_processed_period_start: Option<DateTime<Utc>>,
    compression_in_progress: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionState>("compression_state");
    let now = Utc::now();
    let mut set_doc = doc! {
        "source_interval": source_interval,
        "target_interval": target_interval,
        "compression_in_progress": compression_in_progress,
        "current_operation": Bson::Null,
        "updated_at": bson_datetime(now),
    };
    if let Some(last_processed) = last_processed_period_start {
        set_doc.insert("last_processed_period_start", bson_datetime(last_processed));
    }
    collection
        .update_one(
            doc! { "_id": rollup_state_id(source_interval, target_interval) },
            doc! {
                "$set": set_doc,
                "$setOnInsert": {
                    "created_at": bson_datetime(now),
                }
            },
        )
        .upsert(true)
        .await?;
    Ok(())
}

fn rollup_state_id(source_interval: &str, target_interval: &str) -> String {
    format!("{source_interval}_to_{target_interval}")
}

// Compression logging
#[allow(dead_code)]
pub async fn log_compression(
    db: &Database,
    log: &CompressionLog,
) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionLog>("compression_log");
    collection.insert_one(log).await?;
    Ok(())
}

pub async fn get_compression_logs(
    db: &Database,
    product_id: Option<&str>,
    compression_type: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<CompressionLog>, Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionLog>("compression_log");

    let mut filter = doc! {};
    if let Some(pid) = product_id {
        filter.insert("product_id", pid);
    }
    if let Some(ctype) = compression_type {
        filter.insert("compression_type", ctype);
    }

    let mut cursor = collection
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .limit(limit.unwrap_or(100))
        .await?;
    let mut logs = Vec::new();

    while let Some(log) = cursor.try_next().await? {
        logs.push(log);
    }

    Ok(logs)
}

// Get compression statistics
pub async fn get_compression_stats(db: &Database) -> Result<Document, Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionLog>("compression_log");

    let pipeline = vec![
        doc! {
            "$group": {
                "_id": "$compression_type",
                "total_compressions": { "$sum": 1 },
                "success_count": {
                    "$sum": {
                        "$cond": [{ "$eq": ["$status", "success"] }, 1, 0]
                    }
                },
                "total_source_records": { "$sum": "$source_records_count" },
                "total_compressed_records": { "$sum": "$compressed_records_count" },
                "total_bytes_saved": { "$sum": "$bytes_saved" },
                "avg_compression_time": { "$avg": "$compression_duration_ms" }
            }
        },
        doc! {
            "$group": {
                "_id": null,
                "by_type": {
                    "$push": {
                        "compression_type": "$_id",
                        "total_compressions": "$total_compressions",
                        "success_count": "$success_count",
                        "success_rate": {
                            "$multiply": [
                                { "$divide": ["$success_count", "$total_compressions"] },
                                100
                            ]
                        },
                        "total_source_records": "$total_source_records",
                        "total_compressed_records": "$total_compressed_records",
                        "compression_ratio": {
                            "$multiply": [
                                { "$divide": ["$total_compressed_records", "$total_source_records"] },
                                100
                            ]
                        },
                        "total_bytes_saved": "$total_bytes_saved",
                        "avg_compression_time": "$avg_compression_time"
                    }
                },
                "total_compressions": { "$sum": "$total_compressions" },
                "total_source_records": { "$sum": "$total_source_records" },
                "total_compressed_records": { "$sum": "$total_compressed_records" },
                "total_bytes_saved": { "$sum": "$total_bytes_saved" }
            }
        },
    ];

    let mut cursor = collection.aggregate(pipeline).await?;
    if let Some(result) = cursor.try_next().await? {
        Ok(result)
    } else {
        Ok(doc! {
            "total_compressions": 0,
            "by_type": [],
            "total_source_records": 0,
            "total_compressed_records": 0,
            "total_bytes_saved": 0
        })
    }
}

fn bson_datetime(value: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(value.timestamp_millis())
}

pub fn raw_expires_at(timestamp: DateTime<Utc>) -> DateTime<Utc> {
    timestamp + Duration::hours(24)
}

pub fn retention_expires_at(interval: &str, period_end: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match interval {
        "15s" => Some(period_end + Duration::hours(24)),
        "1m" => Some(period_end + Duration::days(30)),
        "5m" | "15m" => Some(period_end + Duration::days(180)),
        "1h" => Some(period_end + Duration::days(365 * 5)),
        "1d" | "1w" | "1mo" => None,
        _ => Some(period_end + Duration::hours(24)),
    }
}

pub fn interval_seconds(interval: &str) -> Option<i64> {
    match interval {
        "15s" => Some(15),
        "1m" => Some(60),
        "5m" => Some(5 * 60),
        "15m" => Some(15 * 60),
        "1h" => Some(60 * 60),
        "1d" => Some(24 * 60 * 60),
        "1w" => Some(7 * 24 * 60 * 60),
        "1mo" => Some(30 * 24 * 60 * 60),
        _ => None,
    }
}

pub fn candle_period(
    timestamp: DateTime<Utc>,
    interval: &str,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let seconds = interval_seconds(interval)?;
    let timestamp_seconds = timestamp.timestamp();
    let start_seconds = timestamp_seconds - timestamp_seconds.rem_euclid(seconds);
    let start = DateTime::<Utc>::from_timestamp(start_seconds, 0)?;
    let end = DateTime::<Utc>::from_timestamp(start_seconds + seconds, 0)?;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::{candle_period, interval_seconds};
    use chrono::{TimeZone, Utc};

    #[test]
    fn interval_parser_accepts_supported_chart_intervals() {
        assert_eq!(interval_seconds("15s"), Some(15));
        assert_eq!(interval_seconds("1m"), Some(60));
        assert_eq!(interval_seconds("1h"), Some(3600));
        assert_eq!(interval_seconds("bad"), None);
    }

    #[test]
    fn candle_period_floors_timestamp_to_interval() {
        let timestamp = Utc.with_ymd_and_hms(2026, 5, 28, 12, 34, 56).unwrap();
        let (start, end) = candle_period(timestamp, "1m").unwrap();

        assert_eq!(start, Utc.with_ymd_and_hms(2026, 5, 28, 12, 34, 0).unwrap());
        assert_eq!(end, Utc.with_ymd_and_hms(2026, 5, 28, 12, 35, 0).unwrap());
    }
}
