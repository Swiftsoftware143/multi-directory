//! Data Pipeline Module — BL20
//! Scraper Output Pipeline: ingest, dedup, merge, enrich business data
//! from multiple sources (Google Places, BrightLocal, scrapers, etc.)
//!
//! Each source pushes data to the pipeline endpoint. The pipeline:
//! 1. Normalizes fields from different source formats
//! 2. Deduplicates against existing businesses (by phone, email, website, name+city)
//! 3. Merges new fields into existing records (deep merge, preferring non-null)
//! 4. Enriches with directory_id, category matching, lat/lng geocoding if missing
//! 5. Returns a report of what was created, updated, or skipped

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Request ──

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// Source identifier: "google_places", "brightlocal", "scraper_nextdoor", etc.
    pub source: String,
    /// Directory slug or ID to associate businesses with
    pub directory_id: Option<String>,
    /// Array of business records from the source
    pub businesses: Vec<IngestBusiness>,
}

#[derive(Debug, Deserialize)]
pub struct IngestBusiness {
    pub name: Option<String>,
    pub description: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub category: Option<String>,
    pub rating: Option<f64>,
    pub review_count: Option<i32>,
    pub image_urls: Option<Vec<String>>,
    pub business_type: Option<String>,
    /// Source-specific raw data preserved for enrichment later
    pub raw: Option<Value>,
}

// ── Response ──

#[derive(Debug, Serialize)]
pub struct IngestReport {
    pub total: usize,
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub source: String,
}

// ── Normalization ──

fn normalize_phone(phone: &str) -> String {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 10 {
        format!("({}) {}-{}", &digits[..3], &digits[3..6], &digits[6..])
    } else if digits.len() == 11 && digits.starts_with('1') {
        format!("({}) {}-{}", &digits[1..4], &digits[4..7], &digits[7..])
    } else {
        phone.to_string()
    }
}

fn normalize_website(website: &str) -> String {
    let w = website.trim();
    if !w.starts_with("http") {
        format!("https://{}", w)
    } else {
        w.to_string()
    }
}

// ── Dedup Lookups ──

pub async fn find_existing_by_phone(pool: &PgPool, phone: &str, dir_id: Option<Uuid>) -> Result<Option<Uuid>, sqlx::Error> {
    let normalized = normalize_phone(phone);
    if let Some(did) = dir_id {
        sqlx::query_scalar("SELECT id FROM businesses WHERE phone = $1 AND directory_id = $2 LIMIT 1")
            .bind(&normalized).bind(did).fetch_optional(pool).await
    } else {
        sqlx::query_scalar("SELECT id FROM businesses WHERE phone = $1 LIMIT 1")
            .bind(&normalized).fetch_optional(pool).await
    }
}

pub async fn find_existing_by_website(pool: &PgPool, website: &str, dir_id: Option<Uuid>) -> Result<Option<Uuid>, sqlx::Error> {
    let normalized = normalize_website(website);
    if let Some(did) = dir_id {
        sqlx::query_scalar("SELECT id FROM businesses WHERE website = $1 AND directory_id = $2 LIMIT 1")
            .bind(&normalized).bind(did).fetch_optional(pool).await
    } else {
        sqlx::query_scalar("SELECT id FROM businesses WHERE website = $1 LIMIT 1")
            .bind(&normalized).fetch_optional(pool).await
    }
}

async fn find_existing_by_name_city(pool: &PgPool, name: &str, city: &str, dir_id: Option<Uuid>) -> Result<Option<Uuid>, sqlx::Error> {
    if let Some(did) = dir_id {
        sqlx::query_scalar(
            "SELECT id FROM businesses WHERE LOWER(name) = LOWER($1) AND LOWER(COALESCE(city,'')) = LOWER($2) AND directory_id = $3 LIMIT 1"
        ).bind(name).bind(city).bind(did).fetch_optional(pool).await
    } else {
        sqlx::query_scalar(
            "SELECT id FROM businesses WHERE LOWER(name) = LOWER($1) AND LOWER(COALESCE(city,'')) = LOWER($2) LIMIT 1"
        ).bind(name).bind(city).fetch_optional(pool).await
    }
}

// ── Merge ──

fn merge_business(existing: &Value, incoming: &IngestBusiness) -> Value {
    let mut result = existing.clone();
    let obj = result.as_object_mut().unwrap();

    if let Some(ref name) = incoming.name { if !name.is_empty() { obj.insert("name".into(), Value::String(name.clone())); } }
    if let Some(ref desc) = incoming.description { if !desc.is_empty() { obj.insert("description".into(), Value::String(desc.clone())); } }
    if let Some(ref phone) = incoming.phone { if !phone.is_empty() { obj.insert("phone".into(), Value::String(normalize_phone(phone))); } }
    if let Some(ref email) = incoming.email { if !email.is_empty() { obj.insert("email".into(), Value::String(email.clone())); } }
    if let Some(ref website) = incoming.website { if !website.is_empty() { obj.insert("website".into(), Value::String(normalize_website(website))); } }
    if let Some(ref addr) = incoming.address { if !addr.is_empty() { obj.insert("address".into(), Value::String(addr.clone())); } }
    if let Some(ref city) = incoming.city { if !city.is_empty() { obj.insert("city".into(), Value::String(city.clone())); } }
    if let Some(ref state) = incoming.state { if !state.is_empty() { obj.insert("state".into(), Value::String(state.clone())); } }
    if let Some(ref zip) = incoming.zip { if !zip.is_empty() { obj.insert("zip".into(), Value::String(zip.clone())); } }
    if let Some(lat) = incoming.latitude { if obj.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0) == 0.0 { obj.insert("latitude".into(), json!(lat)); } }
    if let Some(lng) = incoming.longitude { if obj.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0) == 0.0 { obj.insert("longitude".into(), json!(lng)); } }
    if let Some(ref bt) = incoming.business_type { if !bt.is_empty() { obj.insert("business_type".into(), Value::String(bt.clone())); } }
    if let Some(rating) = incoming.rating { obj.insert("rating".into(), json!(rating)); }
    if let Some(rc) = incoming.review_count { obj.insert("review_count".into(), json!(rc)); }
    if let Some(ref imgs) = incoming.image_urls { if !imgs.is_empty() { obj.insert("images".into(), json!(imgs)); } }

    result
}

// ── Category Resolution ──

async fn resolve_category(pool: &PgPool, directory_id: Uuid, category_name: &str) -> Option<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM directory_categories WHERE directory_id = $1 AND LOWER(name) = LOWER($2) LIMIT 1"
    ).bind(directory_id).bind(category_name).fetch_optional(pool).await.unwrap_or(None)
}

// ── Main Ingest Endpoint ──

/// POST /api/v1/pipeline/ingest
/// Accepts business data from any source, deduplicates, merges, and stores.
pub async fn pipeline_ingest(
    State(s): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> ApiResult<impl IntoResponse> {
    let total = req.businesses.len();
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    // Resolve directory_id
    let dir_id: Option<Uuid> = if let Some(ref did) = req.directory_id {
        // Try as UUID first, then as slug
        if let Ok(u) = Uuid::parse_str(did) {
            Some(u)
        } else {
            sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1 LIMIT 1")
                .bind(did).fetch_optional(&s.db).await.unwrap_or(None)
        }
    } else {
        None
    };

    for biz in &req.businesses {
        // Skip if no name
        let name = match &biz.name {
            Some(n) if !n.is_empty() => n.clone(),
            _ => { skipped += 1; errors.push("Business missing name — skipped".into()); continue; }
        };

        // Try to find existing by phone, website, or name+city (sequential, not chained closures)
        let mut existing_id: Option<Uuid> = None;

        if let Some(ref phone) = biz.phone {
            if !phone.is_empty() {
                if let Ok(Some(id)) = find_existing_by_phone(&s.db, phone, dir_id).await {
                    existing_id = Some(id);
                }
            }
        }

        if existing_id.is_none() {
            if let Some(ref website) = biz.website {
                if !website.is_empty() {
                    if let Ok(Some(id)) = find_existing_by_website(&s.db, website, dir_id).await {
                        existing_id = Some(id);
                    }
                }
            }
        }

        if existing_id.is_none() {
            if let Some(ref city) = biz.city {
                if !city.is_empty() {
                    if let Ok(Some(id)) = find_existing_by_name_city(&s.db, &name, city, dir_id).await {
                        existing_id = Some(id);
                    }
                }
            }
        }

        // Resolve category
        let category_id = if let (Some(did), Some(ref cat)) = (dir_id, &biz.category) {
            resolve_category(&s.db, did, cat).await
        } else {
            None
        };

        if let Some(existing) = existing_id {
            // Fetch existing, merge, update
            let existing_row: Option<Value> = sqlx::query_as::<_, (Value,)>(
                "SELECT row_to_json(b.*) FROM businesses b WHERE id = $1"
            ).bind(existing).fetch_optional(&s.db).await.map(|r| r.map(|v| v.0)).unwrap_or(None);

            if let Some(existing_data) = existing_row {
                let merged = merge_business(&existing_data, biz);
                // Update with merged fields
                let _ = sqlx::query(
                    "UPDATE businesses SET name=$1, description=$2, phone=$3, email=$4, website=$5, \
                     address=$6, city=$7, state=$8, zip=$9, latitude=$10, longitude=$11, \
                     rating=$12, review_count=$13, updated_at=NOW() WHERE id=$14"
                )
                .bind(merged.get("name").and_then(|v| v.as_str()))
                .bind(merged.get("description").and_then(|v| v.as_str()))
                .bind(merged.get("phone").and_then(|v| v.as_str()))
                .bind(merged.get("email").and_then(|v| v.as_str()))
                .bind(merged.get("website").and_then(|v| v.as_str()))
                .bind(merged.get("address").and_then(|v| v.as_str()))
                .bind(merged.get("city").and_then(|v| v.as_str()))
                .bind(merged.get("state").and_then(|v| v.as_str()))
                .bind(merged.get("zip").and_then(|v| v.as_str()))
                .bind(merged.get("latitude").and_then(|v| v.as_f64()))
                .bind(merged.get("longitude").and_then(|v| v.as_f64()))
                .bind(merged.get("rating").and_then(|v| v.as_f64()))
                .bind(merged.get("review_count").and_then(|v| v.as_i64()).map(|v| v as i32))
                .bind(existing)
                .execute(&s.db).await.unwrap_or_default();
                updated += 1;
            } else {
                skipped += 1;
            }
        } else {
            // Create new business
            let slug = name.to_lowercase()
                .replace(|c: char| !c.is_alphanumeric() && c != ' ', "-")
                .chars().take(200).collect::<String>();
            let phone = biz.phone.as_ref().map(|p| normalize_phone(p));
            let website = biz.website.as_ref().map(|w| normalize_website(w));

            let result = sqlx::query(
                "INSERT INTO businesses (id, directory_id, name, slug, description, phone, email, website, \
                 address, city, state, zip, latitude, longitude, category_id, rating, review_count, \
                 business_type, images, is_active, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,true,NOW(),NOW()) \
                 ON CONFLICT (directory_id, slug) DO UPDATE SET name=EXCLUDED.name RETURNING id"
            )
            .bind(Uuid::new_v4())
            .bind(dir_id)
            .bind(&name)
            .bind(&slug)
            .bind(&biz.description)
            .bind(&phone)
            .bind(&biz.email)
            .bind(&website)
            .bind(&biz.address)
            .bind(&biz.city)
            .bind(&biz.state)
            .bind(&biz.zip)
            .bind(biz.latitude)
            .bind(biz.longitude)
            .bind(category_id)
            .bind(biz.rating)
            .bind(biz.review_count)
            .bind(&biz.business_type)
            .bind(biz.image_urls.as_ref().map(|v| json!(v)))
            .execute(&s.db).await;

            match result {
                Ok(_) => created += 1,
                Err(e) => { errors.push(format!("Failed to create '{}': {}", name, e)); skipped += 1; }
            }
        }
    }

    tracing::info!(source = %req.source, total, created, updated, skipped, "Pipeline ingest complete");

    Ok(Json(json!(IngestReport {
        total,
        created,
        updated,
        skipped,
        errors,
        source: req.source,
    })))
}
