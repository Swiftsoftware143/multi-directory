//! Scraper Engine — BL18, BL19, BL21, BL24
//! Modular scraping system for business directories, chambers, and phone-image capture.
//! Each scraper is a separate module implementing the Scraper trait.
//! Results flow through the pipeline at /pipeline/ingest for dedup/merge.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Scraper Configuration ──

#[derive(Debug, Deserialize)]
pub struct ScraperConfig {
    pub source: String,           // "nextdoor", "chamber", "yellowpages", "manta", "brightlocal", etc.
    pub directory_id: Option<String>,
    pub location: Option<String>,  // city, state or zip
    pub query: Option<String>,     // category or keyword filter
    pub max_results: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ScraperResult {
    pub source: String,
    pub businesses: Vec<ScrapedBusiness>,
    pub total_found: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ScrapedBusiness {
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub category: Option<String>,
    pub rating: Option<f64>,
    pub review_count: Option<i32>,
    pub image_urls: Vec<String>,
    pub source_url: Option<String>,
}

// ── Google Places Scraper (already has API in provider_keys) ──

/// GET /api/v1/scraper/providers — list available scraper sources and their key status
pub async fn list_scraper_providers(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let providers = sqlx::query_as::<_, (String, String, bool)>(
        r#"SELECT ap.key, ap.name, pk.api_key IS NOT NULL as has_key
           FROM available_providers ap
           LEFT JOIN provider_keys pk ON pk.provider = ap.key AND pk.tenant_id = '00000000-0000-0000-0000-000000000000'
           WHERE ap.key IN ('google_places','brightlocal','yext','uberall')
           ORDER BY ap.name"#
    )
    .fetch_all(&s.db)
    .await?;

    let result: Vec<serde_json::Value> = providers.into_iter().map(|(key, name, has_key)| json!({
        "key": key, "name": name, "configured": has_key
    })).collect();

    Ok(Json(json!({"providers": result, "total": result.len()})))
}

/// POST /api/v1/scraper/run — execute a scraper job (admin only)
pub async fn run_scraper(
    State(s): State<AppState>,
    Json(cfg): Json<ScraperConfig>,
) -> ApiResult<impl IntoResponse> {
    // Validate source
    let valid_sources = ["google_places", "brightlocal", "nextdoor", "chamber", "yellowpages", "manta"];
    if !valid_sources.contains(&cfg.source.as_str()) {
        return Err(AppError::BadRequest(format!("Unknown scraper source: {}", cfg.source)));
    }

    // Get API key for the source
    let api_key: Option<String> = sqlx::query_scalar(
        "SELECT api_key FROM provider_keys WHERE provider = $1 AND tenant_id = '00000000-0000-0000-0000-000000000000' LIMIT 1"
    )
    .bind(&cfg.source)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    // For now, return available sources + key status
    // Actual scraping logic is source-specific and will be added per-source
    Ok(Json(json!({
        "source": cfg.source,
        "configured": api_key.is_some(),
        "message": format!("Scraper '{}' is {}. Provide API key in Settings > API Keys.", 
            cfg.source, if api_key.is_some() { "configured" } else { "not configured" }),
        "note": "Full scraping pipeline will stream results through /pipeline/ingest"
    })))
}

/// POST /api/v1/scraper/google-places — run Google Places scraper
pub async fn scrape_google_places(
    State(s): State<AppState>,
    Json(cfg): Json<ScraperConfig>,
) -> ApiResult<impl IntoResponse> {
    // Get Google Places API key
    let api_key = sqlx::query_scalar::<_, String>(
        "SELECT api_key FROM provider_keys WHERE provider = 'google_places' AND tenant_id = '00000000-0000-0000-0000-000000000000' LIMIT 1"
    )
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("Google Places API key not configured. Add it in Settings > API Keys.".into()))?;

    let location = cfg.location.as_deref().unwrap_or("Tampa, FL");
    let query = cfg.query.as_deref().unwrap_or("restaurants");
    let max_results = cfg.max_results.unwrap_or(20).min(60);

    // Call Google Places API
    let url = format!(
        "https://maps.googleapis.com/maps/api/place/textsearch/json?query={}+in+{}&key={}&maxresults={}",
        urlencoding_encode(query),
        urlencoding_encode(location),
        api_key,
        max_results
    );

    let resp = reqwest::get(&url).await
        .map_err(|e| AppError::BadRequest(format!("Google Places API error: {}", e)))?
        .json::<serde_json::Value>().await
        .map_err(|e| AppError::BadRequest(format!("Failed to parse response: {}", e)))?;

    let results = resp.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut businesses = Vec::new();

    for place in &results {
        let name = place.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
        let formatted_addr = place.get("formatted_address").and_then(|v| v.as_str()).unwrap_or("");
        let parts: Vec<&str> = formatted_addr.split(',').collect();
        let city = parts.get(0).map(|s| s.trim().to_string());
        let state_zip = parts.get(1).map(|s| s.trim().to_string());

        businesses.push(ScrapedBusiness {
            name,
            phone: place.get("formatted_phone_number").and_then(|v| v.as_str()).map(|s| s.to_string()),
            email: None,
            website: place.get("website").and_then(|v| v.as_str()).map(|s| s.to_string()),
            address: Some(formatted_addr.to_string()),
            city,
            state: state_zip,
            zip: None,
            category: place.get("types").and_then(|t| t.as_array())
                .and_then(|a| a.first()).and_then(|v| v.as_str()).map(|s| s.to_string()),
            rating: place.get("rating").and_then(|v| v.as_f64()),
            review_count: place.get("user_ratings_total").and_then(|v| v.as_i64()).map(|i| i as i32),
            image_urls: vec![],
            source_url: place.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
        });
    }

    let total = businesses.len();
    Ok(Json(json!(ScraperResult {
        source: "google_places".into(),
        businesses,
        total_found: total,
        errors: vec![],
    })))
}

fn urlencoding_encode(s: &str) -> String {
    s.split(' ').collect::<Vec<_>>().join("+")
}
