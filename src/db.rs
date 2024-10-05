use mongodb::{bson::doc, Client, Collection, options::IndexOptions, IndexModel};
use bson::Document;

const MAX_CHECK_LIMIT: i32 = 500;

// Get Database with db_name and col_name
pub async fn get_database(client: &Client, db_name: &str, col_name: &str) -> Collection<Document> {
    let database = client.database(db_name);
    database.collection::<Document>(col_name)
}

// Create Indexes
pub async fn create_indexes(client: &Client) {
    let db = client.database("skyblock");
    let collection = db.collection::<Document>("bazaar");

    let index_model = IndexModel::builder()
        .keys(doc! { "product_id": 1, "timestamp": -1 })
        .options(IndexOptions::builder().unique(false).build())
        .build();

    if let Err(e) = collection.create_index(index_model, None).await {
        eprintln!("Index Error: {}", e);
    }
}

// Check API Key and Increment Usage
pub async fn check_api_key_and_increment_usage(
    apikey: &str,
    apicollection: &Collection<Document>,
) -> mongodb::error::Result<bool> {
    let filter = doc! { "apikey": apikey };

    if let Some(api_document) = apicollection.find_one(filter.clone(), None).await? {
        let checkLimit: i32 = api_document.get_i32("checkLimit").unwrap_or(0);
        let totalCheck: i32 = api_document.get_i32("totalCheck").unwrap_or(0);
        
        if checkLimit < MAX_CHECK_LIMIT {
            apicollection
                .update_one(
                    filter,
                    doc! {
                        "$set": {
                            "checkLimit": checkLimit + 1,
                            "totalCheck": totalCheck + 1
                        }
                    },
                    None
                )
                .await?;
            return Ok(true);
        }
    }
    Ok(false)
}

// Reset Check Limits
pub async fn reset_check_limits(client: &Client) -> mongodb::error::Result<()> {
    let collection = client.database("users").collection::<Document>("profile");
    
    let filter = doc! { "checkLimit": { "$ne": 0 } };
    let update = doc! { "$set": { "checkLimit": 0 } };
    
    collection.update_many(filter, update, None).await?;
    
    Ok(())
}