//! Contact Intelligence Pipeline — Monthly cron job for unclaimed business enrichment
//!
//! POST /api/v1/cron/contact-intelligence
//!
//! Idempotent handler that processes unclaimed businesses in batches of 500.
//! For each business:
//!   - Validates phone via Twilio Lookup (placeholder)
//!   - Checks website still resolves (HTTP HEAD/GET)
//!   - Attempts Google Places enrichment for hours/photos/address updates
//!   - Attempts email discovery if missing
//!
//! Writes results to `data_enrichment_logs` with source='monthly_pipeline'.
//! Updates `businesses.updated_at` and `businesses.enriched_at`.

use axum::{
    extract::State,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, ApiResult};
use crate::AppState;

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ContactIntelligenceResponse {
    pub records_processed: u64,
    pub successes: u64,
    pub failures: u64,
    pub enriched: Vec<EnrichedBusiness>,
}

#[derive(Debug, Serialize)]
pub struct EnrichedBusiness {
    pub business_id: Uuid,
    pub fields_updated: Vec<String>,
    pub enrichment_source: String,
}

// ── Internal query row ───────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct UnclaimedBusiness {
    id: Uuid,
    name: String,
    address: Option<String>,
    city: Option<String>,
    phone: Option<String>,
    website: Option<String>,
    email: Option<String>,
}

// ── Handler ──────────────────────────────────────────────────────────────────

const BATCH_SIZE: i64 = 500;

/// POST /api/v1/cron/contact-intelligence
pub async fn contact_intelligence_pipeline(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("MultiDirectory/1.0 ContactIntelligence")
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to build HTTP client: {}", e)))?;

    let mut total_processed: u64 = 0;
    let mut total_successes: u64 = 0;
    let mut total_failures: u64 = 0;
    let mut enriched_results: Vec<EnrichedBusiness> = Vec::new();
    let mut offset: i64 = 0;

    loop {
        // ── Batch query: only unclaimed businesses that are stale or never enriched ──
        let batch: Vec<UnclaimedBusiness> = sqlx::query_as::<_, UnclaimedBusiness>(
            "SELECT b.id, b.name, b.address, b.city, b.phone, b.website, b.email \
             FROM businesses b \
             LEFT JOIN claimed_businesses cb ON cb.business_id = b.id \
             WHERE cb.id IS NULL \
               AND (b.updated_at < NOW() - INTERVAL '30 days' OR b.enriched_at IS NULL) \
             ORDER BY b.id \
             LIMIT $1 OFFSET $2"
        )
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(&state.db)
        .await?;

        if batch.is_empty() {
            break; // No more records to process
        }

        for biz in &batch {
            total_processed += 1;

            let mut fields_updated: Vec<String> = Vec::new();
            let mut enrichment_source = "monthly_pipeline".to_string();
            let mut overall_success = true;

            // ── 1. Phone validation via Twilio Lookup (placeholder) ──
            if let Some(ref phone) = biz.phone {
                match validate_phone_placeholder(phone).await {
                    Ok(valid) => {
                        if valid {
                            fields_updated.push("phone_validated".to_string());
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Phone validation failed for business {}: {}", biz.id, e);
                    }
                }
            }

            // ── 2. Website resolution check ──
            if let Some(ref website) = biz.website {
                match check_website_resolves(&http_client, website).await {
                    Ok(resolves) => {
                        if resolves {
                            fields_updated.push("website_resolved".to_string());
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Website check failed for business {}: {}", biz.id, e);
                    }
                }
            }

            // ── 3. Google Places enrichment ──
            match enrich_via_google_places(&state, &http_client, biz).await {
                Ok(Some(place_fields)) => {
                    fields_updated.extend(place_fields);
                    enrichment_source = "monthly_pipeline,google_places".to_string();
                }
                Ok(None) => { /* no match */ }
                Err(e) => {
                    tracing::warn!("Google Places enrichment failed for business {}: {}", biz.id, e);
                }
            }

            // ── 4. Email discovery (if missing) ──
            if biz.email.is_none() || biz.email.as_deref() == Some("") {
                match attempt_email_discovery(biz, &http_client).await {
                    Ok(Some(_discovered)) => {
                        fields_updated.push("email_discovered".to_string());
                        enrichment_source = "monthly_pipeline,email_discovery".to_string();
                    }
                    Ok(None) => { /* no email found */ }
                    Err(e) => {
                        tracing::warn!("Email discovery failed for business {}: {}", biz.id, e);
                    }
                }
            }

            // ── Log the enrichment result ──
            let status = if overall_success { "completed" } else { "partial" };
            let data_after = serde_json::json!({
                "fields_updated": fields_updated,
                "enrichment_source": enrichment_source,
            });

            if let Err(e) = sqlx::query(
                "INSERT INTO data_enrichment_logs \
                 (business_id, source, enrichment_type, data_after, status) \
                 VALUES ($1, $2, $3, $4::jsonb, $5)"
            )
            .bind(biz.id)
            .bind("monthly_pipeline")
            .bind("contact_intelligence")
            .bind(&data_after)
            .bind(status)
            .execute(&state.db)
            .await
            {
                tracing::error!("Failed to log enrichment for business {}: {}", biz.id, e);
                overall_success = false;
            }

            // ── Update business timestamps ──
            if let Err(e) = sqlx::query(
                "UPDATE businesses SET updated_at = NOW(), enriched_at = NOW() WHERE id = $1"
            )
            .bind(biz.id)
            .execute(&state.db)
            .await
            {
                tracing::error!("Failed to update timestamp for business {}: {}", biz.id, e);
                overall_success = false;
            }

            if overall_success {
                total_successes += 1;
            } else {
                total_failures += 1;
            }

            enriched_results.push(EnrichedBusiness {
                business_id: biz.id,
                fields_updated,
                enrichment_source,
            });
        }

        offset += BATCH_SIZE;
    }

    Ok(Json(ContactIntelligenceResponse {
        records_processed: total_processed,
        successes: total_successes,
        failures: total_failures,
        enriched: enriched_results,
    }))
}

// ── Sub-routines ─────────────────────────────────────────────────────────────

/// Validate phone number via Twilio Lookup API (placeholder)
///
/// In production, this would call:
///   GET https://lookups.twilio.com/v2/PhoneNumbers/{phone}
/// with Twilio credentials.
///
/// For now, returns true if the phone looks like a valid E.164 format.
async fn validate_phone_placeholder(phone: &str) -> Result<bool, String> {
    let cleaned: String = phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    if cleaned.len() >= 10 && cleaned.len() <= 16 {
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Check if a website URL resolves via HTTP HEAD (fallback to GET)
async fn check_website_resolves(client: &Client, url: &str) -> Result<bool, String> {
    // Normalize URL
    let normalized = if !url.starts_with("http://") && !url.starts_with("https://") {
        format!("https://{}", url)
    } else {
        url.to_string()
    };

    // Try HEAD first
    match client.head(&normalized).send().await {
        Ok(resp) => Ok(resp.status().is_success() || resp.status().is_redirection()),
        Err(_) => {
            // Fallback to GET
            match client.get(&normalized).send().await {
                Ok(resp) => Ok(resp.status().is_success() || resp.status().is_redirection()),
                Err(e) => {
                    // Try with http://
                    let http_url = normalized.replace("https://", "http://");
                    if http_url != normalized {
                        match client.get(&http_url).send().await {
                            Ok(resp) => Ok(resp.status().is_success() || resp.status().is_redirection()),
                            Err(_) => Err(format!("Website unreachable: {}", e)),
                        }
                    } else {
                        Err(format!("Website unreachable: {}", e))
                    }
                }
            }
        }
    }
}

/// Attempt Google Places enrichment for a business
///
/// In production, this calls the Google Places API to fetch:
///   - hours (opening_hours)
///   - photos
///   - address corrections
///   - phone/website verification
///
/// For now, uses a placeholder search via the existing enrichment infrastructure.
async fn enrich_via_google_places(
    state: &AppState,
    _client: &Client,
    biz: &UnclaimedBusiness,
) -> Result<Option<Vec<String>>, String> {
    let api_key = match std::env::var("GOOGLE_PLACES_API_KEY") {
        Ok(k) => k,
        Err(_) => return Ok(None), // No key configured, skip
    };

    let search_query = format!(
        "{} {} {}",
        biz.name.trim(),
        biz.city.as_deref().unwrap_or(""),
        biz.address.as_deref().unwrap_or(""),
    );

    let url = format!(
        "https://maps.googleapis.com/maps/api/place/findplacefromtext/json?input={}&inputtype=textquery&fields=place_id,name,formatted_address,formatted_phone_number,website,opening_hours,photos,geometry,rating,user_ratings_total&key={}",
        urlencoding(&search_query.trim()),
        api_key
    );

    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("Google Places request failed: {}", e))?;

    let result: serde_json::Value = resp.json()
        .await
        .map_err(|e| format!("Failed to parse Places response: {}", e))?;

    let candidates = result.get("candidates")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let place = match candidates.first() {
        Some(p) => p,
        None => return Ok(None),
    };

    let mut fields: Vec<String> = Vec::new();
    let business_phone = biz.phone.as_deref().unwrap_or("");
    let business_website = biz.website.as_deref().unwrap_or("");

    // Check for opening hours
    if place.get("opening_hours").is_some() {
        fields.push("opening_hours".to_string());
    }

    // Check for photos
    if place.get("photos").and_then(|p| p.as_array()).map(|a| !a.is_empty()).unwrap_or(false) {
        fields.push("photos".to_string());
    }

    // Check for address correction
    if let Some(gplaces_address) = place.get("formatted_address").and_then(|v| v.as_str()) {
        if !gplaces_address.is_empty() {
            if let Some(ref current_addr) = biz.address {
                if current_addr.to_lowercase() != gplaces_address.to_lowercase() {
                    // Address differs — could update, but for now just note it
                    fields.push("address_correction_available".to_string());
                }
            }
        }
    }

    // Check phone match/update
    if let Some(gplaces_phone) = place.get("formatted_phone_number").and_then(|v| v.as_str()) {
        if business_phone.is_empty() || business_phone != gplaces_phone {
            fields.push("phone_correction_available".to_string());
        }
    }

    // Check website match/update
    if let Some(gplaces_website) = place.get("website").and_then(|v| v.as_str()) {
        if business_website.is_empty() || business_website != gplaces_website {
            fields.push("website_correction_available".to_string());
        }
    }

    // Update latitude/longitude if we have them
    if let Some(geometry) = place.get("geometry") {
        if let (Some(lat), Some(lng)) = (
            geometry.get("location").and_then(|l| l.get("lat")).and_then(|v| v.as_f64()),
            geometry.get("location").and_then(|l| l.get("lng")).and_then(|v| v.as_f64()),
        ) {
            sqlx::query("UPDATE businesses SET latitude = $1, longitude = $2 WHERE id = $3")
                .bind(lat)
                .bind(lng)
                .bind(biz.id)
                .execute(&state.db)
                .await
                .map_err(|e| format!("Failed to update coordinates: {}", e))?;
            fields.push("coordinates".to_string());
        }
    }

    // Cache the place details
    let name = place.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let formatted_address = place.get("formatted_address").and_then(|v| v.as_str()).unwrap_or("");
    let phone = place.get("formatted_phone_number").and_then(|v| v.as_str());
    let website = place.get("website").and_then(|v| v.as_str());
    let rating = place.get("rating").and_then(|v| v.as_f64());
    let user_ratings_total = place.get("user_ratings_total").and_then(|v| v.as_i64()).map(|v| v as i32);
    let types: Vec<String> = place.get("types")
        .and_then(|t| t.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let photos: Vec<String> = place.get("photos")
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|v| v.to_string().into()).collect())
        .unwrap_or_default();
    let opening_hours = place.get("opening_hours").cloned();

    if let Some(place_id) = place.get("place_id").and_then(|v| v.as_str()) {
        let _ = sqlx::query(
            "INSERT INTO google_places_cache \
             (query, place_id, name, formatted_address, phone, website, \
              latitude, longitude, rating, user_ratings_total, types, photos, \
              opening_hours, place_details) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::jsonb, $14::jsonb) \
             ON CONFLICT DO NOTHING"
        )
        .bind(&search_query.trim())
        .bind(place_id)
        .bind(name)
        .bind(formatted_address)
        .bind(phone)
        .bind(website)
        .bind(rating) // reuse lat field as rating for cache
        .bind(user_ratings_total)
        .bind(rating)
        .bind(user_ratings_total)
        .bind(&types)
        .bind(&photos)
        .bind(opening_hours.clone().unwrap_or(serde_json::Value::Null))
        .bind(place.clone())
        .execute(&state.db)
        .await;
    }

    if fields.is_empty() {
        Ok(None)
    } else {
        Ok(Some(fields))
    }
}

/// Attempt email discovery for businesses missing email (placeholder)
///
/// In production, this could use:
///   - Hunter.io API
///   - Clearbit Reveal API
///   - Scraping the website for contact pages
///
/// For now, returns None (no email discovered).
async fn attempt_email_discovery(
    _biz: &UnclaimedBusiness,
    _client: &Client,
) -> Result<Option<String>, String> {
    // Placeholder: email discovery requires API key or scraping logic
    // Return None for now — wire up to Hunter.io / Clearbit when keys are configured
    Ok(None)
}

// ── URL encoding helper (copied from data_company.rs for local use) ──────────

fn urlencoding(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "+".to_string(),
        other => format!("%{:02X}", other as u8),
    }).collect()
}
