#![allow(non_snake_case)]
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder, http::header};
use futures_util::stream::TryStreamExt;
use mongodb::{Client, options::{ClientOptions, IndexOptions}, bson::{doc, Document}, IndexModel};
use serde_json::json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_CHECK_LIMIT: i64 = 500; // API sorgu limiti

fn is_valid_product_id(product_id: &str) -> bool {
    product_id.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn is_valid_field(field: &str) -> bool {
    matches!(field, "sellPrice" | "buyPrice" | "sellVolume" | "buyVolume" | "sellOrders" | "buyOrders" | "sellMovingWeek" | "buyMovingWeek")
}

async fn get_database(client: &Client, db_name: &str, col_name: &str) -> mongodb::Collection<Document> {
    let database = client.database(db_name);
    database.collection::<Document>(col_name)
}

struct AppState {
    client: Client,
}

#[derive(Deserialize)]
struct ApiKeyQuery {
    key: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct QuickStatus {
    productId: String,
    sellPrice: f64,
    sellVolume: i64,
    sellMovingWeek: i64,
    sellOrders: i32,
    buyPrice: f64,
    buyVolume: i64,
    buyMovingWeek: i64,
    buyOrders: i32,
}

async fn create_indexes(client: &Client) {
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

// `checkLimit` ve `totalCheck` değerlerini artırır ve sorgu limitini kontrol eder
async fn increment_api_usage(apicollection: &mongodb::Collection<Document>, apikey: &str) -> mongodb::error::Result<bool> {
    let filter = doc! { "apikey": apikey };

    if let Some(api_document) = apicollection.find_one(filter.clone(), None).await? {
        if let Ok(check_limit) = api_document.get_i64("checkLimit") {
            if check_limit >= MAX_CHECK_LIMIT {
                return Ok(false);
            }
        }
    }

    let update = doc! {
        "$inc": { "checkLimit": 1, "totalCheck": 1 }
    };
    apicollection.update_one(filter, update, None).await?;
    Ok(true) // Limit aşılmadı, işlem başarılı
}


#[get("/api/skyblock/bazaar/{product_id}")]
async fn get_latest_product(
    product_id: web::Path<String>,
    query: web::Query<ApiKeyQuery>,
    data: web::Data<AppState>,
) -> impl Responder {

    if !is_valid_product_id(&product_id) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid item"}));
    }

    let apikey = &query.key;
    let collection = get_database(&data.client, "skyblock", "bazaar").await;
    let apicollection = get_database(&data.client, "users", "profile").await;

    let filter = doc! { "product_id": product_id.into_inner() };
    let sort = doc! { "timestamp": -1 };
    let options = mongodb::options::FindOneOptions::builder().sort(sort).build();

    let apifilter = doc! { "apikey": apikey };
    let apioptions = mongodb::options::FindOneOptions::builder().build();

    if let Ok(Some(result)) = apicollection.find_one(apifilter, apioptions).await {
        if let Some(status) = result.get_bool("status").ok() {
            if status == false {
                return HttpResponse::NotFound().json(json!({"error": "status false"}));
            }
            
            // Sorgu limiti kontrolü
            match increment_api_usage(&apicollection, apikey).await {
                Ok(true) => {
                    if let Ok(Some(product)) = collection.find_one(filter, options).await {
                        let mut product_json: Value = serde_json::to_value(product).unwrap();
                        product_json.as_object_mut().unwrap().remove("_id");

                        return HttpResponse::Ok().json(product_json);
                    } else {
                        return HttpResponse::NotFound().json(json!({"error": "Item data not found"}));
                    }
                },
                Ok(false) => return HttpResponse::TooManyRequests().json(json!({"error": "Sorgu limiti aşıldı"})),
                Err(e) => eprintln!("API kullanımı güncellenemedi: {}", e),
            }
        } else {
            return HttpResponse::NotFound().json(json!({"error": "Key Not Found"}));
        }
    } else {
        return HttpResponse::NotFound().json(json!({"error": "Key Not Found"}));
    }

    HttpResponse::InternalServerError().finish()
}

#[get("/api/skyblock/bazaar/{product_id}/{field}")]
async fn get_latest_field(
    params: web::Path<(String, String)>,
    query: web::Query<ApiKeyQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let (product_id, field) = params.into_inner();

    if !is_valid_product_id(&product_id) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid item"}));
    }
    if !is_valid_field(&field) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid field"}));
    }

    let apikey = &query.key;
    let collection = get_database(&data.client, "skyblock", "bazaar").await;
    let apicollection = get_database(&data.client, "users", "profile").await;

    let filter = doc! { "product_id": product_id };
    let sort = doc! { "timestamp": -1 };
    let options = mongodb::options::FindOneOptions::builder().sort(sort).build();

    let apifilter = doc! { "apikey": apikey };
    let apioptions = mongodb::options::FindOneOptions::builder().build();

    if let Ok(Some(result)) = apicollection.find_one(apifilter, apioptions).await {
        if let Some(status) = result.get_bool("status").ok() {
            if status == false {
                return HttpResponse::NotFound().json(json!({"error": "status false"}));
            }
            
            // Sorgu limiti kontrolü
            match increment_api_usage(&apicollection, apikey).await {
                Ok(true) => {
                    if let Ok(Some(result)) = collection.find_one(filter, options).await {
                        if let Some(quick_status) = result.get("quick_status").and_then(|qs| qs.as_document()) {
                            if let Some(value) = quick_status.get(&field) {
                                return HttpResponse::Ok().json(value);
                            }
                        }
                    }
                },
                Ok(false) => return HttpResponse::TooManyRequests().json(json!({"error": "Sorgu limiti aşıldı"})),
                Err(e) => eprintln!("API kullanımı güncellenemedi: {}", e),
            }
        } else {
            return HttpResponse::Ok().json("Key Kullanım Dışı!");
        }
    } else {
        return HttpResponse::NotFound().json(json!({"error": "Key Not Found"}));
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

    if !is_valid_product_id(&product_id) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid item"}));
    }
    if !is_valid_field(&field) {
        return HttpResponse::BadRequest().json(json!({"error": "invalid field"}));
    }

    let apikey = &query.key;
    let collection = get_database(&data.client, "skyblock", "bazaar").await;
    let apicollection = get_database(&data.client, "users", "profile").await;

    let filter = doc! { "product_id": product_id };
    let sort = doc! { "timestamp": -1 };
    let options = mongodb::options::FindOptions::builder().sort(sort).limit(limit as i64).build();
    let mut cursor = collection.find(filter, options).await.unwrap();

    let apifilter = doc! { "apikey": apikey };
    let apioptions = mongodb::options::FindOneOptions::builder().build();

    let mut results = Vec::new();

    if let Ok(Some(apiresult)) = apicollection.find_one(apifilter, apioptions).await {
        if let Some(status) = apiresult.get_bool("status").ok() {
            if status == false {
                return HttpResponse::NotFound().json(json!({"error": "status false"}));
            } else {
                // Sorgu limiti kontrolü
                match increment_api_usage(&apicollection, apikey).await {
                    Ok(true) => {
                        while let Some(result) = cursor.try_next().await.unwrap() {
                            if let Some(quick_status) = result.get("quick_status").and_then(|qs| qs.as_document()) {
                                if let Some(value) = quick_status.get(&field) {
                                    results.push(value.clone());
                                }
                            }
                        }
                    },
                    Ok(false) => return HttpResponse::TooManyRequests().json(json!({"error": "Sorgu limiti aşıldı"})),
                    Err(e) => eprintln!("API kullanımı güncellenemedi: {}", e),
                }
            }
        } else {
            return HttpResponse::Ok().json("Key Kullanım Dışı!");
        }
    } else {
        return HttpResponse::NotFound().json(json!({"error": format!("Key Not Found")}));
    }

    if results.is_empty() {
        HttpResponse::NotFound().json(json!({"error": format!("'{}' alanı için veri bulunamadı", field)}))
    } else {
        HttpResponse::Ok().json(json!(results))
    }
}

async fn not_found() -> impl Responder {
    HttpResponse::NotFound()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .body(r#"{"error": "Page not found"}"#)
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_uri = "mongodb://localhost:27017";
    let client_options = ClientOptions::parse(client_uri).await?;
    let client = Client::with_options(client_options)?;

    create_indexes(&client).await;

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(AppState {
                client: client.clone(),
            }))
            .service(get_latest_field)
            .service(get_latest_product)
            .service(get_fields)
            .default_service(
                web::route().to(not_found)
            )
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await?;

    Ok(())
}
