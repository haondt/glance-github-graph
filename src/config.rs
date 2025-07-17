use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub cache_enabled: bool,
    pub cache_type: String,
    pub cache_duration_secs: u64,
    pub cache_file_path: String,
    pub default_fg: String,
    pub default_bg: String,
    pub default_svg_height: String,
    pub default_show_months: bool,
    pub default_show_weekdays: bool,
    pub cell_radius: u32,
    pub weekday_labels: Vec<(usize, &'static str)>,
    pub default_transition_hue: bool,
    pub default_font_size: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            // Cache configuration
            cache_enabled: env::var("CACHE_ENABLED")
                .unwrap_or_else(|_| "false".to_string()) == "true",
            cache_type: env::var("CACHE_TYPE")
                .unwrap_or_else(|_| "memory".to_string()),
            cache_duration_secs: env::var("CACHE_DURATION_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            cache_file_path: env::var("CACHE_FILE_PATH")
                .unwrap_or_else(|_| "cache.json".to_string()),
            default_fg: "#40c463".to_string(),
            default_bg: "#ebedf0".to_string(),
            default_svg_height: "110".to_string(),
            default_show_months: true,
            default_show_weekdays: true,
            cell_radius: 2,
            weekday_labels: vec![(1, "Mon"), (3, "Wed"), (5, "Fri")],
            default_transition_hue: false,
            default_font_size: "12".to_string(),
        }
    }
} 
