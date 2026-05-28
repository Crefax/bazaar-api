use crate::models::{BazaarData, BazaarAggregatedData, PaginationInfo, CompressionState, CompressionLog};
use mongodb::Database;
use mongodb::bson::{doc, Document};
use futures::TryStreamExt;
use chrono::{DateTime, Utc};

// Database initialization ve indexing
pub async fn ensure_indexes(db: &Database) -> Result<(), Box<dyn std::error::Error>> {
    // Raw data collection indexes
    let collection = db.collection::<BazaarData>("bazaar");
    
    // Compound index for product_id and timestamp (en önemli sorgu)
    let index_model = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1, "timestamp": -1 })
        .build();
    
    // Individual indexes
    let timestamp_index = mongodb::IndexModel::builder()
        .keys(doc! { "timestamp": -1 })
        .build();
        
    let product_index = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1 })
        .build();
        
    let price_index = mongodb::IndexModel::builder()
        .keys(doc! { "buy_price": 1, "sell_price": 1 })
        .build();
    
    collection.create_indexes(vec![index_model, timestamp_index, product_index, price_index]).await?;
    
    // Aggregated data collection indexes
    let aggregated_collection = db.collection::<BazaarAggregatedData>("bazaar_aggregated");
    
    let agg_compound_index = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1, "aggregation_type": 1, "period_start": -1 })
        .build();
        
    let agg_period_index = mongodb::IndexModel::builder()
        .keys(doc! { "period_start": -1, "period_end": -1 })
        .build();
        
    let agg_type_index = mongodb::IndexModel::builder()
        .keys(doc! { "aggregation_type": 1, "interval_minutes": 1 })
        .build();
    
        aggregated_collection.create_indexes(vec![agg_compound_index, agg_period_index, agg_type_index]).await?;

    // Compression tracking collection indexes
    let compression_state_collection = db.collection::<CompressionState>("compression_state");
    let cs_product_index = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1 })
        .options(mongodb::options::IndexOptions::builder().unique(true).build())
        .build();
    let cs_updated_index = mongodb::IndexModel::builder()
        .keys(doc! { "updated_at": -1 })
        .build();
    compression_state_collection.create_indexes(vec![cs_product_index, cs_updated_index]).await?;

    // Compression log collection indexes
    let compression_log_collection = db.collection::<CompressionLog>("compression_log");
    let cl_product_index = mongodb::IndexModel::builder()
        .keys(doc! { "product_id": 1, "created_at": -1 })
        .build();
    let cl_type_index = mongodb::IndexModel::builder()
        .keys(doc! { "compression_type": 1, "created_at": -1 })
        .build();
    let cl_status_index = mongodb::IndexModel::builder()
        .keys(doc! { "status": 1, "created_at": -1 })
        .build();
    compression_log_collection.create_indexes(vec![cl_product_index, cl_type_index, cl_status_index]).await?;

    println!("✅ Database indexes created successfully for raw, aggregated, and compression tracking data!");
    Ok(())
}

pub async fn insert_bazaar_data(db: &Database, data: BazaarData) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar");
    collection.insert_one(data).await?;
    Ok(())
}

pub async fn get_latest_bazaar_data(db: &Database, product_id: &str) -> Result<Option<BazaarData>, Box<dyn std::error::Error>> {
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
pub async fn get_bazaar_data_paginated(
    db: &Database,
    product_id: &str,
    page: u32,
    limit: u32,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
    min_price: Option<f64>,
    max_price: Option<f64>,
    sort_by: Option<String>,
    sort_order: Option<String>,
) -> Result<(Vec<BazaarData>, PaginationInfo), Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar");
    
    // Filter oluştur
    let mut filter = doc! { "product_id": product_id };
    
    // Tarih filtresi
    if let (Some(start), Some(end)) = (start_time, end_time) {
        filter.insert("timestamp", doc! {
            "$gte": start.timestamp_millis(),
            "$lte": end.timestamp_millis()
        });
    }
    
    // Fiyat filtresi
    if let Some(min_p) = min_price {
        filter.insert("buy_price", doc! { "$gte": min_p });
    }
    if let Some(max_p) = max_price {
        filter.insert("buy_price", doc! { "$lte": max_p });
    }
    
    // Toplam kayıt sayısını al
    let total_items = collection.count_documents(filter.clone()).await?;
    
    // Sort kriterini belirle
    let sort_field = sort_by.unwrap_or_else(|| "timestamp".to_string());
    let sort_direction = if sort_order.unwrap_or_else(|| "desc".to_string()) == "asc" { 1 } else { -1 };
    
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
        }
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
                period_start: aggregated.get_datetime("period_start")
                    .ok()
                    .and_then(|dt| DateTime::from_timestamp(dt.timestamp_millis() / 1000, 0))
                    .unwrap_or(start_time),
                period_end: aggregated.get_datetime("period_end")
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
        return get_bazaar_data_aggregated(db, product_id, aggregation_type, start_time, end_time).await;
    }
    
    Ok(results)
}

// Eski fonksiyon - geriye dönük uyumluluk için
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
        None
    ).await?;
    Ok(results)
}

// === COMPRESSION TRACKING FUNCTIONS ===

// Compression state management
pub async fn get_compression_state(db: &Database, product_id: &str) -> Result<Option<CompressionState>, Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionState>("compression_state");
    let state = collection.find_one(doc! { "product_id": product_id }).await?;
    Ok(state)
}

pub async fn upsert_compression_state(db: &Database, state: &CompressionState) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionState>("compression_state");
    let filter = doc! { "product_id": &state.product_id };
    let update = doc! {
        "$set": {
            "product_id": &state.product_id,
            "last_minutely_compression": state.last_minutely_compression.timestamp_millis(),
            "last_hourly_compression": state.last_hourly_compression.timestamp_millis(),
            "last_daily_compression": state.last_daily_compression.timestamp_millis(),
            "last_weekly_compression": state.last_weekly_compression.timestamp_millis(),
            "compression_in_progress": state.compression_in_progress,
            "current_operation": &state.current_operation,
            "updated_at": Utc::now().timestamp_millis()
        },
        "$setOnInsert": {
            "created_at": Utc::now().timestamp_millis()
        }
    };
    
    collection.update_one(filter, update).upsert(true).await?;
    Ok(())
}

// Compression logging
pub async fn log_compression(db: &Database, log: &CompressionLog) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<CompressionLog>("compression_log");
    collection.insert_one(log).await?;
    Ok(())
}

pub async fn get_compression_logs(
    db: &Database, 
    product_id: Option<&str>, 
    compression_type: Option<&str>,
    limit: Option<i64>
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

// Check if data needs compression based on last compression timestamp
pub async fn needs_compression(
    db: &Database, 
    product_id: &str, 
    compression_type: &str,
    cutoff_time: DateTime<Utc>
) -> Result<bool, Box<dyn std::error::Error>> {
    if let Some(state) = get_compression_state(db, product_id).await? {
        let last_compression = match compression_type {
            "minutely_to_hourly" => state.last_minutely_compression,
            "hourly_to_daily" => state.last_hourly_compression,
            "daily_to_weekly" => state.last_daily_compression,
            "weekly_to_monthly" => state.last_weekly_compression,
            _ => return Ok(true), // Unknown type, assume needs compression
        };
        
        // If last compression is before cutoff time, we need compression
        Ok(last_compression < cutoff_time)
    } else {
        // No compression state found, needs compression
        Ok(true)
    }
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
        }
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
