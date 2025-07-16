use actix_web::{web, App, HttpServer, Responder, HttpResponse, HttpRequest};
use crate::fetch_contribution_stats;
use std::env;
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

struct GraphTemplateData {
    max_count: u32,
    cells: Vec<GraphCell>,
    show_months: bool,
    show_weekdays: bool,
    primary_color: String,
    color_shades: Vec<String>,
    month_labels: Vec<(usize, String)>,
    weekday_labels: Vec<(usize, &'static str)>,
    cell_radius: u32,
}

fn prepare_graph_template_data(
    stats: &crate::ContributionStats,
    primary_color: String,
    bg_color: String
) -> GraphTemplateData {
    let max_count = stats.daily_contributions.iter().map(|(_, c, _)| *c).max().unwrap_or(0);
    let max_rows = 7;
    let show_months = std::env::var("GRAPH_SHOW_MONTHS").unwrap_or_else(|_| "true".to_string()) == "true";
    let show_weekdays = std::env::var("GRAPH_SHOW_WEEKDAYS").unwrap_or_else(|_| "true".to_string()) == "true";
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
    let weekday_labels = vec![(1, "Mon"), (3, "Wed"), (5, "Fri")];
    let cell_radius = std::env::var("GRAPH_CELL_RADIUS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    GraphTemplateData {
        max_count,
        cells,
        show_months,
        show_weekdays,
        primary_color,
        color_shades,
        month_labels,
        weekday_labels,
        cell_radius,
    }
}

fn add_widget_headers(username: &str, builder: &mut actix_web::HttpResponseBuilder) {
    builder.insert_header(("Widget-Title", "GitHub Contributions"));
    builder.insert_header(("Widget-Title-URL", format!("https://github.com/{}", username)));
    builder.insert_header(("Widget-Content-Type", "html"));
}

pub async fn run_api_server() -> std::io::Result<()> {
    let cache_enabled = std::env::var("CACHE_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
    let cache_type = std::env::var("CACHE_TYPE").unwrap_or_else(|_| "memory".to_string());
    let cache_duration_secs: u64 = std::env::var("CACHE_DURATION_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3600);

    info!("Starting API server on 0.0.0.0:8080");
    info!("Cache enabled: {}, type: {}, duration: {}s", cache_enabled, cache_type, cache_duration_secs);

    if cache_enabled && cache_type == "memory" {
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(60);
            loop {
                tokio::time::sleep(interval).await;
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                let mut cache = MEMORY_CACHE.lock().unwrap();
                let before = cache.len();
                cache.retain(|_, &mut (_, timestamp)| now - timestamp < cache_duration_secs);
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
    let cache_enabled = env::var("CACHE_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
    let cache_type = env::var("CACHE_TYPE").unwrap_or_else(|_| "memory".to_string());
    let cache_duration_secs: u64 = env::var("CACHE_DURATION_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3600);
    let cache_file_path = env::var("CACHE_FILE_PATH").unwrap_or_else(|_| "cache.json".to_string());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let stats = if cache_enabled {
        if cache_type == "memory" {
            if let Some(stats) = {
                let cache = MEMORY_CACHE.lock().unwrap();
                cache.get(username).cloned()
            } {
                if now - stats.1 < cache_duration_secs {
                    Some(stats.0)
                } else {
                    None
                }
            } else {
                None
            }
        } else if cache_type == "file" {
            if let Ok(mut file) = std::fs::File::open(&cache_file_path) {
                if let Ok(file_cache) = serde_json::from_reader::<_, FileCache>(&mut file) {
                    if let Some((stats, timestamp)) = file_cache.0.get(username) {
                        if now - *timestamp < cache_duration_secs {
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
                if cache_enabled {
                    if cache_type == "memory" {
                        let mut cache = MEMORY_CACHE.lock().unwrap();
                        cache.insert(username.to_string(), (stats.clone(), now));
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
                        cache_map.insert(username.to_string(), (stats.clone(), now));
                        let file_cache = FileCache(cache_map);
                        if let Ok(mut file) = std::fs::File::create(&cache_file_path) {
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
            let template = ContributionStatsTemplate { stats: &stats, show_quartiles };
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
    let primary_color = params.get("fg").cloned().unwrap_or_else(|| "#40c463".to_string());
    let bg_color = params.get("bg").cloned().unwrap_or_else(|| "#ebedf0".to_string());
    let svg_height = params.get("svg_height").cloned().unwrap_or_else(|| "110".to_string());
    match get_stats(&username).await {
        Ok(stats) => {
            let data = prepare_graph_template_data(&stats, primary_color, bg_color);
            let template = ContributionSvgGraphTemplate {
                stats: &stats,
                max_count: data.max_count,
                cells: data.cells,
                show_months: data.show_months,
                svg_height,
                show_weekdays: data.show_weekdays,
                primary_color: data.primary_color,
                color_shades: data.color_shades,
                month_labels: data.month_labels,
                weekday_labels: data.weekday_labels,
                cell_radius: data.cell_radius,
            };
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
    let primary_color = params.get("fg").cloned().unwrap_or_else(|| "#40c463".to_string());
    let bg_color = params.get("bg").cloned().unwrap_or_else(|| "#ebedf0".to_string());
    let svg_height = params.get("svg_height").cloned().unwrap_or_else(|| "110".to_string());
    match get_stats(&username).await {
        Ok(stats) => {
            let data = prepare_graph_template_data(&stats, primary_color, bg_color);
            let template = ContributionGraphHtmlTemplate {
                stats: &stats,
                max_count: data.max_count,
                cells: data.cells,
                show_months: data.show_months,
                svg_height,
                show_weekdays: data.show_weekdays,
                primary_color: data.primary_color,
                color_shades: data.color_shades,
                month_labels: data.month_labels,
                weekday_labels: data.weekday_labels,
                cell_radius: data.cell_radius,
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
