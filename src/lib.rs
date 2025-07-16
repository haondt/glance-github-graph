use serde::Deserialize;
use anyhow::{Result, anyhow};
use scraper::{Html, Selector};
use std::collections::HashMap;
use log::{info, error};

pub mod api;
pub mod color;
pub mod config;
pub mod templates;

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct ContributionStats {
    pub username: String,
    pub today: u32,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub high_score: HighScore,
    pub quartiles: [u32; 5],
    pub daily_contributions: Vec<(String, u32, String)>, // (date, count, label)
    pub yearly_contributions: String,
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct HighScore {
    pub score: u32,
    pub date: String,
}

pub async fn fetch_contribution_stats(username: &str, _github_url: Option<&str>) -> Result<ContributionStats> {
    let url = format!("https://github.com/users/{}/contributions", username);
    info!("Fetching contributions for user '{}' from {}", username, url);
    let body = match reqwest::get(&url).await {
        Ok(resp) => {
            info!("Successfully fetched page for user '{}'.", username);
            match resp.text().await {
                Ok(text) => text,
                Err(e) => {
                    error!("Failed to read response text for user '{}': {}", username, e);
                    return Err(anyhow!("Failed to read response text: {}", e));
                }
            }
        },
        Err(e) => {
            error!("Failed to fetch page for user '{}': {}", username, e);
            return Err(anyhow!("Failed to fetch page: {}", e));
        }
    };
    let document = Html::parse_document(&body);

    // Build a map from td id to tooltip text
    let tooltip_selector = Selector::parse("tool-tip").unwrap();
    let mut tooltip_map = HashMap::new();
    for tooltip in document.select(&tooltip_selector) {
        if let Some(for_id) = tooltip.value().attr("for") {
            let text = tooltip.text().collect::<String>().trim().to_string();
            tooltip_map.insert(for_id.to_string(), text);
        }
    }

    // Parse yearly contributions as text from the h2 element
    let mut yearly_contributions = String::new();
    if let Ok(h2_selector) = Selector::parse("h2#js-contribution-activity-description") {
        if let Some(h2) = document.select(&h2_selector).next() {
            let text = h2.text().collect::<String>().trim().to_string();
            if let Some(num) = text.split_whitespace().next() {
                yearly_contributions = num.to_string();
            }
        }
    }

    let td_selector = Selector::parse("td.ContributionCalendar-day").unwrap();
    let mut contributions: Vec<(String, u32, String)> = Vec::new();
    let mut high_score = 0;
    let mut high_score_date = String::new();
    for td in document.select(&td_selector) {
        let date = td.value().attr("data-date").unwrap_or("").to_string();
        let id = td.value().attr("id").unwrap_or("");
        let tooltip_text = tooltip_map.get(id);
        let count = tooltip_text
            .and_then(|text| parse_contribution_count(text))
            .unwrap_or(0);
        let label = tooltip_text.cloned().unwrap_or_default();
        if count > high_score {
            high_score = count;
            high_score_date = date.clone();
        }
        if !date.is_empty() {
            contributions.push((date, count, label));
        }
    }
    if contributions.is_empty() {
        error!("No contributions found for user {}", username);
        return Err(anyhow!("No contributions found for user {}", username));
    }
    // Sort by date string (alphabetically, which works for YYYY-MM-DD)
    contributions.sort_by(|a, b| a.0.cmp(&b.0));
    let counts: Vec<u32> = contributions.iter().map(|(_, c, _)| *c).collect();
    // Calculate quartiles
    let mut sorted = counts.clone();
    sorted.sort();
    let n = sorted.len();
    let quartiles = [
        *sorted.get(0).unwrap_or(&0),
        *sorted.get(n / 4).unwrap_or(&0),
        *sorted.get(n / 2).unwrap_or(&0),
        *sorted.get(3 * n / 4).unwrap_or(&0),
        *sorted.last().unwrap_or(&0),
    ];
    // Calculate streaks
    let mut current_streak = 0;
    let mut longest_streak = 0;
    let mut streak = 0;
    for &count in counts.iter().rev() {
        if count > 0 {
            streak += 1;
            if streak > longest_streak {
                longest_streak = streak;
            }
        } else {
            if current_streak == 0 {
                current_streak = streak;
            }
            streak = 0;
        }
    }
    if current_streak == 0 {
        current_streak = streak;
    }
    let today = *counts.last().unwrap_or(&0);
    Ok(ContributionStats {
        username: username.to_string(),
        today,
        current_streak,
        longest_streak,
        high_score: HighScore { score: high_score, date: high_score_date },
        quartiles,
        daily_contributions: contributions,
        yearly_contributions,
    })
}

fn parse_contribution_count(text: &str) -> Option<u32> {
    // Examples: "No contributions on July 14th.", "7 contributions on September 1st.", "1 contribution on November 3rd."
    if text.starts_with("No contributions") {
        Some(0)
    } else {
        text.split_whitespace().next()?.parse::<u32>().ok()
    }
}
