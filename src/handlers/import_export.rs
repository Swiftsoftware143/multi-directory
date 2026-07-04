//! Import/Export handlers for Multi-Directory API.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;
use sqlx::Row;

use std::collections::HashSet;
use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::handlers::data_company::{get_google_api_key, get_yelp_api_key};
use crate::models::*;

// ── Import ────────────────────────────────────────────────────────────────

pub async fn import_data(
    State(state): State<AppState>,
    Json(req): Json<ImportDataRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.data.is_empty() {
        return Err(AppError::Validation("No data provided for import".to_string()));
    }
    let valid_entities = ["businesses", "reviews", "contacts", "deals"];
    if !valid_entities.contains(&req.entity_type.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid entity_type: {}. Must be one of: {}",
            req.entity_type, valid_entities.join(", ")
        )));
    }
    let log_entry = sqlx::query_as::<_, ImportLog>(
        "INSERT INTO import_logs (entity_type, filename, rows_total, status) VALUES ($1, $2, $3, 'processing') RETURNING *"
    )
    .bind(&req.entity_type)
    .bind(format!("import-{}.json", req.entity_type))
    .bind(req.data.len() as i32)
    .fetch_one(&state.db)
    .await?;
    let mut success = 0i32;
    let mut failed = 0i32;
    let mut errors: Vec<serde_json::Value> = Vec::new();
    for (idx, row) in req.data.iter().enumerate() {
        match import_single_row(&state.db, &req.entity_type, row, req.directory_id).await {
            Ok(_) => success += 1,
            Err(e) => {
                failed += 1;
                errors.push(json!({"row": idx, "error": e.to_string(), "data": row}));
            }
        }
    }
    let final_status = if failed == 0 { "completed" } else if success == 0 { "failed" } else { "completed" };
    sqlx::query(
        "UPDATE import_logs SET rows_success = $1, rows_failed = $2, errors = $3::jsonb, status = $4 WHERE id = $5"
    )
    .bind(success).bind(failed)
    .bind(serde_json::to_value(&errors).unwrap_or(json!([])))
    .bind(final_status).bind(log_entry.id)
    .execute(&state.db).await?;
    Ok(Json(json!(ImportResult {
        import_log_id: log_entry.id,
        rows_total: req.data.len() as i32,
        rows_success: success,
        rows_failed: failed,
        errors,
        status: final_status.to_string(),
    })))
}

async fn import_single_row(
    db: &sqlx::PgPool, entity_type: &str,
    row: &serde_json::Value, directory_id: Option<Uuid>,
) -> Result<(), AppError> {
    match entity_type {
        "businesses" => import_business_row(db, row, directory_id).await,
        "reviews" => import_review_row(db, row, directory_id).await,
        "contacts" => import_contact_row(db, row, directory_id).await,
        "deals" => import_deal_row(db, row, directory_id).await,
        _ => Err(AppError::Internal(format!("Unknown entity type: {}", entity_type))),
    }
}

async fn import_business_row(
    db: &sqlx::PgPool, row: &serde_json::Value, dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let name = row.get("name").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("Missing 'name' field".to_string()))?;
    let slug = row.get("slug").and_then(|v| v.as_str()).unwrap_or(name);
    let directory_id = row.get("directory_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or(dir_id)
        .ok_or_else(|| AppError::Validation("Missing 'directory_id' field".to_string()))?;
    let address = row.get("address").and_then(|v| v.as_str()).unwrap_or("");
    let city = row.get("city").and_then(|v| v.as_str()).unwrap_or("");
    let state = row.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let zip = row.get("zip").and_then(|v| v.as_str()).unwrap_or("");
    let phone = row.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let email = row.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let website = row.get("website").and_then(|v| v.as_str()).unwrap_or("");
    let description = row.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let latitude = row.get("latitude").and_then(|v| v.as_f64());
    let longitude = row.get("longitude").and_then(|v| v.as_f64());
    let rating = row.get("rating").and_then(|v| v.as_f64());
    let category = row.get("category").and_then(|v| v.as_str()).unwrap_or("");
    let review_count = row.get("review_count").and_then(|v| v.as_i64()).map(|v| v as i32);
    
    // Handle images — merge arrays, removing duplicates
    let images: Vec<String> = row.get("images")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let images_json = serde_json::to_value(&images).unwrap_or(json!([]));
    
    // Try to find matching category_id by name
    let cat_id: Option<uuid::Uuid> = if !category.is_empty() {
        sqlx::query_scalar(
            "SELECT id FROM directory_categories WHERE directory_id = $1 AND name = $2 LIMIT 1"
        )
        .bind(directory_id)
        .bind(category)
        .fetch_optional(db)
        .await?
            .flatten()
    } else {
        None
    };
    
    sqlx::query(
        "INSERT INTO businesses (name, slug, directory_id, description, address, city, state, zip, phone, email, website, latitude, longitude, rating, review_count, category_id, images) 
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
         ON CONFLICT (directory_id, slug) DO UPDATE SET
            name = EXCLUDED.name,
            description = COALESCE(NULLIF(EXCLUDED.description, ''), businesses.description),
            address = COALESCE(NULLIF(EXCLUDED.address, ''), businesses.address),
            city = COALESCE(NULLIF(EXCLUDED.city, ''), businesses.city),
            state = COALESCE(NULLIF(EXCLUDED.state, ''), businesses.state),
            zip = COALESCE(NULLIF(EXCLUDED.zip, ''), businesses.zip),
            phone = COALESCE(NULLIF(EXCLUDED.phone, ''), businesses.phone),
            email = COALESCE(NULLIF(EXCLUDED.email, ''), businesses.email),
            website = COALESCE(NULLIF(EXCLUDED.website, ''), businesses.website),
            latitude = COALESCE(EXCLUDED.latitude, businesses.latitude),
            longitude = COALESCE(EXCLUDED.longitude, businesses.longitude),
            rating = GREATEST(COALESCE(EXCLUDED.rating, 0), COALESCE(businesses.rating, 0)),
            review_count = GREATEST(COALESCE(EXCLUDED.review_count, 0), COALESCE(businesses.review_count, 0)),
            category_id = COALESCE(EXCLUDED.category_id, businesses.category_id),
            images = (
                SELECT jsonb_agg(DISTINCT v) FROM jsonb_array_elements_text(
                    CASE WHEN businesses.images IS NULL OR businesses.images = '[]'::jsonb 
                         THEN $17::jsonb 
                         WHEN $17::jsonb = '[]'::jsonb 
                         THEN businesses.images 
                         ELSE businesses.images || $17::jsonb 
                    END
                ) AS v
            ),
            updated_at = NOW()"
    )
    .bind(name).bind(slug).bind(directory_id)
    .bind(&description)
    .bind(&address)
    .bind(&city).bind(&state).bind(&zip)
    .bind(&phone).bind(&email).bind(&website)
    .bind(latitude).bind(longitude)
    .bind(rating)
    .bind(review_count)
    .bind(cat_id)
    .bind(&images_json)
    .execute(db)
    .await?;
    Ok(())
}

async fn import_review_row(
    db: &sqlx::PgPool, row: &serde_json::Value, _dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let business_id = row.get("business_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::Validation("Missing 'business_id' field".to_string()))?;
    let rating = row.get("rating").and_then(|v| v.as_i64()).unwrap_or(5) as i32;
    let content = row.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let reviewer_name = row.get("reviewer_name").or_else(|| row.get("author")).and_then(|v| v.as_str()).unwrap_or("Anonymous");
    sqlx::query(
        "INSERT INTO reviews (business_id, rating, content, reviewer_name) VALUES ($1, $2, $3, $4)"
    )
    .bind(business_id).bind(rating).bind(content).bind(reviewer_name)
    .execute(db).await?;
    Ok(())
}

async fn import_contact_row(
    db: &sqlx::PgPool, row: &serde_json::Value, dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let first_name = row.get("first_name").and_then(|v| v.as_str()).or_else(|| row.get("name").and_then(|v| v.as_str())).unwrap_or("Unknown");
    let last_name = row.get("last_name").and_then(|v| v.as_str()).unwrap_or("");
    let email = row.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let phone = row.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let directory_id = row.get("directory_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or(dir_id);
    sqlx::query(
        "INSERT INTO crm_contacts (first_name, last_name, email, phone, directory_id) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(first_name).bind(last_name).bind(email).bind(phone).bind(directory_id)
    .execute(db).await?;
    Ok(())
}

async fn import_deal_row(
    db: &sqlx::PgPool, row: &serde_json::Value, dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let title = row.get("title").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("Missing 'title' field".to_string()))?;
    let description = row.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let directory_id = row.get("directory_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or(dir_id)
        .ok_or_else(|| AppError::Validation("Missing 'directory_id' field".to_string()))?;
    sqlx::query(
        "INSERT INTO deals (title, description, directory_id) VALUES ($1, $2, $3)"
    )
    .bind(title).bind(description).bind(directory_id)
    .execute(db).await?;
    Ok(())
}

// ── Cross-Source Enrich ───────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct EnrichRequest {
    pub name: String,
    pub directory_id: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub phone: Option<String>,
    pub source: Option<String>,  // "google", "yelp", or "both"
}

/// POST /api/v1/import/enrich — enrich a business by checking multiple sources
/// Merges the best data from Google Places + Yelp, returns a unified record.
/// Does NOT save to DB — just returns the enriched data for preview.
pub async fn enrich_business_from_sources(
    State(state): State<AppState>,
    Json(req): Json<EnrichRequest>,
) -> ApiResult<impl IntoResponse> {
    let mut result = json!({
        "name": &req.name,
        "address": req.address.as_deref().unwrap_or(""),
        "city": req.city.as_deref().unwrap_or(""),
        "state": req.state.as_deref().unwrap_or(""),
        "phone": req.phone.as_deref().unwrap_or(""),
        "website": "",
        "description": "",
        "latitude": serde_json::Value::Null,
        "longitude": serde_json::Value::Null,
        "rating": serde_json::Value::Null,
        "review_count": serde_json::Value::Null,
        "images": json!([]),
        "categories": "",
        "sources_checked": json!([]),
    });

    let mut sources_checked: Vec<String> = Vec::new();
    let source_filter = req.source.as_deref().unwrap_or("both");

    // 1. Check Google Places
    if source_filter == "google" || source_filter == "both" {
        match enrich_from_google(&state, &req).await {
            Ok(Some(data)) => {
                sources_checked.push("google".to_string());
                merge_enrichment(&mut result, data);
            }
            Ok(None) => { /* no results */ }
            Err(e) => { sources_checked.push(format!("google_error: {}", e)); }
        }
    }

    // 2. Check Yelp
    if source_filter == "yelp" || source_filter == "both" {
        match enrich_from_yelp(&state, &req).await {
            Ok(Some(data)) => {
                sources_checked.push("yelp".to_string());
                merge_enrichment(&mut result, data);
            }
            Ok(None) => { /* no results */ }
            Err(e) => { sources_checked.push(format!("yelp_error: {}", e)); }
        }
    }

    result.as_object_mut().unwrap().insert("sources_checked".to_string(), json!(sources_checked));
    Ok(Json(result))
}

async fn enrich_from_google(state: &AppState, req: &EnrichRequest) -> Result<Option<serde_json::Value>, String> {
    let api_key = get_google_api_key(state, Some(&req.directory_id)).await
        .map_err(|e| format!("Google key: {}", e))?;
    
    let query = build_search_query(req);
    let client = reqwest::Client::new();
    
    // Autocomplete
    let autocomplete_url = format!(
        "https://maps.googleapis.com/maps/api/place/autocomplete/json?input={}&key={}&types=establishment",
        urlencode(&query), &api_key
    );
    let resp = client.get(&autocomplete_url).send().await
        .map_err(|e| format!("Google autocomplete request: {}", e))?;
    let body: serde_json::Value = resp.json().await
        .map_err(|e| format!("Google autocomplete parse: {}", e))?;
    
    let predictions = body.get("predictions").and_then(|v| v.as_array());
    let place_id = predictions
        .and_then(|arr| arr.first())
        .and_then(|p| p.get("place_id").and_then(|v| v.as_str().map(|s| s.to_string())));
    
    if let Some(pid) = place_id {
        // Get place details
        let details_url = format!(
            "https://maps.googleapis.com/maps/api/place/details/json?place_id={}&key={}&fields=name,formatted_address,formatted_phone_number,website,geometry,rating,user_ratings_total,types,photos,editorial_summary",
            urlencode(&pid), &api_key
        );
        let dresp = client.get(&details_url).send().await
            .map_err(|e| format!("Google details request: {}", e))?;
        let dbody: serde_json::Value = dresp.json().await
            .map_err(|e| format!("Google details parse: {}", e))?;
        
        if let Some(d) = dbody.get("result") {
            // Build photo URLs
            let photos: Vec<String> = d.get("photos")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().filter_map(|p| {
                        let ref_ = p.get("photo_reference").and_then(|v| v.as_str())?;
                        Some(format!(
                            "https://maps.googleapis.com/maps/api/place/photo?maxwidth=800&photo_reference={}&key={}",
                            urlencode(ref_), &api_key
                        ))
                    }).collect()
                })
                .unwrap_or_default();
            
            let lat = d.get("geometry").and_then(|g| g.get("location"))
                .and_then(|l| l.get("lat").and_then(|v| v.as_f64()));
            let lng = d.get("geometry").and_then(|g| g.get("location"))
                .and_then(|l| l.get("lng").and_then(|v| v.as_f64()));
            
            let types: Vec<String> = d.get("types")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().filter_map(|t| {
                        let s = t.as_str()?;
                        if s.contains("_generic") || s.contains("_listing") 
                           || s == "point_of_interest" || s == "establishment" 
                           || s == "food" {
                            None
                        } else {
                            Some(s.to_string().replace("_", " "))
                        }
                    }).collect()
                })
                .unwrap_or_default();
            
            return Ok(Some(json!({
                "name": d.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                "address": d.get("formatted_address").and_then(|v| v.as_str()).unwrap_or(""),
                "phone": d.get("formatted_phone_number").and_then(|v| v.as_str()).unwrap_or(""),
                "website": d.get("website").and_then(|v| v.as_str()).unwrap_or(""),
                "description": d.get("editorial_summary")
                    .and_then(|e| e.get("overview").and_then(|v| v.as_str())).unwrap_or(""),
                "latitude": lat,
                "longitude": lng,
                "rating": d.get("rating").and_then(|v| v.as_f64()),
                "review_count": d.get("user_ratings_total").and_then(|v| v.as_i64()),
                "images": json!(photos),
                "categories": types.join(", "),
                "source": "google",
            })));
        }
    }
    
    Ok(None)
}

async fn enrich_from_yelp(state: &AppState, req: &EnrichRequest) -> Result<Option<serde_json::Value>, String> {
    let api_key = get_yelp_api_key(state, Some(&req.directory_id)).await
        .map_err(|e| format!("Yelp key: {}", e))?;
    
    let query = build_search_query(req);
    let location = if req.city.as_deref().unwrap_or("").is_empty() {
        req.state.as_deref().unwrap_or("US")
    } else if req.state.as_deref().unwrap_or("").is_empty() {
        req.city.as_deref().unwrap_or("US")
    } else {
        &format!("{} {}", req.city.as_deref().unwrap_or(""), req.state.as_deref().unwrap_or(""))
    };
    
    let client = reqwest::Client::new();
    let search_url = format!(
        "https://api.yelp.com/v3/businesses/search?term={}&location={}&limit=3",
        urlencode(&query), urlencode(location)
    );
    
    let resp = client.get(&search_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send().await
        .map_err(|e| format!("Yelp search request: {}", e))?;
    let body: serde_json::Value = resp.json().await
        .map_err(|e| format!("Yelp search parse: {}", e))?;
    
    let businesses = body.get("businesses").and_then(|v| v.as_array());
    
    if let Some(biz_list) = businesses {
        if let Some(biz) = biz_list.first() {
            let photos: Vec<String> = biz.get("image_url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_string()])
                .unwrap_or_default();
            
            let address_str = biz.get("location").and_then(|l| l.get("display_address"))
                .and_then(|a| a.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            
            let categories: Vec<String> = biz.get("categories")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|c| {
                    c.get("title").and_then(|v| v.as_str()).map(|s| s.to_string())
                }).collect())
                .unwrap_or_default();
            
            let coords = biz.get("coordinates");
            
            return Ok(Some(json!({
                "name": biz.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                "address": &address_str,
                "phone": biz.get("display_phone").and_then(|v| v.as_str()).unwrap_or(""),
                "website": biz.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "description": categories.join(", "),
                "latitude": coords.and_then(|c| c.get("latitude").and_then(|v| v.as_f64())),
                "longitude": coords.and_then(|c| c.get("longitude").and_then(|v| v.as_f64())),
                "rating": biz.get("rating").and_then(|v| v.as_f64()),
                "review_count": biz.get("review_count").and_then(|v| v.as_i64()),
                "images": json!(photos),
                "categories": categories.join(", "),
                "source": "yelp",
            })));
        }
    }
    
    Ok(None)
}

/// Merge two enrichment results — source values fill gaps in target
fn merge_enrichment(target: &mut serde_json::Value, source: serde_json::Value) {
    let target_obj = target.as_object_mut().unwrap();
    if let Some(src_obj) = source.as_object() {
        // Fill empty fields from source where target is empty
        let fill_fields = ["address", "phone", "website", "description", "categories"];
        for field in fill_fields {
            if let Some(val) = src_obj.get(field) {
                let t_val = target_obj.get(field).and_then(|v| v.as_str()).unwrap_or("");
                if t_val.is_empty() {
                    if let Some(s) = val.as_str() {
                        if !s.is_empty() {
                            target_obj.insert(field.to_string(), json!(s));
                        }
                    }
                }
            }
        }
        
        // Take best rating
        if let Some(src_rating) = src_obj.get("rating").and_then(|v| v.as_f64()) {
            let t_rating = target_obj.get("rating").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if src_rating > t_rating {
                target_obj.insert("rating".to_string(), json!(src_rating));
            }
        }
        
        // Take best review_count
        if let Some(src_count) = src_obj.get("review_count").and_then(|v| v.as_i64()) {
            let t_count = target_obj.get("review_count").and_then(|v| v.as_i64()).unwrap_or(0);
            if src_count > t_count {
                target_obj.insert("review_count".to_string(), json!(src_count));
            }
        }
        
        // Fill coordinates
        for coord in ["latitude", "longitude"] {
            if target_obj.get(coord).and_then(|v| v.as_f64()).is_none() {
                if let Some(val) = src_obj.get(coord).and_then(|v| v.as_f64()) {
                    target_obj.insert(coord.to_string(), json!(val));
                }
            }
        }
        
        // Merge images — dedup by URL
        if let Some(src_images) = src_obj.get("images").and_then(|v| v.as_array()) {
            let mut existing: HashSet<String> = target_obj.get("images")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            for img in src_images {
                if let Some(url) = img.as_str() {
                    if existing.insert(url.to_string()) {
                        // image added to set
                    }
                }
            }
            let merged: Vec<String> = existing.into_iter().collect();
            if !merged.is_empty() {
                target_obj.insert("images".to_string(), json!(merged));
            }
        }
    }
}

fn build_search_query(req: &EnrichRequest) -> String {
    let mut parts: Vec<&str> = Vec::new();
    parts.push(&req.name);
    if let Some(ref city) = req.city { if !city.is_empty() { parts.push(city); } }
    if let Some(ref state) = req.state { if !state.is_empty() { parts.push(state); } }
    parts.join(" ")
}

fn urlencode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "%20".to_string(),
        other => format!("%{:02X}", other as u8),
    }).collect()
}

// ── Import Logs ───────────────────────────────────────────────────────────

pub async fn list_import_logs(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let logs = sqlx::query_as::<_, ImportLog>(
        "SELECT * FROM import_logs ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(logs)))
}

pub async fn get_import_log(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let log = sqlx::query_as::<_, ImportLog>(
        "SELECT * FROM import_logs WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Import log not found".to_string()))?;
    Ok(Json(json!(log)))
}

// ── Export ────────────────────────────────────────────────────────────────

pub async fn export_businesses(
    State(state): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let businesses = sqlx::query_as::<_, Business>(
        "SELECT * FROM businesses WHERE directory_id = $1 ORDER BY name"
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(businesses)))
}

pub async fn export_reviews(
    State(state): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let reviews = sqlx::query_as::<_, Review>(
        "SELECT r.* FROM reviews r JOIN businesses b ON r.business_id = b.id WHERE b.directory_id = $1 ORDER BY r.created_at"
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(reviews)))
}

pub async fn export_contacts(
    State(state): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
    struct CrmContactExport {
        id: Uuid,
        first_name: Option<String>,
        last_name: Option<String>,
        email: Option<String>,
        phone: Option<String>,
        company: Option<String>,
        directory_id: Option<Uuid>,
        created_at: Option<chrono::DateTime<chrono::Utc>>,
    }
    let contacts = sqlx::query_as::<_, CrmContactExport>(
        "SELECT id, first_name, last_name, email, phone, company, directory_id, created_at FROM crm_contacts WHERE directory_id = $1 ORDER BY first_name"
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(contacts)))
}

// ── Export Templates ──────────────────────────────────────────────────────

pub async fn list_export_templates(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let templates = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates ORDER BY name"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(templates)))
}

pub async fn create_export_template(
    State(state): State<AppState>,
    Json(req): Json<CreateExportTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, ExportTemplate>(
        "INSERT INTO export_templates (name, entity_type, fields, directory_id, delimiter, include_header) VALUES ($1, $2, $3::jsonb, $4, $5, $6) RETURNING *"
    )
    .bind(&req.name)
    .bind(&req.entity_type)
    .bind(serde_json::to_value(&req.fields).map_err(|e| AppError::Internal(e.to_string()))?)
    .bind(req.directory_id)
    .bind(req.delimiter.unwrap_or_else(|| ",".to_string()))
    .bind(req.include_header.unwrap_or(true))
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(json!(template))))
}

pub async fn get_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Export template not found".to_string()))?;
    Ok(Json(json!(template)))
}

pub async fn update_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateExportTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE export_templates SET name = $1, entity_type = $2, fields = $3::jsonb, directory_id = $4, delimiter = $5, include_header = $6 WHERE id = $7"
    )
    .bind(&req.name)
    .bind(&req.entity_type)
    .bind(serde_json::to_value(&req.fields).map_err(|e| AppError::Internal(e.to_string()))?)
    .bind(req.directory_id)
    .bind(req.delimiter.unwrap_or_else(|| ",".to_string()))
    .bind(req.include_header.unwrap_or(true))
    .bind(id)
    .execute(&state.db)
    .await?;
    let template = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!(template)))
}

pub async fn delete_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM export_templates WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Export template not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Export template not found".to_string()))?;
    let data: Vec<serde_json::Value> = match template.entity_type.as_str() {
        "businesses" => {
            let rows = if let Some(did) = template.directory_id {
                sqlx::query("SELECT row_to_json(b.*)::text as j FROM businesses b WHERE b.directory_id = $1 ORDER BY b.name")
                    .bind(did).fetch_all(&state.db).await?
            } else {
                sqlx::query("SELECT row_to_json(b.*)::text as j FROM businesses b ORDER BY b.name")
                    .fetch_all(&state.db).await?
            };
            rows.iter().filter_map(|r| {
                let s: String = r.get("j");
                serde_json::from_str(&s).ok()
            }).collect()
        }
        "reviews" => {
            let rows = sqlx::query("SELECT row_to_json(r.*)::text as j FROM reviews r ORDER BY r.created_at")
                .fetch_all(&state.db).await?;
            rows.iter().filter_map(|r| {
                let s: String = r.get("j");
                serde_json::from_str(&s).ok()
            }).collect()
        }
        "contacts" => {
            let rows = if let Some(did) = template.directory_id {
                sqlx::query("SELECT row_to_json(c.*)::text as j FROM crm_contacts c WHERE c.directory_id = $1 ORDER BY c.first_name")
                    .bind(did).fetch_all(&state.db).await?
            } else {
                sqlx::query("SELECT row_to_json(c.*)::text as j FROM crm_contacts c ORDER BY c.first_name")
                    .fetch_all(&state.db).await?
            };
            rows.iter().filter_map(|r| {
                let s: String = r.get("j");
                serde_json::from_str(&s).ok()
            }).collect()
        }
        _ => return Err(AppError::Validation(format!("Unsupported entity type: {}", template.entity_type))),
    };
    Ok(Json(json!({
        "template": template,
        "rows": data.len(),
        "data": data,
    })))
}
