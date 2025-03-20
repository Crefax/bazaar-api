use crate::models::BazaarData;
use mongodb::Database;
use mongodb::bson::doc;
use futures::TryStreamExt;

pub async fn insert_bazaar_data(db: &Database, data: BazaarData) -> Result<(), Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar_data");
    collection.insert_one(data, None).await?;
    Ok(())
}

pub async fn get_latest_bazaar_data(db: &Database, product_id: &str) -> Result<Option<BazaarData>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar_data");
    let filter = doc! { "product_id": product_id };
    let options = mongodb::options::FindOptions::builder()
        .sort(doc! { "timestamp": -1 })
        .limit(1)
        .build();
    
    let mut cursor = collection.find(filter, options).await?;
    if let Some(data) = cursor.try_next().await? {
        Ok(Some(data))
    } else {
        Ok(None)
    }
}

pub async fn get_bazaar_data_by_timeframe(
    db: &Database,
    product_id: &str,
    start_time: chrono::DateTime<chrono::Utc>,
    end_time: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<BazaarData>, Box<dyn std::error::Error>> {
    let collection = db.collection::<BazaarData>("bazaar_data");
    
    // Print timestamps for debugging
    println!("Start time: {}, End time: {}", start_time, end_time);
    println!("Start millis: {}, End millis: {}", start_time.timestamp_millis(), end_time.timestamp_millis());
    
    let filter = doc! {
        "product_id": product_id,
        "timestamp": {
            "$gte": start_time.timestamp_millis(),
            "$lte": end_time.timestamp_millis()
        }
    };
    
    // Print filter for debugging
    println!("Filter: {:?}", filter);
    
    let mut cursor = collection.find(filter, None).await?;
    let mut results = Vec::new();
    
    while let Some(data) = cursor.try_next().await? {
        // Print found data for debugging
        println!("Found data: {:?}", data);
        results.push(data);
    }
    
    // Print total number of results for debugging
    println!("Total results: {}", results.len());
    
    Ok(results)
} 