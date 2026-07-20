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
use crate::handlers::pipeline::{pipeline_ingest, find_existing_by_phone, find_existing_by_website, IngestBusiness, IngestRequest};

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


/// POST /api/v1/scraper/import — unified data import endpoint
/// Admin selects source, business type, location, and keyword.
/// Returns matched/mergeable results for review before committing.

#[derive(Debug, Deserialize)]
pub struct DataImportRequest {
    /// Source: "google_places", "brightlocal", "yext", "uberall", "chamber", "nextdoor", "yellowpages", "manta"
    pub source: String,
    /// "business" or "supplier" — determines business_type tag
    pub business_type: String,
    pub location: String,
    pub keyword: Option<String>,
    pub directory_id: Option<String>,
    pub max_results: Option<i32>,
    /// If true, import directly without review
    pub auto_import: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ImportReviewResult {
    pub source: String,
    pub business_type: String,
    pub location: String,
    pub keyword: String,
    pub businesses: Vec<ImportReviewBusiness>,
    pub total_found: usize,
}

#[derive(Debug, Serialize)]
pub struct ImportReviewBusiness {
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
    /// If a match exists in the DB, show the existing business info
    pub existing_match: Option<ExistingMatch>,
    /// Whether this would be a create or merge
    pub action: String, // "create", "merge", "duplicate"
}

#[derive(Debug, Serialize)]
pub struct ExistingMatch {
    pub id: String,
    pub name: String,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub fields_to_merge: Vec<String>,
}

/// Unified data import endpoint
pub async fn data_import(
    State(s): State<AppState>,
    Json(req): Json<DataImportRequest>,
) -> ApiResult<impl IntoResponse> {
    // Route to the appropriate source handler
    match req.source.as_str() {
        "google_places" => google_places_import(State(s), req).await,
        _ => Err(AppError::BadRequest(format!("Source '{}' not yet implemented. Available: google_places", req.source)))
    }
}

async fn google_places_import(
    s: State<AppState>,
    req: DataImportRequest,
) -> ApiResult<impl IntoResponse> {
    // Get Google Places API key
    let api_key = sqlx::query_scalar::<_, String>(
        "SELECT api_key FROM provider_keys WHERE provider = 'google_places' AND tenant_id = '00000000-0000-0000-0000-000000000000' LIMIT 1"
    )
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("Google Places API key not configured. Add it in Settings > API Keys.".into()))?;

    let query = match (&req.keyword, req.business_type.as_str()) {
        (Some(kw), _) if !kw.is_empty() => format!("{} in {}", kw, req.location),
        (_, "supplier") => format!("wholesale suppliers distributors in {}", req.location),
        (_, _) => format!("businesses in {}", req.location),
    };

    let max_results = req.max_results.unwrap_or(20).min(60);
    let url = format!(
        "https://maps.googleapis.com/maps/api/place/textsearch/json?query={}&key={}&maxresults={}",
        urlencoding_encode(&query),
        api_key,
        max_results
    );

    let resp = reqwest::get(&url).await
        .map_err(|e| AppError::BadRequest(format!("Google Places API error: {}", e)))?
        .json::<serde_json::Value>().await
        .map_err(|e| AppError::BadRequest(format!("Failed to parse response: {}", e)))?;

    let results = resp.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut businesses = Vec::new();

    // Resolve directory_id
    let dir_id = if let Some(ref did) = req.directory_id {
        if let Ok(u) = Uuid::parse_str(did) {
            Some(u)
        } else {
            sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1 LIMIT 1")
                .bind(did).fetch_optional(&s.db).await.unwrap_or(None)
        }
    } else { None };

    for place in &results {
        let name = place.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
        let formatted_addr = place.get("formatted_address").and_then(|v| v.as_str()).unwrap_or("");
        let addr_parts: Vec<&str> = formatted_addr.split(',').collect();
        let city = addr_parts.first().map(|s| s.trim().to_string());
        let rest = addr_parts.get(1).map(|s| s.trim().to_string());

        // Check for existing match by phone, website, or name+city
        let phone = place.get("formatted_phone_number").and_then(|v| v.as_str()).map(|s| s.to_string());
        let website = place.get("website").and_then(|v| v.as_str()).map(|s| s.to_string());
        let mut existing_match = None;
        let mut action = "create".to_string();

        if let Some(ref p) = phone {
            if let Ok(Some(id)) = find_existing_by_phone(&s.db, p, dir_id).await {
                let existing_name: Option<String> = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
                    .bind(id).fetch_optional(&s.db).await.unwrap_or(None);
                existing_match = Some(ExistingMatch {
                    id: id.to_string(),
                    name: existing_name.unwrap_or_default(),
                    phone: phone.clone(),
                    website: website.clone(),
                    fields_to_merge: vec!["description".into(), "website".into(), "rating".into(), "review_count".into(), "images".into()],
                });
                action = "merge".to_string();
            }
        }

        if existing_match.is_none() {
            if let Some(ref w) = website {
                if let Ok(Some(id)) = find_existing_by_website(&s.db, w, dir_id).await {
                    let existing_name: Option<String> = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
                        .bind(id).fetch_optional(&s.db).await.unwrap_or(None);
                    existing_match = Some(ExistingMatch {
                        id: id.to_string(),
                        name: existing_name.unwrap_or_default(),
                        phone: phone.clone(),
                        website: website.clone(),
                        fields_to_merge: vec!["phone".into(), "description".into(), "rating".into()],
                    });
                    action = "merge".to_string();
                }
            }
        }

        let category = place.get("types").and_then(|t| t.as_array())
            .and_then(|a| a.first()).and_then(|v| v.as_str()).map(|s| s.to_string());

        businesses.push(ImportReviewBusiness {
            name,
            phone,
            email: None,
            website,
            address: Some(formatted_addr.to_string()),
            city,
            state: rest,
            zip: None,
            category,
            rating: place.get("rating").and_then(|v| v.as_f64()),
            review_count: place.get("user_ratings_total").and_then(|v| v.as_i64()).map(|i| i as i32),
            image_urls: vec![],
            existing_match,
            action,
        });
    }

    // If auto_import, push through pipeline
    if req.auto_import.unwrap_or(false) && !businesses.is_empty() {
        let ingest_req = crate::handlers::pipeline::IngestRequest {
            source: "google_places".into(),
            directory_id: req.directory_id.clone(),
            businesses: businesses.iter().map(|b| crate::handlers::pipeline::IngestBusiness {
                name: Some(b.name.clone()),
                description: None,
                phone: b.phone.clone(),
                email: b.email.clone(),
                website: b.website.clone(),
                address: b.address.clone(),
                city: b.city.clone(),
                state: b.state.clone(),
                zip: b.zip.clone(),
                latitude: None,
                longitude: None,
                category: b.category.clone(),
                rating: b.rating,
                review_count: b.review_count,
                image_urls: None,
                business_type: if req.business_type == "supplier" { Some("supplier".into()) } else { None },
                raw: None,
            }).collect(),
        };
        // Fire and forget — results go through pipeline
        let _ = pipeline_ingest(s, Json(ingest_req)).await;
    }

    Ok(Json(json!(ImportReviewResult {
        source: "google_places".into(),
        business_type: req.business_type,
        location: req.location,
        keyword: query,
        total_found: businesses.len(),
        businesses,
    })))
}

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
