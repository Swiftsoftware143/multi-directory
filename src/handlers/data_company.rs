//! Phase 4 — Data Company features
//! Google Places autofill, business verification, data enrichment, bulk CSV export

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Google Places Cache ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct GooglePlacesCache {
    pub id: Uuid,
    pub query: String,
    pub place_id: Option<String>,
    pub name: Option<String>,
    pub formatted_address: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub rating: Option<f64>,
    pub user_ratings_total: Option<i32>,
    pub types: Option<Vec<String>>,
    pub photos: Option<Vec<String>>,
    pub opening_hours: Option<serde_json::Value>,
    pub place_details: Option<serde_json::Value>,
    pub cached_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct PlacesAutocompleteQuery {
    pub input: String,
    pub radius: Option<f64>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub directory_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlacesAutocompleteResult {
    pub place_id: String,
    pub name: String,
    pub formatted_address: String,
    pub types: Vec<String>,
    pub matched: bool,
}

#[derive(Debug, Deserialize)]
pub struct PlaceDetailsQuery {
    pub place_id: String,
    pub directory_id: Option<String>,
}

/// GET /api/v1/places/autocomplete — search Google Places for autocomplete suggestions
pub async fn places_autocomplete(
    State(state): State<AppState>,
    Query(q): Query<PlacesAutocompleteQuery>,
) -> ApiResult<impl IntoResponse> {
    let api_key = get_google_api_key(&state, q.directory_id.as_deref()).await?;

    // Check cache first
    let cached = sqlx::query_as::<_, GooglePlacesCache>(
        "SELECT * FROM google_places_cache WHERE query = $1 AND expires_at > NOW() ORDER BY cached_at DESC LIMIT 1"
    )
    .bind(&q.input)
    .fetch_optional(&state.db)
    .await?;

    if let Some(cached_entry) = cached {
        let results: Vec<PlacesAutocompleteResult> = if let Some(details) = &cached_entry.place_details {
            serde_json::from_value(details.clone())
                .unwrap_or_else(|_| vec![PlacesAutocompleteResult {
                    place_id: cached_entry.place_id.clone().unwrap_or_default(),
                    name: cached_entry.name.clone().unwrap_or_default(),
                    formatted_address: cached_entry.formatted_address.clone().unwrap_or_default(),
                    types: cached_entry.types.clone().unwrap_or_default(),
                    matched: true,
                }])
        } else {
            vec![PlacesAutocompleteResult {
                place_id: cached_entry.place_id.clone().unwrap_or_default(),
                name: cached_entry.name.clone().unwrap_or_default(),
                formatted_address: cached_entry.formatted_address.clone().unwrap_or_default(),
                types: cached_entry.types.clone().unwrap_or_default(),
                matched: true,
            }]
        };
        return Ok(Json(serde_json::json!({ "results": results, "cached": true })));
    }

    // Build Google Places Autocomplete URL
    let mut url = format!(
        "https://maps.googleapis.com/maps/api/place/autocomplete/json?input={}&key={}",
        urlencoding(&q.input),
        api_key
    );
    if let Some(lat) = q.lat {
        if let Some(lng) = q.lng {
            let radius = q.radius.unwrap_or(50000.0);
            url.push_str(&format!("&location={},{}&radius={}", lat, lng, radius as i64));
        }
    }

    let resp = reqwest::get(&url).await
        .map_err(|e| AppError::Internal(format!("Google Places request failed: {}", e)))?;

    let body: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("Failed to parse Google Places response: {}", e)))?;

    let predictions = body.get("predictions").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let results: Vec<PlacesAutocompleteResult> = predictions.iter().filter_map(|p| {
        let place_id = p.get("place_id")?.as_str()?.to_string();
        let name = p.get("structured_formatting")
            .and_then(|f| f.get("main_text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let formatted_address = p.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
        let types: Vec<String> = p.get("types")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        Some(PlacesAutocompleteResult { place_id, name, formatted_address, types, matched: false })
    }).collect();

    // Cache the results
    if let Some(first) = results.first() {
        let details = serde_json::to_value(&results).unwrap_or_default();
        sqlx::query(
            "INSERT INTO google_places_cache (query, place_id, name, formatted_address, types, place_details) VALUES ($1, $2, $3, $4, $5, $6::jsonb)
             ON CONFLICT DO NOTHING"
        )
        .bind(&q.input)
        .bind(&first.place_id)
        .bind(&first.name)
        .bind(&first.formatted_address)
        .bind(&first.types)
        .bind(&details)
        .execute(&state.db)
        .await?;
    }

    Ok(Json(serde_json::json!({ "results": results, "cached": false })))
}

/// GET /api/v1/places/details — get detailed info for a Google Place
pub async fn place_details(
    State(state): State<AppState>,
    Query(q): Query<PlaceDetailsQuery>,
) -> ApiResult<impl IntoResponse> {
    let api_key = get_google_api_key(&state, q.directory_id.as_deref()).await?;

    // Check cache
    let cached = sqlx::query_as::<_, GooglePlacesCache>(
        "SELECT * FROM google_places_cache WHERE place_id = $1 AND expires_at > NOW() ORDER BY cached_at DESC LIMIT 1"
    )
    .bind(&q.place_id)
    .fetch_optional(&state.db)
    .await?;

    if let Some(cached_entry) = cached {
        if let Some(details) = &cached_entry.place_details {
            return Ok(Json(serde_json::json!({ "place": details, "cached": true })));
        }
    }

    let url = format!(
        "https://maps.googleapis.com/maps/api/place/details/json?place_id={}&fields=name,formatted_address,formatted_phone_number,website,geometry,rating,user_ratings_total,types,photos,opening_hours,url,vicinity&key={}",
        q.place_id, api_key
    );

    let resp = reqwest::get(&url).await
        .map_err(|e| AppError::Internal(format!("Google Places details request failed: {}", e)))?;

    let mut body: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("Failed to parse Google Places details response: {}", e)))?;

    let result = body.get_mut("result").cloned().unwrap_or_default();

    // Cache the details
    let name = result.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let formatted_address = result.get("formatted_address").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let phone = result.get("formatted_phone_number").and_then(|v| v.as_str()).map(String::from);
    let website = result.get("website").and_then(|v| v.as_str()).map(String::from);
    let latitude = result.get("geometry")
        .and_then(|g| g.get("location"))
        .and_then(|l| l.get("lat"))
        .and_then(|v| v.as_f64());
    let longitude = result.get("geometry")
        .and_then(|g| g.get("location"))
        .and_then(|l| l.get("lng"))
        .and_then(|v| v.as_f64());
    let rating = result.get("rating").and_then(|v| v.as_f64());
    let user_ratings_total = result.get("user_ratings_total").and_then(|v| v.as_i64()).map(|v| v as i32);
    let types: Vec<String> = result.get("types")
        .and_then(|t| t.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    // Deduplicate/update in cache
    sqlx::query(
        "INSERT INTO google_places_cache (query, place_id, name, formatted_address, phone, website, latitude, longitude, rating, user_ratings_total, types, place_details)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb)
         ON CONFLICT DO NOTHING"
    )
    .bind(&q.place_id)
    .bind(&q.place_id)
    .bind(&name)
    .bind(&formatted_address)
    .bind(&phone)
    .bind(&website)
    .bind(latitude)
    .bind(longitude)
    .bind(rating)
    .bind(user_ratings_total)
    .bind(&types)
    .bind(&result)
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({ "place": result, "cached": false })))
}

// ── Business Verification ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BusinessVerification {
    pub id: Uuid,
    pub business_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub method: String,
    pub status: String,
    pub verified_by: Option<Uuid>,
    pub verified_at: Option<DateTime<Utc>>,
    pub verification_doc_url: Option<String>,
    pub notes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub verified_data: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// ── Yelp Fusion API ──

#[derive(Debug, Deserialize)]
pub struct YelpSearchQuery {
    pub term: Option<String>,
    pub location: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub categories: Option<String>,
    pub limit: Option<u32>,
    pub directory_id: Option<String>,  // used to look up per-directory API key
}

#[derive(Debug, Deserialize)]
pub struct YelpDetailsQuery {
    pub yelp_id: String,
    pub directory_id: Option<String>,
}

/// GET /api/v1/yelp/search — search Yelp Fusion API for businesses
pub async fn yelp_search(
    State(state): State<AppState>,
    Query(q): Query<YelpSearchQuery>,
) -> ApiResult<impl IntoResponse> {
    let api_key = get_yelp_api_key(&state, q.directory_id.as_deref()).await?;

    let mut url = format!("https://api.yelp.com/v3/businesses/search?limit={}", q.limit.unwrap_or(10));
    if let Some(term) = &q.term {
        url.push_str(&format!("&term={}", urlencoding(&term)));
    }
    if let Some(location) = &q.location {
        url.push_str(&format!("&location={}", urlencoding(&location)));
    }
    if let Some(lat) = q.latitude {
        if let Some(lon) = q.longitude {
            url.push_str(&format!("&latitude={}&longitude={}", lat, lon));
        }
    }
    if let Some(cats) = &q.categories {
        url.push_str(&format!("&categories={}", urlencoding(&cats)));
    }

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Yelp request failed: {}", e)))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Yelp parse failed: {}", e)))?;

    Ok(Json(body))
}

/// GET /api/v1/yelp/details — get detailed info for a Yelp business
pub async fn yelp_details(
    State(state): State<AppState>,
    Query(q): Query<YelpDetailsQuery>,
) -> ApiResult<impl IntoResponse> {
    let api_key = get_yelp_api_key(&state, q.directory_id.as_deref()).await?;

    let url = format!("https://api.yelp.com/v3/businesses/{}", urlencoding(&q.yelp_id));

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Yelp request failed: {}", e)))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Yelp parse failed: {}", e)))?;

    Ok(Json(body))
}

/// Get Yelp API key — per-directory with env var fallback
pub(crate) async fn get_yelp_api_key(state: &AppState, directory_id: Option<&str>) -> Result<String, AppError> {
    // Check per-directory config first
    if let Some(dir_id) = directory_id {
        if let Ok(uid) = Uuid::parse_str(dir_id) {
            let config: Option<serde_json::Value> = sqlx::query_scalar(
                "SELECT api_config FROM directories WHERE id = $1"
            )
            .bind(uid)
            .fetch_optional(&state.db)
            .await
            .map_err(|_| AppError::Internal("DB error reading directory config".to_string()))?
            .flatten();

            if let Some(cfg) = config {
                if let Some(key) = cfg.get("yelp_api_key").and_then(|k| k.as_str()) {
                    if !key.is_empty() && key != "disabled" {
                        return Ok(key.to_string());
                    }
                }
            }
        }
    }

    // Fallback to env var
    std::env::var("YELP_API_KEY")
        .map_err(|_| AppError::Internal("YELP_API_KEY not configured (set globally or per-directory)".to_string()))
}

/// Get Google Places API key — per-directory with env var fallback
pub(crate) async fn get_google_api_key(state: &AppState, directory_id: Option<&str>) -> Result<String, AppError> {
    if let Some(dir_id) = directory_id {
        if let Ok(uid) = Uuid::parse_str(dir_id) {
            let config: Option<serde_json::Value> = sqlx::query_scalar(
                "SELECT api_config FROM directories WHERE id = $1"
            )
            .bind(uid)
            .fetch_optional(&state.db)
            .await
            .map_err(|_| AppError::Internal("DB error reading directory config".to_string()))?
            .flatten();

            if let Some(cfg) = config {
                if let Some(key) = cfg.get("google_places_api_key").and_then(|k| k.as_str()) {
                    if !key.is_empty() && key != "disabled" {
                        return Ok(key.to_string());
                    }
                }
            }
        }
    }

    std::env::var("GOOGLE_PLACES_API_KEY")
        .map_err(|_| AppError::Internal("GOOGLE_PLACES_API_KEY not configured".to_string()))
}

#[derive(Debug, Deserialize)]
pub struct CreateVerificationRequest {
    pub business_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub method: Option<String>,
    pub verification_doc_url: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVerificationRequest {
    pub status: Option<String>,
    pub notes: Option<String>,
    pub verification_doc_url: Option<String>,
    pub verified_data: Option<serde_json::Value>,
}

/// POST /api/v1/verifications — create a business verification request
pub async fn create_verification(
    State(state): State<AppState>,
    Json(req): Json<CreateVerificationRequest>,
) -> ApiResult<impl IntoResponse> {
    // Check business exists
    let biz_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM businesses WHERE id = $1)"
    )
    .bind(req.business_id)
    .fetch_one(&state.db)
    .await?;

    if !biz_exists {
        return Err(AppError::NotFound("Business not found".to_string()));
    }

    let method = req.method.unwrap_or_else(|| "manual".to_string());

    let verification = sqlx::query_as::<_, BusinessVerification>(
        "INSERT INTO business_verifications (business_id, directory_id, method, verification_doc_url, notes)
         VALUES ($1, $2, $3, $4, $5) RETURNING *"
    )
    .bind(req.business_id)
    .bind(req.directory_id)
    .bind(&method)
    .bind(&req.verification_doc_url)
    .bind(&req.notes)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(serde_json::json!(verification))))
}

/// GET /api/v1/verifications/:id — get a verification record
pub async fn get_verification(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let verification = sqlx::query_as::<_, BusinessVerification>(
        "SELECT * FROM business_verifications WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Verification not found".to_string()))?;

    Ok(Json(serde_json::json!(verification)))
}

/// PUT /api/v1/verifications/:id — update verification status (approve/reject)
pub async fn update_verification(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateVerificationRequest>,
) -> ApiResult<impl IntoResponse> {
    let status = req.status.as_deref().unwrap_or("pending");
    let valid_statuses = ["pending", "approved", "rejected", "expired"];
    if !valid_statuses.contains(&status) {
        return Err(AppError::Validation(format!(
            "Invalid status '{}'. Must be one of: {}", status, valid_statuses.join(", ")
        )));
    }

    let verification = sqlx::query_as::<_, BusinessVerification>(
        "UPDATE business_verifications SET status = COALESCE($1, status), notes = COALESCE($2, notes),
         verification_doc_url = COALESCE($3, verification_doc_url),
         verified_data = COALESCE($4::jsonb, verified_data),
         verified_at = CASE WHEN $1 = 'approved' THEN NOW() ELSE verified_at END,
         updated_at = NOW()
         WHERE id = $5 RETURNING *"
    )
    .bind(&req.status)
    .bind(&req.notes)
    .bind(&req.verification_doc_url)
    .bind(&req.verified_data)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Verification not found".to_string()))?;

    Ok(Json(serde_json::json!(verification)))
}

/// GET /api/v1/businesses/:id/verifications — list verifications for a business
pub async fn business_verifications(
    State(state): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let verifications = sqlx::query_as::<_, BusinessVerification>(
        "SELECT * FROM business_verifications WHERE business_id = $1 ORDER BY created_at DESC"
    )
    .bind(business_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!(verifications)))
}

/// GET /api/v1/verifications — list all verifications (with optional status filter)
pub async fn list_verifications(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let verifications = sqlx::query_as::<_, BusinessVerification>(
        "SELECT * FROM business_verifications ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!(verifications)))
}

// ── Data Enrichment ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EnrichmentRequest {
    pub business_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DataEnrichmentLog {
    pub id: Uuid,
    pub business_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub source: String,
    pub enrichment_type: String,
    pub data_before: Option<serde_json::Value>,
    pub data_after: Option<serde_json::Value>,
    pub confidence: Option<f64>,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// POST /api/v1/enrich/business — enrich a business's data from Google Places
pub async fn enrich_business(
    State(state): State<AppState>,
    Json(req): Json<EnrichmentRequest>,
) -> ApiResult<impl IntoResponse> {
    let source = req.source.clone().unwrap_or_else(|| "google_places".to_string());

    // Get current business data
    let business = sqlx::query_as::<_, crate::models::Business>(
        "SELECT * FROM businesses WHERE id = $1"
    )
    .bind(req.business_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;

    let data_before = serde_json::json!({
        "name": business.name,
        "address": business.address,
        "phone": business.phone,
        "email": business.email,
        "website": business.website,
        "latitude": business.latitude,
        "longitude": business.longitude,
    });

    // Try Google Places enrichment
    let api_key = match std::env::var("GOOGLE_PLACES_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            return Ok(Json(serde_json::json!({
                "status": "skipped",
                "message": "GOOGLE_PLACES_API_KEY not configured",
                "business_id": req.business_id,
            })));
        }
    };

    // Search for the business
    let search_query = format!("{} {} {} {}", 
        business.name,
        business.city.as_deref().unwrap_or(""),
        business.state.as_deref().unwrap_or(""),
        business.zip.as_deref().unwrap_or(""),
    );

    let search_url = format!(
        "https://maps.googleapis.com/maps/api/place/findplacefromtext/json?input={}&inputtype=textquery&fields=place_id,name,formatted_address,formatted_phone_number,website,geometry,rating,user_ratings_total&key={}",
        urlencoding(&search_query.trim()),
        api_key
    );

    let resp = reqwest::get(&search_url).await
        .map_err(|e| AppError::Internal(format!("Places search failed: {}", e)))?;

    let result: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("Failed to parse Places search response: {}", e)))?;

    let candidates = result.get("candidates").and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let place = candidates.first();

    let enrichment_result = if let Some(p) = place {
        let enriched_name = p.get("name").and_then(|v| v.as_str());
        let enriched_address = p.get("formatted_address").and_then(|v| v.as_str());
        let enriched_phone = p.get("formatted_phone_number").and_then(|v| v.as_str());
        let enriched_website = p.get("website").and_then(|v| v.as_str());
        let lat = p.get("geometry").and_then(|g| g.get("location")).and_then(|l| l.get("lat")).and_then(|v| v.as_f64());
        let lng = p.get("geometry").and_then(|g| g.get("location")).and_then(|l| l.get("lng")).and_then(|v| v.as_f64());
        let rating = p.get("rating").and_then(|v| v.as_f64());
        let rating_total = p.get("user_ratings_total").and_then(|v| v.as_i64());

        let data_after = serde_json::json!({
            "name": enriched_name,
            "address": enriched_address,
            "phone": enriched_phone,
            "website": enriched_website,
            "latitude": lat,
            "longitude": lng,
            "rating": rating,
            "user_ratings_total": rating_total,
        });

        let confidence = if enriched_phone.is_some() || enriched_website.is_some() { 0.9 } else { 0.5 };

        // Update business with enriched data if better than current
        if let Some(addr) = enriched_address {
            if business.address.is_none() || business.address.as_deref() == Some("") {
                sqlx::query("UPDATE businesses SET address = $1 WHERE id = $2")
                    .bind(addr).bind(req.business_id)
                    .execute(&state.db).await?;
            }
        }
        if let Some(ph) = enriched_phone {
            if business.phone.is_none() || business.phone.as_deref() == Some("") {
                sqlx::query("UPDATE businesses SET phone = $1 WHERE id = $2")
                    .bind(ph).bind(req.business_id)
                    .execute(&state.db).await?;
            }
        }
        if let Some(ws) = enriched_website {
            if business.website.is_none() || business.website.as_deref() == Some("") {
                sqlx::query("UPDATE businesses SET website = $1 WHERE id = $2")
                    .bind(ws).bind(req.business_id)
                    .execute(&state.db).await?;
            }
        }
        if let (Some(latv), Some(lngv)) = (lat, lng) {
            sqlx::query("UPDATE businesses SET latitude = $1, longitude = $2 WHERE id = $3")
                .bind(latv).bind(lngv).bind(req.business_id)
                .execute(&state.db).await?;
        }

        serde_json::json!({
            "source": "google_places",
            "matched": true,
            "confidence": confidence,
            "place_id": p.get("place_id"),
            "data_after": data_after,
        })
    } else {
        serde_json::json!({
            "source": "google_places",
            "matched": false,
            "confidence": 0.0,
        })
    };

    // Log the enrichment
    sqlx::query(
        "INSERT INTO data_enrichment_logs (business_id, directory_id, source, enrichment_type, data_before, data_after, confidence, status)
         VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, $7, $8)"
    )
    .bind(req.business_id)
    .bind(req.directory_id)
    .bind(&source)
    .bind("places_enrichment")
    .bind(&data_before)
    .bind(enrichment_result.get("data_after").cloned().unwrap_or_default())
    .bind(enrichment_result.get("confidence").and_then(|v| v.as_f64()))
    .bind(if enrichment_result.get("matched").and_then(|v| v.as_bool()).unwrap_or(false) { "completed" } else { "no_match" })
    .execute(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "business_id": req.business_id,
        "enrichment": enrichment_result,
    })))
}

/// GET /api/v1/enrich/logs — list enrichment logs
pub async fn list_enrichment_logs(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let logs = sqlx::query_as::<_, DataEnrichmentLog>(
        "SELECT * FROM data_enrichment_logs ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(serde_json::json!(logs)))
}

// ── Bulk CSV Export ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BulkExportQuery {
    pub entity_type: Option<String>,
    pub directory_id: Option<Uuid>,
    pub format: Option<String>,
}

/// GET /api/v1/export/bulk — bulk export businesses/reviews as JSON (CSV via format=csv)
pub async fn bulk_export(
    State(state): State<AppState>,
    Query(q): Query<BulkExportQuery>,
) -> ApiResult<impl IntoResponse> {
    let entity_type = q.entity_type.as_deref().unwrap_or("businesses");
    let output_format = q.format.as_deref().unwrap_or("json");

    let rows: Vec<serde_json::Value> = match entity_type {
        "businesses" => {
            let results = if let Some(did) = q.directory_id {
                sqlx::query("SELECT jsonb_agg(to_jsonb(b.*)) as data FROM businesses b WHERE b.directory_id = $1")
                    .bind(did).fetch_optional(&state.db).await?
            } else {
                sqlx::query("SELECT jsonb_agg(to_jsonb(b.*)) as data FROM businesses b")
                    .fetch_optional(&state.db).await?
            };
            results
                .and_then(|r| r.get::<Option<serde_json::Value>, _>("data"))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        }
        "reviews" => {
            let results = if let Some(did) = q.directory_id {
                sqlx::query("SELECT jsonb_agg(to_jsonb(r.*)) as data FROM reviews r JOIN businesses b ON r.business_id = b.id WHERE b.directory_id = $1")
                    .bind(did).fetch_optional(&state.db).await?
            } else {
                sqlx::query("SELECT jsonb_agg(to_jsonb(r.*)) as data FROM reviews r")
                    .fetch_optional(&state.db).await?
            };
            results
                .and_then(|r| r.get::<Option<serde_json::Value>, _>("data"))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        }
        "contacts" => {
            let results = if let Some(did) = q.directory_id {
                sqlx::query("SELECT jsonb_agg(to_jsonb(c.*)) as data FROM crm_contacts c WHERE c.directory_id = $1")
                    .bind(did).fetch_optional(&state.db).await?
            } else {
                sqlx::query("SELECT jsonb_agg(to_jsonb(c.*)) as data FROM crm_contacts c")
                    .fetch_optional(&state.db).await?
            };
            results
                .and_then(|r| r.get::<Option<serde_json::Value>, _>("data"))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        }
        "deals" => {
            let results = sqlx::query("SELECT jsonb_agg(to_jsonb(d.*)) as data FROM deals d")
                .fetch_optional(&state.db).await?;
            results
                .and_then(|r| r.get::<Option<serde_json::Value>, _>("data"))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        }
        _ => return Err(AppError::Validation(format!("Unsupported entity type: {}", entity_type))),
    };

    if output_format == "csv" && !rows.is_empty() {
        let headers: Vec<String> = rows[0].as_object()
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default();

        let mut csv_lines: Vec<String> = Vec::new();
        csv_lines.push(headers.join(","));

        for row in &rows {
            let vals: Vec<String> = headers.iter().map(|h| {
                row.get(h)
                    .and_then(|v| {
                        match v {
                            serde_json::Value::String(s) => Some(s.clone()),
                            serde_json::Value::Null => String::new().into(),
                            other => Some(other.to_string()),
                        }
                    })
                    .unwrap_or_default()
                    .replace('"', "\"\"")
            }).collect();
            csv_lines.push(format!("\"{}\"", vals.join("\",\"")));
        }

        return Ok(Json(serde_json::json!({
            "format": "csv",
            "entity_type": entity_type,
            "count": rows.len(),
            "headers": headers,
            "data": csv_lines,
        })));
    }

    Ok(Json(serde_json::json!({
        "entity_type": entity_type,
        "count": rows.len(),
        "data": rows,
    })))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn urlencoding(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "+".to_string(),
        other => format!("%{:02X}", other as u8),
    }).collect()
}
