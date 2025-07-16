use actix_web::{web, App, HttpServer, Responder, HttpResponse};
use crate::fetch_contribution_stats;
use std::env;
use std::sync::Mutex;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use lazy_static::lazy_static;
use serde::{Serialize, Deserialize};

// In-memory cache
lazy_static! {
    static ref MEMORY_CACHE: Mutex<HashMap<String, (crate::ContributionStats, u64)>> = Mutex::new(HashMap::new());
}

#[derive(Serialize, Deserialize)]
struct FileCache(HashMap<String, (crate::ContributionStats, u64)>);

pub async fn run_api_server() -> std::io::Result<()> {
    let cache_enabled = std::env::var("CACHE_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
    let cache_type = std::env::var("CACHE_TYPE").unwrap_or_else(|_| "memory".to_string());
    let cache_duration_secs: u64 = std::env::var("CACHE_DURATION_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3600);

    // Spawn background cleanup for memory cache
    if cache_enabled && cache_type == "memory" {
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(60);
            loop {
                tokio::time::sleep(interval).await;
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                let mut cache = MEMORY_CACHE.lock().unwrap();
                cache.retain(|_, &mut (_, timestamp)| now - timestamp < cache_duration_secs);
            }
        });
    }

    HttpServer::new(|| {
        App::new()
            .route("/stats/{username}", web::get().to(stats_handler))
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}

async fn stats_handler(path: web::Path<String>) -> impl Responder {
    let username = path.into_inner();
    let cache_enabled = env::var("CACHE_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
    let cache_type = env::var("CACHE_TYPE").unwrap_or_else(|_| "memory".to_string());
    let cache_duration_secs: u64 = env::var("CACHE_DURATION_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3600);
    let cache_file_path = env::var("CACHE_FILE_PATH").unwrap_or_else(|_| "cache.json".to_string());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    if cache_enabled {
        if cache_type == "memory" {
            // In-memory cache
            if let Some(stats) = {
                let cache = MEMORY_CACHE.lock().unwrap();
                cache.get(&username).cloned()
            } {
                if now - stats.1 < cache_duration_secs {
                    return HttpResponse::Ok().json(stats.0);
                }
            }
        } else if cache_type == "file" {
            // File cache
            if let Ok(mut file) = std::fs::File::open(&cache_file_path) {
                if let Ok(file_cache) = serde_json::from_reader::<_, FileCache>(&mut file) {
                    if let Some((stats, timestamp)) = file_cache.0.get(&username) {
                        if now - *timestamp < cache_duration_secs {
                            return HttpResponse::Ok().json(stats);
                        }
                    }
                }
            }
        }
    }

    // Not cached or expired, fetch fresh
    match fetch_contribution_stats(&username, None).await {
        Ok(stats) => {
            if cache_enabled {
                if cache_type == "memory" {
                    let mut cache = MEMORY_CACHE.lock().unwrap();
                    cache.insert(username.clone(), (stats.clone(), now));
                } else if cache_type == "file" {
                    let mut cache_map = if let Ok(mut file) = std::fs::File::open(&cache_file_path) {
                        if let Ok(file_cache) = serde_json::from_reader::<_, FileCache>(&mut file) {
                            file_cache.0
                        } else {
                            HashMap::new()
                        }
                    } else {
                        HashMap::new()
                    };
                    cache_map.insert(username.clone(), (stats.clone(), now));
                    let file_cache = FileCache(cache_map);
                    if let Ok(mut file) = std::fs::File::create(&cache_file_path) {
                        let _ = serde_json::to_writer(&mut file, &file_cache);
                    }
                }
            }
            HttpResponse::Ok().json(stats)
        },
        Err(e) => HttpResponse::InternalServerError().body(format!("Error: {}", e)),
    }
} 