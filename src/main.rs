use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use futures_util::stream::TryStreamExt; // Doğru kütüphane içe aktarılıyor
use mongodb::{Client, options::ClientOptions, bson::{doc, Document}};
use serde_json::json;
use serde::{Deserialize, Serialize};
use serde_json::Value;



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

#[get("/api/{product_id}")]
async fn get_latest_product(product_id: web::Path<String>) -> impl Responder {
    // MongoDB'ye bağlanmak için bağlantı URI'sini ayarla
    let client_uri = "mongodb://localhost:27017"; // Kendi MongoDB bağlantı URI'nı güncelle
    let mut client_options = ClientOptions::parse(client_uri).await.unwrap();
    client_options.app_name = Some("hypixel".to_string()); // Uygulama adını ekle
    let client = Client::with_options(client_options).unwrap();

    // Veritabanı ve koleksiyonlara erişim
    let database = client.database("skyblock"); // hypixel içindeki skyblock veritabanını seç
    let collection = database.collection::<Document>("bazaar"); // 'bazaar' koleksiyonunu kullan

    // Belirtilen product_id'ye göre filtrele ve en son veriyi getir
    let filter = doc! { "product_id": product_id.into_inner() };
    let sort = doc! { "timestamp": -1 }; // Zaman damgasına göre azalan sırada sırala
    let options = mongodb::options::FindOneOptions::builder().sort(sort).build();

    // En son ürün verisini çek
    if let Ok(Some(product)) = collection.find_one(filter, options).await {
        // _id alanını kaldır
        let mut product_json: Value = serde_json::to_value(product).unwrap();
        product_json.as_object_mut().unwrap().remove("_id"); // _id alanını çıkar

        HttpResponse::Ok().json(product_json) // Veriyi JSON olarak döndür
    } else {
        HttpResponse::NotFound().json(json!({"error": "Ürün verisi bulunamadı"})) // Ürün bulunamazsa hata mesajı
    }
}


#[get("/api/{product_id}/{field}")]
async fn get_latest_field(
    params: web::Path<(String, String)>,
) -> impl Responder {
    let (product_id, field) = params.into_inner();
    
    // MongoDB bağlantısı
    let client_uri = "mongodb://localhost:27017";
    let mut client_options = ClientOptions::parse(client_uri).await.unwrap();
    client_options.app_name = Some("hypixel".to_string());
    let client = Client::with_options(client_options).unwrap();

    // Veritabanı ve koleksiyon
    let database = client.database("skyblock");
    let collection = database.collection::<Document>("bazaar");

    // MongoDB sorgusu: product_id'yi filtrele ve en son veriyi getir
    let filter = doc! { "product_id": product_id };
    let sort = doc! { "timestamp": -1 }; // Zaman damgasına göre sırala
    let options = mongodb::options::FindOneOptions::builder().sort(sort).build();

    // En son veriyi çek
    if let Ok(Some(result)) = collection.find_one(filter, options).await {
        // İstenilen alana erişim
        if let Some(quick_status) = result.get("quick_status").and_then(|qs| qs.as_document()) {
            if let Some(value) = quick_status.get(&field) {
                // Sonucu JSON olarak döndür
                return HttpResponse::Ok().json(value);
            }
        }
    }

    // Eğer sonuçlar bulunmazsa hata döndür
    HttpResponse::NotFound().json(json!({"error": format!("'{}' alanı için veri bulunamadı", field)}))
}

#[get("/api/{product_id}/{field}/{limit}")]
async fn get_fields(
    params: web::Path<(String, String, usize)>,
) -> impl Responder {
    let (product_id, field, limit) = params.into_inner();
    
    // MongoDB bağlantısı
    let client_uri = "mongodb://localhost:27017";
    let mut client_options = ClientOptions::parse(client_uri).await.unwrap();
    client_options.app_name = Some("hypixel".to_string());
    let client = Client::with_options(client_options).unwrap();

    // Veritabanı ve koleksiyon
    let database = client.database("skyblock");
    let collection = database.collection::<Document>("bazaar");

    // MongoDB sorgusu: product_id'yi filtrele ve limit kadar sonuç getir
    let filter = doc! { "product_id": product_id };
    let sort = doc! { "timestamp": -1 }; // Zaman damgasına göre artan sırada sırala (en eski veriden yeni veriye)
    let options = mongodb::options::FindOptions::builder().sort(sort).limit(limit as i64).build();
    let mut cursor = collection.find(filter, options).await.unwrap();

    // İstenen alanları toplayacağımız vektör
    let mut results = Vec::new();

    // Sorgu sonuçlarını döngüyle işleyelim
    while let Some(result) = cursor.try_next().await.unwrap() {
        if let Some(quick_status) = result.get("quick_status").and_then(|qs| qs.as_document()) {
            if let Some(value) = quick_status.get(&field) {
                results.push(value.clone()); // İstenen alanı vektöre ekle
            }
        }
    }

    // Eğer sonuçlar bulunmazsa hata döndür
    if results.is_empty() {
        HttpResponse::NotFound().json(json!({"error": format!("'{}' alanı için veri bulunamadı", field)}))
    } else {
        // Sonuçları JSON olarak döndür
        HttpResponse::Ok().json(json!(results))
    }
}



#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(get_latest_field)
            .service(get_latest_product)
            .service(get_fields)
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}