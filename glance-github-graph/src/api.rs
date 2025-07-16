use actix_web::{web, App, HttpServer, Responder, HttpResponse, HttpRequest};
use crate::fetch_contribution_stats;
use std::env;
use std::sync::Mutex;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use lazy_static::lazy_static;
use serde::{Serialize, Deserialize};
use askama::Template;

lazy_static! {
    static ref MEMORY_CACHE: Mutex<HashMap<String, (crate::ContributionStats, u64)>> = Mutex::new(HashMap::new());
}

#[derive(Serialize, Deserialize)]
struct FileCache(HashMap<String, (crate::ContributionStats, u64)>);

#[derive(Template)]
#[template(path = "stats.html")]
pub struct ContributionStatsTemplate<'a> {
    pub stats: &'a crate::ContributionStats,
}

impl<'a> ContributionStatsTemplate<'a> {
    pub fn quartiles_display(&self) -> String {
        self.stats.quartiles.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Template)]
#[template(path = "svg_graph.svg")]
pub struct ContributionSvgGraphTemplate<'a> {
    pub stats: &'a crate::ContributionStats,
    pub max_count: u32,
    pub cells: Vec<GraphCell>,
    pub show_months: bool,
    pub show_weekdays: bool,
    pub primary_color: String,
    pub color_shades: Vec<String>,
    pub month_labels: Vec<(usize, String)>,
    pub weekday_labels: Vec<(usize, &'static str)>,
    pub cell_radius: u32,
}

pub struct GraphCell {
    pub date: String,
    pub count: u32,
    pub col: usize,
    pub row: usize,
    pub color: String,
    pub hover_text: String,
}

pub async fn run_api_server() -> std::io::Result<()> {
    let cache_enabled = std::env::var("CACHE_ENABLED").unwrap_or_else(|_| "false".to_string()) == "true";
    let cache_type = std::env::var("CACHE_TYPE").unwrap_or_else(|_| "memory".to_string());
    let cache_duration_secs: u64 = std::env::var("CACHE_DURATION_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3600);

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
            .route("/graph/{username}", web::get().to(|path, req| svg_graph_handler(path, req)))
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

async fn stats_handler(path: web::Path<String>) -> impl Responder {
    let username = path.into_inner();
    match get_stats(&username).await {
        Ok(stats) => {
            let template = ContributionStatsTemplate { stats: &stats };
            match template.render() {
                Ok(body) => HttpResponse::Ok()
                    .content_type("text/html")
                    .insert_header(("Widget-Content-Type", "html"))
                    .body(body),
                Err(e) => HttpResponse::InternalServerError().body(format!("Template error: {}", e)),
            }
        },
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}


async fn svg_graph_handler(path: web::Path<String>, req: HttpRequest) -> impl Responder {
    let username = path.into_inner();
    // Get color params from URL
    let query = req.query_string();
    let params: HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let primary_color = params.get("fg").cloned()
        .unwrap_or_else(|| "#40c463".to_string());
    let bg_color = params.get("bg").cloned()
        .unwrap_or_else(|| "#ebedf0".to_string());
    match get_stats(&username).await {
        Ok(stats) => {
            let max_count = stats.daily_contributions.iter().map(|(_, c, _)| *c).max().unwrap_or(0);
            let max_rows = 7;
            let show_months = std::env::var("GRAPH_SHOW_MONTHS").unwrap_or_else(|_| "true".to_string()) == "true";
            let show_weekdays = std::env::var("GRAPH_SHOW_WEEKDAYS").unwrap_or_else(|_| "true".to_string()) == "true";
            let color_shades = derive_color_shades_with_bg(&primary_color, &bg_color);
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
            let template = ContributionSvgGraphTemplate {
                stats: &stats,
                max_count,
                cells,
                show_months,
                show_weekdays,
                primary_color,
                color_shades,
                month_labels,
                weekday_labels,
                cell_radius,
            };
            match template.render() {
                Ok(body) => HttpResponse::Ok()
                    .content_type("image/svg+xml")
                    .insert_header(("Widget-Content-Type", "html"))
                    .body(body),
                Err(e) => HttpResponse::InternalServerError().body(format!("Template error: {}", e)),
            }
        },
        Err(e) => HttpResponse::InternalServerError().body(e),
    }
}

fn hsl_string(h: f32, s: f32, l: f32) -> String {
    format!("hsl({:.0}, {:.0}%, {:.0}%)", h, s * 100.0, l * 100.0)
}

fn hex_to_hsl(hex: &str) -> Result<(f32, f32, f32), ()> {
    let (r, g, b) = hex_to_rgb(hex)?;
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, s);
    if d == 0.0 {
        h = 0.0;
        s = 0.0;
    } else {
        s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } / 6.0;
    }
    Ok((h * 360.0, s, l))
}



fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), ()> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(r) = u8::from_str_radix(&hex[0..2], 16) {
            if let Ok(g) = u8::from_str_radix(&hex[2..4], 16) {
                if let Ok(b) = u8::from_str_radix(&hex[4..6], 16) {
                    return Ok((r, g, b));
                }
            }
        }
    }
    Err(())
} 

fn derive_color_shades_with_bg(primary: &str, bg_color: &str) -> Vec<String> {
    if let (Ok((h1, s1, l1)), Ok((h2, s2, l2))) = (hex_to_hsl(bg_color), hex_to_hsl(primary)) {
        let steps = 5;
        (0..steps)
            .map(|i| {
                let t = i as f32 / (steps - 1) as f32;
                let h = if i == 0 { h1 } else { h2 };
                let s = s1 + (s2 - s1) * t;
                let l = l1 + (l2 - l1) * t;
                hsl_string(h, s, l)
            })
            .collect()
    } else {
        vec![
            bg_color.to_string(),
            "hsl(0, 0%, 70%)".to_string(),
            "hsl(0, 0%, 50%)".to_string(),
            "hsl(0, 0%, 35%)".to_string(),
            "hsl(0, 0%, 20%)".to_string(),
        ]
    }
} 
