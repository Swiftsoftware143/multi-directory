//! Booking handlers — public appointment booking for directory businesses.
//!
//! These endpoints proxy to CoreSwift's public booking API to:
//! 1. Fetch available slot types for a directory's tenant
//! 2. Create a booking against a business
//! 3. Auto-provision a CoreSwift booking calendar if one doesn't exist
//! 4. Optionally advance the CRM deal on successful booking

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use tracing;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::coreswift::{coreswift_url, internal_key};

lazy_static::lazy_static! {
    static ref HTTP: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build reqwest client");
}

// ── Request / Response Types ──

#[derive(Debug, Deserialize)]
pub struct BookingRequest {
    pub contact_name: String,
    pub contact_email: String,
    pub contact_phone: Option<String>,
    pub preferred_date: String,       // YYYY-MM-DD
    pub preferred_time: Option<String>,
    pub notes: Option<String>,
    pub slot_id: Option<String>,      // optional slot type ID from available-slots
}

#[derive(Debug, Serialize)]
pub struct BookingResponse {
    pub success: bool,
    pub booking_id: Option<String>,
    pub message: String,
}

// ── Handler: GET available slots ──

/// GET /api/v1/directories/:slug/businesses/:business_id/available-slots
///
/// Proxies to CoreSwift's public available-slots endpoint for this directory's tenant.
pub async fn get_available_slots(
    State(s): State<AppState>,
    Path((slug, _business_id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<String>)>(
        "SELECT id, coreswift_tenant_id, booking_calendar_slug FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let (dir_id, tenant_id, calendar_prefix) = directory;

    let tenant_id = tenant_id
        .ok_or(AppError::BadRequest(format!(
            "Directory '{}' has no CoreSwift tenant configured", slug
        )))?;

    let base = coreswift_url();
    let resp = HTTP
        .get(format!("{}/api/public/bookings/public/slots/available/{}", base, tenant_id))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CoreSwift proxy error: {e}")))?;

    let status = resp.status();
    let body: Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("CoreSwift response parse error: {e}")))?;

    if !status.is_success() {
        return Err(AppError::Internal(format!(
            "CoreSwift available-slots returned {}: {}",
            status, body
        )));
    }

    Ok(Json(json!({
        "success": true,
        "tenant_id": tenant_id,
        "directory_id": dir_id,
        "calendar_prefix": calendar_prefix,
        "slots": body,
    })))
}

// ── Handler: POST book appointment ──

/// POST /api/v1/directories/:slug/businesses/:business_id/book
///
/// Creates a booking via CoreSwift's public checkout endpoint.
/// Auto-provisions the calendar if it doesn't exist.
/// On success, advances the deal from "Contacted" to "Qualified".
pub async fn create_booking(
    State(s): State<AppState>,
    Path((slug, business_id_or_slug)): Path<(String, String)>,
    Json(req): Json<BookingRequest>,
) -> ApiResult<impl IntoResponse> {
    // 1. Validate required fields
    if req.contact_name.trim().is_empty() {
        return Err(AppError::Validation("contact_name is required".to_string()));
    }
    if req.contact_email.trim().is_empty() {
        return Err(AppError::Validation("contact_email is required".to_string()));
    }
    if req.preferred_date.trim().is_empty() {
        return Err(AppError::Validation("preferred_date is required (YYYY-MM-DD)".to_string()));
    }

    // 2. Resolve directory by slug
    let dir = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<String>)>(
        "SELECT id, coreswift_tenant_id, booking_calendar_slug FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let (dir_id, tenant_id_opt, calendar_slug_opt) = dir;

    let tenant_id = tenant_id_opt
        .ok_or(AppError::BadRequest(format!(
            "Directory '{}' has no CoreSwift tenant configured", slug
        )))?;

    // 3. Use city-prefixed calendar slug (e.g., "pb-" for Palm Bay), fallback to directory slug
    let calendar_slug = calendar_slug_opt.clone().unwrap_or_else(|| slug.clone());

    // 4. Resolve business
    let business = if let Ok(bid) = Uuid::parse_str(&business_id_or_slug) {
        sqlx::query_as::<_, (Uuid, String, Option<String>)>(
            "SELECT id, name, website FROM businesses WHERE id = $1 AND directory_id = $2"
        )
        .bind(bid)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, (Uuid, String, Option<String>)>(
            "SELECT id, name, website FROM businesses WHERE slug = $1 AND directory_id = $2"
        )
        .bind(&business_id_or_slug)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    };

    let (business_id, business_name, business_website) = business
        .ok_or(AppError::NotFound("Business not found".to_string()))?;

    // 5. Auto-provision calendar in CoreSwift if it doesn't exist yet
    if let Err(e) = ensure_calendar_exists(&tenant_id, &calendar_slug, &slug).await {
        tracing::warn!("[bookings] Calendar auto-provision warning (non-fatal): {e}");
    }

    // 6. Build the checkout payload for CoreSwift — always include slot_name
    //    so it matches the default "Appointment Booking" slot we auto-provision
    let mut checkout_payload = json!({
        "calendar_slug": &calendar_slug,
        "slot_name": "Appointment Booking",
        "business_name": &business_name,
        "contact_name": &req.contact_name,
        "contact_email": &req.contact_email,
        "contact_phone": req.contact_phone,
        "start_date": &req.preferred_date,
        "start_time": req.preferred_time,
        "description": req.notes,
        "website": business_website,
    });

    if let Some(ref sid) = req.slot_id {
        if !sid.trim().is_empty() && sid.len() >= 32 {
            checkout_payload["slot_id"] = json!(sid);
        }
    }

    // 7. Proxy to CoreSwift public checkout endpoint
    let base = coreswift_url();
    let resp = HTTP
        .post(format!("{}/api/public/bookings/public/checkout", base))
        .json(&checkout_payload)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("CoreSwift checkout proxy error: {e}")))?;

    let status = resp.status();
    let body_text = resp.text().await
        .map_err(|e| AppError::Internal(format!("CoreSwift checkout read error: {e}")))?;

    tracing::info!("[bookings] CoreSwift checkout response: status={}, body={}", status, body_text);

    let body: Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            return Err(AppError::Internal(format!(
                "CoreSwift checkout returned status {} with non-JSON body: {} (parse: {})",
                status, &body_text.chars().take(200).collect::<String>(), e
            )));
        }
    };

    if !status.is_success() {
        let err_msg = body.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Booking failed at CoreSwift");
        return Err(AppError::Internal(format!(
            "CoreSwift checkout returned {}: {}",
            status, err_msg
        )));
    }

    // 8. Extract booking ID from response
    let booking_id = body.get("id")
        .or_else(|| body.get("booking_id"))
        .and_then(|v| v.as_str())
        .or_else(|| body.get("booking").and_then(|b| b.get("id")).and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    // 9. On successful booking, advance the deal from "Contacted" to "Qualified"
    let _ = advance_deal_to_qualified(&s.db, business_id).await;

    Ok((StatusCode::CREATED, Json(json!(BookingResponse {
        success: true,
        booking_id,
        message: "Booking created successfully".to_string(),
    }))))
}

// ── Auto-provision calendar ──

/// Ensure the booking calendar exists in CoreSwift for this directory.
/// Creates it via the internal API if it doesn't already exist.
/// Ignores "already exists" errors since the calendar may have been created manually.
async fn ensure_calendar_exists(
    tenant_id: &Uuid,
    calendar_slug: &str,
    directory_slug: &str,
) -> Result<(), String> {
    let base = coreswift_url();
    let key = internal_key();

    // Derive a human-readable city name from the directory slug
    let city_name = directory_slug.replace('-', " ").split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Step 1: Create the calendar
    let calendar_resp = HTTP
        .post(format!("{}/api/internal/bookings/calendars", base))
        .header("x-internal-key", &key)
        .json(&json!({
            "name": format!("{} Directory Bookings", city_name),
            "slug": calendar_slug,
            "description": format!("Appointment bookings for {} directory", city_name),
            "calendar_type": "city",
            "metadata": {
                "tenant_id": tenant_id.to_string(),
                "city_slug": directory_slug,
            }
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift calendar creation request failed: {e}"))?;

    let cal_status = calendar_resp.status();
    if !cal_status.is_success() {
        let body_text = calendar_resp.text().await.unwrap_or_else(|_| "unknown".to_string());
        // If already exists, continue; otherwise return error
        if !body_text.contains("already exists") && !body_text.contains("duplicate key") && !body_text.contains("unique") {
            return Err(format!("CoreSwift calendar creation returned {}: {}", cal_status, body_text));
        }
    }
    
    tracing::info!("[bookings] Calendar '{calendar_slug}' ready for tenant {tenant_id}");

    // Step 2: Create a default "Appointment Booking" slot type if it doesn't exist
    // The public checkout endpoint needs at least one slot to create a booking
    let slot_resp = HTTP
        .post(format!("{}/api/internal/bookings/slots/default", base))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "calendar_slug": calendar_slug,
            "slot_name": "Appointment Booking",
            "total_slots": -1,
            "default_duration_days": 1,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift slot creation request failed: {e}"))?;
    
    let slot_status = slot_resp.status();
    if slot_status.is_success() || slot_status.as_u16() == 409 || slot_status.as_u16() == 422 {
        tracing::info!("[bookings] Default slot ready for calendar '{calendar_slug}'");
        Ok(())
    } else {
        let body_text = slot_resp.text().await.unwrap_or_else(|_| "unknown".to_string());
        tracing::warn!("[bookings] Default slot creation non-fatal: {} {}", slot_status, body_text);
        // Slot creation is not fatal — the booking can still try with slot_name
        Ok(())
    }
}

/// Advance a deal from "Contacted" to "Qualified" when a booking is confirmed.
async fn advance_deal_to_qualified(
    db: &sqlx::PgPool,
    business_id: Uuid,
) -> Result<(), String> {
    let deal_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT dr.id FROM crm_deal_records dr
           JOIN claimed_businesses cb ON cb.business_id = $1
           WHERE dr.title ILIKE (SELECT name FROM businesses WHERE id = $1) || '%'
             AND dr.stage = 'Contacted'
             AND dr.status = 'open'
           LIMIT 1"#
    )
    .bind(business_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error finding deal: {e}"))?
    .flatten();

    if let Some(deal_id) = deal_id {
        sqlx::query("UPDATE crm_deal_records SET stage = 'Qualified', updated_at = NOW() WHERE id = $1")
            .bind(deal_id)
            .execute(db)
            .await
            .map_err(|e| format!("Failed to advance deal: {e}"))?;
        tracing::info!("[pipeline] Advanced deal {deal_id} to 'Qualified' after booking");
    } else {
        tracing::info!("[pipeline] No deal at 'Contacted' stage found for business {business_id} — skipping advance");
    }

    Ok(())
}
