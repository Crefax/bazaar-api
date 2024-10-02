use actix_web::{get, web, HttpResponse, Responder};
use futures_util::stream::TryStreamExt;
use mongodb::bson::doc;
use serde_json::json;
use serde_json::Value;
use crate::AppState;
use crate::utils;
use crate::{db, models::ApiKeyQuery};


#[get("/api/skyblock/bazaar/{product_id}")]
async fn get_latest_product(
    product_id: web::Path<String>,
    data: web::Data<AppState>,
) -> impl Responder {
    if !utils::is_valid_product_id(&product_id) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid item"}));
    }

    let collection = db::get_database(&data.client, "skyblock", "bazaar").await;

    let filter = doc! { "product_id": product_id.into_inner() };
    let sort = doc! { "timestamp": -1 };
    let options = mongodb::options::FindOneOptions::builder().sort(sort).build();

    if let Ok(Some(product)) = collection.find_one(filter, options).await {
        let mut product_json: Value = serde_json::to_value(product).unwrap();
        product_json.as_object_mut().unwrap().remove("_id");

        return HttpResponse::Ok().json(product_json);
    } else {
        return HttpResponse::NotFound().json(json!({"error": "Item data not found"}));
    }
}

#[get("/api/skyblock/bazaar/{product_id}/{field}")]
async fn get_latest_field(
    params: web::Path<(String, String)>,
    query: web::Query<ApiKeyQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let (product_id, field) = params.into_inner();

    if !utils::is_valid_product_id(&product_id) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid item"}));
    }
    if !utils::is_valid_field(&field) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid field"}));
    }

    let apikey = &query.key;
    let collection = db::get_database(&data.client, "skyblock", "bazaar").await;
    let apicollection = db::get_database(&data.client, "users", "profile").await;

    let filter = doc! { "product_id": product_id };
    let sort = doc! { "timestamp": -1 };
    let options = mongodb::options::FindOneOptions::builder().sort(sort).build();

    // Check API key and increment usage
    match db::check_api_key_and_increment_usage(apikey, &apicollection).await {
        Ok(true) => {
            if let Ok(Some(result)) = collection.find_one(filter, options).await {
                if let Some(quick_status) = result.get("quick_status").and_then(|qs| qs.as_document()) {
                    if let Some(value) = quick_status.get(&field) {
                        return HttpResponse::Ok().json(value);
                    }
                }
            }
        }
        Ok(false) => return HttpResponse::TooManyRequests().json(json!({"error": "API Limit Exceeded"})),
        Err(e) => eprintln!("Failed to Update API: {}", e),
    }

    HttpResponse::NotFound().json(json!({"error": format!("'{}' alanı için veri bulunamadı", field)}))
}

#[get("/api/skyblock/bazaar/{product_id}/{field}/{limit}")]
async fn get_fields(
    params: web::Path<(String, String, usize)>,
    query: web::Query<ApiKeyQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let (product_id, field, limit) = params.into_inner();

    if !utils::is_valid_product_id(&product_id) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid item"}));
    }
    if !utils::is_valid_field(&field) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid field"}));
    }

    let apikey = &query.key;
    let collection = db::get_database(&data.client, "skyblock", "bazaar").await;
    let apicollection = db::get_database(&data.client, "users", "profile").await;

    let filter = doc! { "product_id": product_id };
    let sort = doc! { "timestamp": -1 };
    let options = mongodb::options::FindOptions::builder().sort(sort).limit(limit as i64).build();
    let mut cursor = collection.find(filter, options).await.unwrap();

    // Check API key and increment usage
    match db::check_api_key_and_increment_usage(apikey, &apicollection).await {
        Ok(true) => {
            let mut results = Vec::new();
            while let Some(result) = cursor.try_next().await.unwrap() {
                if let Some(quick_status) = result.get("quick_status").and_then(|qs| qs.as_document()) {
                    if let Some(value) = quick_status.get(&field) {
                        results.push(value.clone());
                    }
                }
            }
            return HttpResponse::Ok().json(json!(results));
        }
        Ok(false) => return HttpResponse::TooManyRequests().json(json!({"error": "API Limit Exceeded"})),
        Err(e) => eprintln!("Failed to Update API: {}", e),
    }

    HttpResponse::NotFound().json(json!({"error": format!("'{}' alanı için veri bulunamadı", field)}))
}

pub async fn not_found() -> impl Responder {
    HttpResponse::NotFound().body("404 Not Found") 
}
