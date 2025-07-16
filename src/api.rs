use actix_web::{web, App, HttpServer, Responder, HttpResponse, HttpRequest};
use crate::fetch_contribution_stats;
use crate::config::Config;
use std::sync::Mutex;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use lazy_static::lazy_static;
use serde::{Serialize, Deserialize};
use askama::Template;
use crate::color;
use crate::templates::{ContributionStatsTemplate, ContributionSvgGraphTemplate, ContributionGraphHtmlTemplate, GraphCell};
use log::{info, error};

lazy_static! {
    static ref MEMORY_CACHE: Mutex<HashMap<String, (crate::ContributionStats, u64)>> = Mutex::new(HashMap::new());
}

#[derive(Serialize, Deserialize)]
struct FileCache(HashMap<String, (crate::ContributionStats, u64)>);

fn prepare_graph_template_data<'a>(
    stats: &'a crate::ContributionStats,
    params: &HashMap<String, String>,
    config: &Config
) -> ContributionSvgGraphTemplate<'a> {
    let primary_color = params.get("primary-color").cloned().unwrap_or_else(|| config.default_fg.clone());
    let bg_color = params.get("background-color").cloned().unwrap_or_else(|| config.default_bg.clone());
    let svg_height = params.get("svg-height").cloned().unwrap_or_else(|| config.default_svg_height.clone());
    let show_months = params.get("show-months").and_then(|v| v.parse::<bool>().ok()).unwrap_or(config.default_show_months);
    let show_weekdays = params.get("show-weekdays").and_then(|v| v.parse::<bool>().ok()).unwrap_or(config.default_show_weekdays);

    let max_count = stats.daily_contributions.iter().map(|(_, c, _)| *c).max().unwrap_or(0);
    let max_rows = 7;
    let color_shades = color::derive_color_shades_with_bg(&primary_color, &bg_color);
    let cells: Vec<GraphCell> = stats.daily_contributions.iter().enumerate().map(|(i, (date, count, label))| {
        let col = i / max_rows;
        let row = i % max_rows;
        let color = match *count {
            c if c > 15 => color_shades[4].clone(),
            c if c > 8 => color_shades[3].clone(),
            c if c > 4 => color_shades[2].clone(),
            c if c > 0 => color_shades[1].clone(),
            _ => color_shades[0].clone(),
        };
        let hover_text = if !label.is_empty() { label.clone() } else { format!("{}: {} contributions", date, count) };
        GraphCell {
            date: date.clone(),
            count: *count,
            col,
            row,
            color,
            hover_text,
        }
    }).collect();
    let mut month_labels = Vec::new();
    let mut last_month = String::new();
    for (i, (date, _, _)) in stats.daily_contributions.iter().enumerate() {
        if let Ok(ndate) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            let month = ndate.format("%b").to_string();
            if month != last_month {
                month_labels.push((i / max_rows, month.clone()));
                last_month = month;
            }
        }
    }
    ContributionSvgGraphTemplate{
        stats,
        max_count,
        cells,
        show_months,
        show_weekdays,
        primary_color,
        color_shades,
        month_labels,
        weekday_labels: config.weekday_labels.clone(),
        svg_height,
        cell_radius: config.cell_radius,
    }
}

fn add_widget_headers(username: &str, builder: &mut actix_web::HttpResponseBuilder) {
    builder.insert_header(("Widget-Title", "GitHub Contributions"));
    builder.insert_header(("Widget-Title-URL", format!("https://github.com/{}", username)));
    builder.insert_header(("Widget-Content-Type", "html"));
}

pub async fn run_api_server() -> std::io::Result<()> {
    let config = Config::from_env();

    info!("Starting API server on 0.0.0.0:8080");
    info!("Cache enabled: {}, type: {}, duration: {}s", config.cache_enabled, config.cache_type, config.cache_duration_secs);

    if config.cache_enabled && config.cache_type == "memory" {
        let config_clone = config.clone();
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(60);
            loop {
                tokio::time::sleep(interval).await;
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                let mut cache = MEMORY_CACHE.lock().unwrap();
                let before = cache.len();
                cache.retain(|_, &mut (_, timestamp)| now - timestamp < config_clone.cache_duration_secs);
                let after = cache.len();
                if before != after {
                    info!("Memory cache cleaned: {} -> {} entries", before, after);
                }
            }
        });
    }

    HttpServer::new(|| {
        App::new()
            .route("/stats/{username}", web::get().to(stats_handler))
            .route("/graph_svg/{username}", web::get().to(|path, req| svg_graph_handler(path, req)))
            .route("/graph/{username}", web::get().to(|path, req| graph_html_handler(path, req)))
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}

async fn get_stats(username: &str) -> Result<crate::ContributionStats, String> {
    let config = Config::from_env();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let stats = if config.cache_enabled {
        if config.cache_type == "memory" {
            if let Some(stats) = {
                let cache = MEMORY_CACHE.lock().unwrap();
                cache.get(username).cloned()
            } {
                if now - stats.1 < config.cache_duration_secs {
                    Some(stats.0)
                } else {
                    None
                }
            } else {
                None
            }
        } else if config.cache_type == "file" {
            if let Ok(mut file) = std::fs::File::open(&config.cache_file_path) {
                if let Ok(file_cache) = serde_json::from_reader::<_, FileCache>(&mut file) {
                    if let Some((stats, timestamp)) = file_cache.0.get(username) {
                        if now - *timestamp < config.cache_duration_secs {
                            Some(stats.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let stats = match stats {
        Some(stats) => stats,
        None => match fetch_contribution_stats(username, None).await {
            Ok(stats) => {
                if config.cache_enabled {
                    if config.cache_type == "memory" {
                        let mut cache = MEMORY_CACHE.lock().unwrap();
                        cache.insert(username.to_string(), (stats.clone(), now));
                    } else if config.cache_type == "file" {
                        let mut cache_map = if let Ok(mut file) = std::fs::File::open(&config.cache_file_path) {
                            if let Ok(file_cache) = serde_json::from_reader::<_, FileCache>(&mut file) {
                                file_cache.0
                            } else {
                                HashMap::new()
                            }
                        } else {
                            HashMap::new()
                        };
                        cache_map.insert(username.to_string(), (stats.clone(), now));
                        let file_cache = FileCache(cache_map);
                        if let Ok(mut file) = std::fs::File::create(&config.cache_file_path) {
                            let _ = serde_json::to_writer(&mut file, &file_cache);
                        }
                    }
                }
                stats
            },
            Err(e) => return Err(format!("Error: {}", e)),
        },
    };
    Ok(stats)
}

async fn stats_handler(path: web::Path<String>, req: HttpRequest) -> impl Responder {
    let username = path.into_inner();
    info!("Received /stats request for user: {}", username);
    let query = req.query_string();
    let params: HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let show_quartiles = params.get("show_quartiles").map(|v| v == "true").unwrap_or(true);
    match get_stats(&username).await {
        Ok(stats) => {
            info!("Successfully got stats for user: {}", username);
            let template = ContributionStatsTemplate { 
                stats: &stats,
                show_quartiles,
                quartiles_string: stats.quartiles.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(" "),
            };
            match template.render() {
                Ok(body) => HttpResponse::Ok()
                    .content_type("text/html")
                    .insert_header(("Widget-Title", "GitHub Stats"))
                    .insert_header(("Widget-Title-Url", format!("https://github.com/{}", username)))
                    .insert_header(("Widget-Content-Type", "html"))
                    .body(body),
                Err(e) => {
                    error!("Template error for user '{}': {}", username, e);
                    HttpResponse::InternalServerError().body(format!("Template error: {}", e))
                },
            }
        },
        Err(e) => {
            error!("Failed to get stats for user '{}': {}", username, e);
            HttpResponse::InternalServerError().body(e)
        },
    }
}

async fn svg_graph_handler(path: web::Path<String>, req: HttpRequest) -> impl Responder {
    let username = path.into_inner();
    let query = req.query_string();
    let params: HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let config = Config::from_env();
    match get_stats(&username).await {
        Ok(stats) => {
            let template = prepare_graph_template_data(&stats, &params, &config);
            let mut builder = HttpResponse::Ok();
            add_widget_headers(&username, &mut builder);
            match template.render() {
                Ok(body) => builder
                    .content_type("image/svg+xml")
                    .insert_header(("Widget-Content-Type", "html"))
                    .body(body),
                Err(e) => HttpResponse::InternalServerError().body(format!("Template error: {}", e)),
            }
        },
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

async fn graph_html_handler(path: web::Path<String>, req: HttpRequest) -> impl Responder {
    let username = path.into_inner();
    let query = req.query_string();
    let params: HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let config = Config::from_env();
    match get_stats(&username).await {
        Ok(stats) => {
            let svg = prepare_graph_template_data(&stats, &params, &config);
            let quartiles = svg.stats.quartiles.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(" ");
            let template = ContributionGraphHtmlTemplate {
                svg,
                quartiles,
            };
            let mut builder = HttpResponse::Ok();
            add_widget_headers(&username, &mut builder);
            match template.render() {
                Ok(body) => builder
                    .content_type("text/html")
                    .body(body),
                Err(e) => HttpResponse::InternalServerError().body(format!("Template error: {}", e)),
            }
        },
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
} 
