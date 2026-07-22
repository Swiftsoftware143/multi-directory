//! Booking handlers — service booking requests for directory businesses.
//!
//! Stage 5: Visitor booking flow.
//! Visitors browse services → request booking → merchant gets notified.
//!
//! Existing CoreSwift proxy endpoints (available-slots, create_booking, booking_page)
//! are preserved alongside the new visitor booking system.

use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use tracing;

use crate::AppState;
use crate::auth::models::Claims;
use crate::auth::middleware::{verify_token, is_business_owner, is_admin, is_visitor};
use crate::error::{AppError, ApiResult};
use crate::coreswift::{coreswift_url, internal_key};

lazy_static::lazy_static! {
    static ref HTTP: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build reqwest client");
}


// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════
/// Extract and verify JWT claims from Authorization header
fn extract_claims_from_headers(headers: &HeaderMap, jwt_secret: &str) -> Result<Claims, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    crate::auth::middleware::verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)
}

// ═══════════════════════════════════════════════════════════════════════════════
// CoreSwift Proxy Endpoints (preserved from Stage 3)
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct BookingRequest {
    pub contact_name: String,
    pub contact_email: String,
    pub contact_phone: Option<String>,
    pub preferred_date: String,
    pub preferred_time: Option<String>,
    pub notes: Option<String>,
    pub slot_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BookingResponse {
    pub success: bool,
    pub booking_id: Option<String>,
    pub message: String,
}

/// GET /api/v1/directories/:slug/businesses/:business_id/available-slots
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

/// POST /api/v1/directories/:slug/businesses/:business_id/book
pub async fn create_booking(
    State(s): State<AppState>,
    Path((slug, business_id_or_slug)): Path<(String, String)>,
    Json(req): Json<BookingRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.contact_name.trim().is_empty() {
        return Err(AppError::Validation("contact_name is required".to_string()));
    }
    if req.contact_email.trim().is_empty() {
        return Err(AppError::Validation("contact_email is required".to_string()));
    }
    if req.preferred_date.trim().is_empty() {
        return Err(AppError::Validation("preferred_date is required (YYYY-MM-DD)".to_string()));
    }

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

    let calendar_slug = calendar_slug_opt.clone().unwrap_or_else(|| slug.clone());

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

    if let Err(e) = ensure_calendar_exists(&tenant_id, &calendar_slug, &slug).await {
        tracing::warn!("[bookings] Calendar auto-provision warning (non-fatal): {e}");
    }

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

    let booking_id = body.get("id")
        .or_else(|| body.get("booking_id"))
        .and_then(|v| v.as_str())
        .or_else(|| body.get("booking").and_then(|b| b.get("id")).and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let _ = advance_deal_to_qualified(&s.db, business_id).await;

    Ok((StatusCode::CREATED, Json(json!(BookingResponse {
        success: true,
        booking_id,
        message: "Booking created successfully".to_string(),
    }))))
}

async fn ensure_calendar_exists(
    tenant_id: &Uuid,
    calendar_slug: &str,
    directory_slug: &str,
) -> Result<(), String> {
    let base = coreswift_url();
    let key = internal_key();

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
        if !body_text.contains("already exists") && !body_text.contains("duplicate key") && !body_text.contains("unique") {
            return Err(format!("CoreSwift calendar creation returned {}: {}", cal_status, body_text));
        }
    }

    tracing::info!("[bookings] Calendar '{calendar_slug}' ready for tenant {tenant_id}");

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
        Ok(())
    }
}

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

// ═══════════════════════════════════════════════════════════════════════════════
// Stage 5: Visitor Service Booking System
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct CreateServiceBookingRequest {
    pub directory_id: Uuid,
    pub business_id: Uuid,
    pub service_name: Option<String>,
    pub description: Option<String>,
    pub preferred_date: Option<String>,
    pub preferred_time: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_email: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ServiceBookingResponse {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub business_id: Uuid,
    pub visitor_account_id: Uuid,
    pub service_name: Option<String>,
    pub description: Option<String>,
    pub preferred_date: Option<chrono::DateTime<chrono::Utc>>,
    pub preferred_time: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_email: Option<String>,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// POST /api/v1/bookings — visitor creates a service booking request
pub async fn create_service_booking(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateServiceBookingRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let visitor_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Validate visitor has a visitor account
    let exists: bool = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT EXISTS(SELECT 1 FROM visitor_accounts WHERE id = $1)"
    )
    .bind(visitor_id)
    .fetch_one(&s.db)
    .await?
    .unwrap_or(false);

    if !exists {
        return Err(AppError::Forbidden("Not a registered visitor".to_string()));
    }

    // Verify business exists in directory
    let biz_exists: bool = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT EXISTS(SELECT 1 FROM businesses WHERE id = $1 AND directory_id = $2)"
    )
    .bind(req.business_id)
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?
    .unwrap_or(false);

    if !biz_exists {
        return Err(AppError::NotFound("Business not found in this directory".to_string()));
    }

    // Extract visitor email from their account
    let visitor_email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM visitor_accounts WHERE id = $1"
    )
    .bind(visitor_id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    let contact_email = req.contact_email.clone().or(visitor_email);

    // Parse preferred_date if provided
    let preferred_date = if let Some(ref date_str) = req.preferred_date {
        Some(chrono::DateTime::parse_from_rfc3339(date_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()))
    } else {
        None
    };

    let row = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<String>, Option<String>,
        Option<chrono::DateTime<chrono::Utc>>, Option<String>, Option<String>, Option<String>,
        String, Option<String>, chrono::DateTime<chrono::Utc>)>(
        r#"INSERT INTO service_bookings
           (directory_id, business_id, visitor_account_id, service_name, description,
            preferred_date, preferred_time, contact_phone, contact_email, notes)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, directory_id, business_id, service_name, description,
                     preferred_date, preferred_time, contact_phone, contact_email,
                     status, notes, created_at"#
    )
    .bind(req.directory_id)
    .bind(req.business_id)
    .bind(visitor_id)
    .bind(&req.service_name)
    .bind(&req.description)
    .bind(preferred_date)
    .bind(&req.preferred_time)
    .bind(&req.contact_phone)
    .bind(&contact_email)
    .bind(&req.notes)
    .fetch_one(&s.db)
    .await?;

    let (id, directory_id, business_id_row, service_name, description,
         preferred_date_r, preferred_time, contact_phone, contact_email_r,
         status, notes, created_at) = row;

    // Notify business owner(s) — async, non-blocking
    let _ = notify_business_owners(&s, business_id_row, &service_name, &notes).await;

    Ok((StatusCode::CREATED, Json(json!(ServiceBookingResponse {
        id,
        directory_id,
        business_id: business_id_row,
        visitor_account_id: visitor_id,
        service_name,
        description,
        preferred_date: preferred_date_r,
        preferred_time,
        contact_phone,
        contact_email: contact_email_r,
        status,
        notes,
        created_at,
    }))))
}

/// GET /api/v1/bookings — list visitor's own bookings
pub async fn list_visitor_bookings(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let visitor_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<String>, Option<String>,
        Option<chrono::DateTime<chrono::Utc>>, Option<String>, Option<String>, Option<String>,
        String, Option<String>, chrono::DateTime<chrono::Utc>, String)>(
        r#"SELECT sb.id, sb.directory_id, sb.business_id, sb.service_name, sb.description,
                  sb.preferred_date, sb.preferred_time, sb.contact_phone, sb.contact_email,
                  sb.status, sb.notes, sb.created_at, b.name as business_name
           FROM service_bookings sb
           JOIN businesses b ON b.id = sb.business_id
           WHERE sb.visitor_account_id = $1
           ORDER BY sb.created_at DESC"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    let bookings: Vec<Value> = rows.into_iter().map(|(id, dir_id, biz_id, svc_name, desc,
        pref_date, pref_time, phone, email, status, notes, created_at, biz_name)| {
        json!({
            "id": id,
            "directory_id": dir_id,
            "business_id": biz_id,
            "business_name": biz_name,
            "service_name": svc_name,
            "description": desc,
            "preferred_date": pref_date,
            "preferred_time": pref_time,
            "status": status,
            "notes": notes,
            "created_at": created_at,
        })
    }).collect();

    Ok(Json(json!({ "success": true, "bookings": bookings })))
}

/// GET /api/v1/bookings/:id — get booking details
/// Visible to visitor who created it OR business owner
pub async fn get_booking(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(booking_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let caller_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let role = &claims.role;

    let row = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<String>, Option<String>,
        Option<chrono::DateTime<chrono::Utc>>, Option<String>, Option<String>, Option<String>,
        String, Option<String>, chrono::DateTime<chrono::Utc>,)>(
        r#"SELECT directory_id, business_id, visitor_account_id, service_name, description,
                  preferred_date, preferred_time, contact_phone, contact_email,
                  status, notes, created_at
           FROM service_bookings WHERE id = $1"#
    )
    .bind(booking_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Booking not found".to_string()))?;

    let (dir_id, biz_id, visitor_id, svc_name, desc,
         pref_date, pref_time, phone, email, status, notes, created_at) = row;

    // Check authorization: visitor can see own booking, business owner can see theirs
    let is_owner = if role == "visitor" {
        visitor_id == caller_id
    } else if role == "business_owner" {
        is_business_claimed_by_user(&s.db, biz_id, caller_id).await?
    } else {
        is_admin(&claims)
    };

    if !is_owner {
        return Err(AppError::Forbidden("Not authorized to view this booking".to_string()));
    }

    // Get business name
    let biz_name: String = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
        .bind(biz_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or_else(|_| "Unknown Business".to_string());

    // Only expose contact info to visitor or business owner
    let (contact_phone, contact_email) = if is_owner {
        (phone, email)
    } else {
        (None, None)
    };

    Ok(Json(json!({
        "success": true,
        "booking": {
            "id": booking_id,
            "directory_id": dir_id,
            "business_id": biz_id,
            "business_name": biz_name,
            "visitor_account_id": visitor_id,
            "service_name": svc_name,
            "description": desc,
            "preferred_date": pref_date,
            "preferred_time": pref_time,
            "contact_phone": contact_phone,
            "contact_email": contact_email,
            "status": status,
            "notes": notes,
            "created_at": created_at,
        }
    })))
}

/// GET /api/v1/business/:business_id/bookings — list bookings for a business (owner only)
pub async fn list_business_bookings(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Verify the caller owns or manages this business
    let is_authorized = if is_admin(&claims) {
        true
    } else {
        is_business_claimed_by_user(&s.db, business_id, user_id).await?
    };

    if !is_authorized {
        return Err(AppError::Forbidden("Not authorized to view bookings for this business".to_string()));
    }

    let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<String>, Option<String>,
        Option<chrono::DateTime<chrono::Utc>>, Option<String>, Option<String>, Option<String>,
        String, Option<String>, chrono::DateTime<chrono::Utc>, Option<String>)>(
        r#"SELECT sb.id, sb.directory_id, sb.visitor_account_id, sb.service_name, sb.description,
                  sb.preferred_date, sb.preferred_time, sb.contact_phone, sb.contact_email,
                  sb.status, sb.notes, sb.created_at, va.name as visitor_name
           FROM service_bookings sb
           LEFT JOIN visitor_accounts va ON va.id = sb.visitor_account_id
           WHERE sb.business_id = $1
           ORDER BY sb.created_at DESC"#
    )
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    let bookings: Vec<Value> = rows.into_iter().map(|(id, dir_id, visitor_id, svc_name, desc,
        pref_date, pref_time, phone, email, status, notes, created_at, visitor_name)| {
        json!({
            "id": id,
            "directory_id": dir_id,
            "visitor_name": visitor_name,
            "visitor_account_id": visitor_id,
            "service_name": svc_name,
            "description": desc,
            "preferred_date": pref_date,
            "preferred_time": pref_time,
            "contact_phone": phone,
            "contact_email": email,
            "status": status,
            "notes": notes,
            "created_at": created_at,
        })
    }).collect();

    Ok(Json(json!({ "success": true, "bookings": bookings, "business_id": business_id })))
}

/// POST /api/v1/bookings/:id/status — business owner updates booking status
pub async fn update_booking_status(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(booking_id): Path<Uuid>,
    Json(req): Json<UpdateStatusRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Get the booking's business_id
    let biz_id: Uuid = sqlx::query_scalar(
        "SELECT business_id FROM service_bookings WHERE id = $1"
    )
    .bind(booking_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Booking not found".to_string()))?;

    // Verify caller owns or manages the business
    let is_authorized = if is_admin(&claims) {
        true
    } else {
        is_business_claimed_by_user(&s.db, biz_id, user_id).await?
    };

    if !is_authorized {
        return Err(AppError::Forbidden("Not authorized to update this booking's status".to_string()));
    }

    let valid_statuses = ["pending", "confirmed", "declined", "completed", "cancelled"];
    if !valid_statuses.contains(&req.status.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid status '{}'. Valid: {}", req.status, valid_statuses.join(", ")
        )));
    }

    // Prevent transitioning from completed/cancelled
    let current_status: String = sqlx::query_scalar(
        "SELECT status FROM service_bookings WHERE id = $1"
    )
    .bind(booking_id)
    .fetch_one(&s.db)
    .await?;

    if current_status == "completed" || current_status == "cancelled" {
        return Err(AppError::BadRequest(format!(
            "Cannot update status of a {} booking", current_status
        )));
    }

    sqlx::query("UPDATE service_bookings SET status = $1 WHERE id = $2")
        .bind(&req.status)
        .bind(booking_id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({
        "success": true,
        "booking_id": booking_id,
        "status": req.status,
        "message": format!("Booking status updated to '{}'", req.status),
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
}

/// POST /api/v1/bookings/:id/cancel — visitor cancels own booking
pub async fn cancel_booking(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(booking_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let visitor_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let role = &claims.role;

    // Get booking info to verify ownership
    let booking: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT visitor_account_id, status FROM service_bookings WHERE id = $1"
    )
    .bind(booking_id)
    .fetch_optional(&s.db)
    .await?;

    let (owner_id, current_status) = booking
        .ok_or(AppError::NotFound("Booking not found".to_string()))?;

    // Only visitor who owns the booking (or admin) can cancel
    if visitor_id != owner_id && !is_admin(&claims) {
        return Err(AppError::Forbidden("Not authorized to cancel this booking".to_string()));
    }

    if current_status == "completed" || current_status == "cancelled" {
        return Err(AppError::BadRequest(format!(
            "Cannot cancel a {} booking", current_status
        )));
    }

    sqlx::query("UPDATE service_bookings SET status = 'cancelled' WHERE id = $1")
        .bind(booking_id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({
        "success": true,
        "booking_id": booking_id,
        "status": "cancelled",
        "message": "Booking cancelled successfully",
    })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Check if a user owns/claims a business
async fn is_business_claimed_by_user(
    db: &sqlx::PgPool,
    business_id: Uuid,
    user_id: Uuid,
) -> Result<bool, AppError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM claimed_businesses WHERE business_id = $1 AND user_id = $2 AND is_active = true"
    )
    .bind(business_id)
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(count > 0)
}

/// Notify business owners of a new booking (async via event logging)
async fn notify_business_owners(
    s: &AppState,
    business_id: Uuid,
    service_name: &Option<String>,
    notes: &Option<String>,
) -> Result<(), String> {
    // Log the notification event for the automation system to pick up
    let event_payload = json!({
        "type": "service_booking_created",
        "business_id": business_id.to_string(),
        "service_name": service_name,
        "notes": notes,
    });

    let result = sqlx::query(
        "INSERT INTO directory_events (directory_id, business_id, event_type, payload)
         VALUES ((SELECT directory_id FROM businesses WHERE id = $1), $1, 'service_booking_created', $2)"
    )
    .bind(business_id)
    .bind(event_payload.to_string())
    .execute(&s.db)
    .await;

    match result {
        Ok(_) => tracing::info!("[notifications] Created service_booking_created event for business {}", business_id),
        Err(e) => tracing::warn!("[notifications] Failed to create event: {}", e),
    }

    // Also try to send an email notification if configured
    if let Some(owner_emails) = get_business_owner_emails(&s.db, business_id).await {
        for owner_email in &owner_emails {
            let _ = send_booking_notification_email(s, owner_email, business_id, service_name).await;
        }
    }

    Ok(())
}

/// Get owner emails for a business
async fn get_business_owner_emails(db: &sqlx::PgPool, business_id: Uuid) -> Option<Vec<String>> {
    let emails: Vec<String> = sqlx::query_scalar(
        r#"SELECT DISTINCT u.email FROM users u
           JOIN claimed_businesses cb ON cb.user_id = u.id
           WHERE cb.business_id = $1 AND cb.is_active = true"#
    )
    .bind(business_id)
    .fetch_all(db)
    .await
    .ok()?;

    if emails.is_empty() {
        None
    } else {
        Some(emails)
    }
}

/// Send a notification email about new booking (placeholder)
async fn send_booking_notification_email(
    s: &AppState,
    to_email: &str,
    business_id: Uuid,
    service_name: &Option<String>,
) -> Result<(), String> {
    let biz_name: String = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or_else(|_| "Your Business".to_string());

    let svc = service_name.as_deref().unwrap_or("a service");

    // Log email to events system — actual sending handled by automation
    let email_payload = json!({
        "to": to_email,
        "subject": format!("New Booking Request for {}", biz_name),
        "body": format!(
            "You have received a new booking request for {} ({}) from {}.\n\nLog in to your dashboard to review and respond.",
            biz_name, svc, biz_name
        ),
        "type": "booking_notification",
    });

    sqlx::query(
        "INSERT INTO directory_events (directory_id, business_id, event_type, payload)
         VALUES (
            (SELECT directory_id FROM businesses WHERE id = $1),
            $1, 'email_notification', $2
        )"
    )
    .bind(business_id)
    .bind(email_payload.to_string())
    .execute(&s.db)
    .await
    .ok();

    Ok(())
}
