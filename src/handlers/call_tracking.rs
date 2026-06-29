//! Call Tracking handlers for Multi-Directory API.
//! Tracks incoming/outgoing calls and phone number management.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Data Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CallLog {
    pub id: Uuid,
    pub caller_number: Option<String>,
    pub called_number: Option<String>,
    pub direction: Option<String>,
    pub duration_seconds: Option<i32>,
    pub call_status: Option<String>,
    pub recording_url: Option<String>,
    pub transcription: Option<String>,
    pub business_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub lead_name: Option<String>,
    pub lead_email: Option<String>,
    pub lead_notes: Option<String>,
    pub lead_status: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCallLogRequest {
    pub caller_number: Option<String>,
    pub called_number: Option<String>,
    pub direction: Option<String>,
    pub duration_seconds: Option<i32>,
    pub call_status: Option<String>,
    pub recording_url: Option<String>,
    pub transcription: Option<String>,
    pub business_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub lead_name: Option<String>,
    pub lead_email: Option<String>,
    pub lead_notes: Option<String>,
    pub lead_status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCallLeadRequest {
    pub lead_name: Option<String>,
    pub lead_email: Option<String>,
    pub lead_notes: Option<String>,
    pub lead_status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CallLogStats {
    pub total_calls: i64,
    pub missed_calls: i64,
    pub completed_calls: i64,
    pub voicemail_calls: i64,
    pub missed_percentage: f64,
    pub avg_duration_seconds: f64,
    pub total_duration_seconds: i64,
    pub total_unique_callers: i64,
    pub total_leads: i64,
}

// ── Phone Numbers ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PhoneNumber {
    pub id: Uuid,
    pub phone_number: String,
    pub friendly_name: Option<String>,
    pub sid: Option<String>,
    pub provider: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub forwarding_number: Option<String>,
    pub webhook_url: Option<String>,
    pub call_logging: Option<bool>,
    pub monthly_cost: Option<f64>,
    pub status: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePhoneNumberRequest {
    pub phone_number: String,
    pub friendly_name: Option<String>,
    pub sid: Option<String>,
    pub provider: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub forwarding_number: Option<String>,
    pub webhook_url: Option<String>,
    pub call_logging: Option<bool>,
    pub monthly_cost: Option<f64>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePhoneNumberRequest {
    pub friendly_name: Option<String>,
    pub sid: Option<String>,
    pub provider: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub forwarding_number: Option<String>,
    pub webhook_url: Option<String>,
    pub call_logging: Option<bool>,
    pub monthly_cost: Option<f64>,
    pub status: Option<String>,
}

// ── Directory Slug Extractor ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SlugPath {
    pub slug: String,
}

#[derive(Debug, Deserialize)]
pub struct IdPath {
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct BizIdPath {
    pub id: Uuid,
}

// ── Call Log Handlers ────────────────────────────────────────────────────────

/// GET /api/v1/call-logs — list all call logs
pub async fn list_call_logs(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let logs = sqlx::query_as::<_, CallLog>(
        "SELECT id, caller_number, called_number, direction, duration_seconds, call_status, recording_url, transcription, business_id, directory_id, lead_name, lead_email, lead_notes, lead_status, created_at FROM call_logs ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 "
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(logs))
}

/// POST /api/v1/call-logs — create a new call log
pub async fn create_call_log(
    State(s): State<AppState>,
    Json(body): Json<CreateCallLogRequest>,
) -> ApiResult<impl IntoResponse> {
    let log = sqlx::query_as::<_, CallLog>(
        "INSERT INTO call_logs (caller_number, called_number, direction, duration_seconds, call_status, recording_url, transcription, business_id, directory_id, lead_name, lead_email, lead_notes, lead_status) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413) RETURNING id, caller_number, called_number, direction, duration_seconds, call_status, recording_url, transcription, business_id, directory_id, lead_name, lead_email, lead_notes, lead_status, created_at "
    )
    .bind(&body.caller_number)
    .bind(&body.called_number)
    .bind(&body.direction)
    .bind(body.duration_seconds)
    .bind(&body.call_status)
    .bind(&body.recording_url)
    .bind(&body.transcription)
    .bind(body.business_id)
    .bind(body.directory_id)
    .bind(&body.lead_name)
    .bind(&body.lead_email)
    .bind(&body.lead_notes)
    .bind(&body.lead_status)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(log)))
}

/// GET /api/v1/call-logs/:id — get a single call log
pub async fn get_call_log(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let log = sqlx::query_as::<_, CallLog>(
        "SELECT id, caller_number, called_number, direction, duration_seconds, call_status, recording_url, transcription, business_id, directory_id, lead_name, lead_email, lead_notes, lead_status, created_at FROM call_logs WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Call log not found".to_string()))?;

    Ok(Json(log))
}

/// PUT /api/v1/call-logs/:id/lead — update lead info and status
pub async fn update_call_lead(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCallLeadRequest>,
) -> ApiResult<impl IntoResponse> {
    // Build dynamic update query
    let mut updates: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut param_idx = 1;

    if body.lead_name.is_some() {
        updates.push(format!("lead_name = ${}", param_idx));
        params.push(body.lead_name.clone().unwrap_or_default());
        param_idx += 1;
    }
    if body.lead_email.is_some() {
        updates.push(format!("lead_email = ${}", param_idx));
        params.push(body.lead_email.clone().unwrap_or_default());
        param_idx += 1;
    }
    if body.lead_notes.is_some() {
        updates.push(format!("lead_notes = ${}", param_idx));
        params.push(body.lead_notes.clone().unwrap_or_default());
        param_idx += 1;
    }
    if body.lead_status.is_some() {
        updates.push(format!("lead_status = ${}", param_idx));
        params.push(body.lead_status.clone().unwrap_or_default());
        param_idx += 1;
    }

    if updates.is_empty() {
        return Err(AppError::Validation("No fields to update".to_string()));
    }

    let query = format!(
        "UPDATE call_logs SET {} WHERE id = ${} RETURNING id, caller_number, called_number, direction, duration_seconds, call_status, recording_url, transcription, business_id, directory_id, lead_name, lead_email, lead_notes, lead_status, created_at",
        updates.join(", "),
        param_idx
    );

    let mut q = sqlx::query_as::<_, CallLog>(&query);
    for p in &params {
        q = q.bind(p);
    }
    q = q.bind(id);

    let log = q.fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Call log not found".to_string()))?;

    Ok(Json(log))
}

/// GET /api/v1/call-logs/stats — aggregated call statistics
pub async fn call_log_stats(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let stats = sqlx::query_as::<_, CallLogStats>(
        "SELECT COUNT(*)::bigint AS total_calls, COUNT(*) FILTER (WHERE call_status = 'missed')::bigint AS missed_calls, COUNT(*) FILTER (WHERE call_status = 'completed')::bigint AS completed_calls, COUNT(*) FILTER (WHERE call_status = 'voicemail')::bigint AS voicemail_calls, CASE WHEN COUNT(*) > 0 THEN ROUND(COUNT(*) FILTER (WHERE call_status = 'missed')::numeric / COUNT(*)::numeric * 100, 1)::float8 ELSE 0.0 END AS missed_percentage, COALESCE(ROUND(AVG(duration_seconds)::numeric, 1), 0.0)::float8 AS avg_duration_seconds, COALESCE(SUM(duration_seconds), 0)::bigint AS total_duration_seconds, COUNT(DISTINCT caller_number)::bigint AS total_unique_callers, COUNT(*) FILTER (WHERE lead_name IS NOT NULL AND lead_name != '')::bigint AS total_leads FROM call_logs "
    )
    .fetch_one(&s.db)
    .await?;

    Ok(Json(stats))
}

/// GET /api/v1/directories/:slug/call-logs — directory scoped
pub async fn directory_call_logs(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let logs = sqlx::query_as::<_, CallLog>(
        "SELECT cl.id, cl.caller_number, cl.called_number, cl.direction, cl.duration_seconds, cl.call_status, cl.recording_url, cl.transcription, cl.business_id, cl.directory_id, cl.lead_name, cl.lead_email, cl.lead_notes, cl.lead_status, cl.created_at FROM call_logs cl JOIN directories d ON d.id = cl.directory_id WHERE d.slug = \x241 ORDER BY cl.created_at DESC LIMIT \x242 OFFSET \x243 "
    )
    .bind(&slug)
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(logs))
}

/// GET /api/v1/businesses/:id/call-logs — per business
pub async fn business_call_logs(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let logs = sqlx::query_as::<_, CallLog>(
        "SELECT id, caller_number, called_number, direction, duration_seconds, call_status, recording_url, transcription, business_id, directory_id, lead_name, lead_email, lead_notes, lead_status, created_at FROM call_logs WHERE business_id = \x241 ORDER BY created_at DESC LIMIT \x242 OFFSET \x243 "
    )
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(logs))
}

// ── Phone Number Handlers ────────────────────────────────────────────────────

/// GET /api/v1/phone-numbers — list all phone numbers
pub async fn list_phone_numbers(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let numbers = sqlx::query_as::<_, PhoneNumber>(
        "SELECT id, phone_number, friendly_name, sid, provider, directory_id, business_id, forwarding_number, webhook_url, call_logging, monthly_cost, status, created_at FROM twilio_numbers ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 "
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(numbers))
}

/// POST /api/v1/phone-numbers — create a phone number record
pub async fn create_phone_number(
    State(s): State<AppState>,
    Json(body): Json<CreatePhoneNumberRequest>,
) -> ApiResult<impl IntoResponse> {
    if body.phone_number.trim().is_empty() {
        return Err(AppError::Validation("phone_number is required".to_string()));
    }

    let number = sqlx::query_as::<_, PhoneNumber>(
        "INSERT INTO twilio_numbers (phone_number, friendly_name, sid, provider, directory_id, business_id, forwarding_number, webhook_url, call_logging, monthly_cost, status) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411) RETURNING id, phone_number, friendly_name, sid, provider, directory_id, business_id, forwarding_number, webhook_url, call_logging, monthly_cost, status, created_at "
    )
    .bind(&body.phone_number)
    .bind(&body.friendly_name)
    .bind(&body.sid)
    .bind(&body.provider)
    .bind(body.directory_id)
    .bind(body.business_id)
    .bind(&body.forwarding_number)
    .bind(&body.webhook_url)
    .bind(body.call_logging)
    .bind(body.monthly_cost)
    .bind(&body.status)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(number)))
}

/// GET /api/v1/phone-numbers/:id — get a single phone number
pub async fn get_phone_number(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let number = sqlx::query_as::<_, PhoneNumber>(
        "SELECT id, phone_number, friendly_name, sid, provider, directory_id, business_id, forwarding_number, webhook_url, call_logging, monthly_cost, status, created_at FROM twilio_numbers WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Phone number not found".to_string()))?;

    Ok(Json(number))
}

/// PUT /api/v1/phone-numbers/:id — update a phone number
pub async fn update_phone_number(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePhoneNumberRequest>,
) -> ApiResult<impl IntoResponse> {
    // Build dynamic update
    let mut updates: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut bind_values: Vec<serde_json::Value> = Vec::new();
    let mut param_idx = 1;

    macro_rules! push_update {
        ($field:expr, $val:expr) => {
            if $val.is_some() {
                updates.push(format!("{} = ${}", $field, param_idx));
                if let Some(v) = $val {
                    // Use serde_json::Value for flexibility
                    bind_values.push(serde_json::to_value(&v).unwrap_or(serde_json::Value::Null));
                    params.push(format!("${}", param_idx));
                    param_idx += 1;
                }
            }
        };
    }

    // Since sqlx doesn't do dynamic bind easily, use raw sql with explicit bind
    // We'll use a simpler approach with query builder
    let log = sqlx::query_as::<_, PhoneNumber>(
        "UPDATE twilio_numbers SET friendly_name = COALESCE(\x241, friendly_name), sid = COALESCE(\x242, sid), provider = COALESCE(\x243, provider), directory_id = COALESCE(\x244, directory_id), business_id = COALESCE(\x245, business_id), forwarding_number = COALESCE(\x246, forwarding_number), webhook_url = COALESCE(\x247, webhook_url), call_logging = COALESCE(\x248, call_logging), monthly_cost = COALESCE(\x249, monthly_cost), status = COALESCE(\x2410, status) WHERE id = \x2411 RETURNING id, phone_number, friendly_name, sid, provider, directory_id, business_id, forwarding_number, webhook_url, call_logging, monthly_cost, status, created_at "
    )
    .bind(&body.friendly_name)
    .bind(&body.sid)
    .bind(&body.provider)
    .bind(body.directory_id)
    .bind(body.business_id)
    .bind(&body.forwarding_number)
    .bind(&body.webhook_url)
    .bind(body.call_logging)
    .bind(body.monthly_cost)
    .bind(&body.status)
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Phone number not found".to_string()))?;

    Ok(Json(log))
}

/// DELETE /api/v1/phone-numbers/:id — delete a phone number
pub async fn delete_phone_number(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM twilio_numbers WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Phone number not found".to_string()));
    }

    Ok(Json(serde_json::json!({"deleted": true, "id": id})))
}

/// POST /api/v1/phone-numbers/:id/provision — placeholder mark as provisioned
pub async fn provision_phone_number(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let number = sqlx::query_as::<_, PhoneNumber>(
        "UPDATE twilio_numbers SET status = 'provisioned' WHERE id = \x241 RETURNING id, phone_number, friendly_name, sid, provider, directory_id, business_id, forwarding_number, webhook_url, call_logging, monthly_cost, status, created_at "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Phone number not found".to_string()))?;

    Ok(Json(serde_json::json!({
        "provisioned": true,
        "phone_number": number.phone_number,
        "id": number.id
    })))
}

/// GET /api/v1/directories/:slug/phone-numbers — per directory
pub async fn directory_phone_numbers(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let numbers = sqlx::query_as::<_, PhoneNumber>(
        "SELECT pn.id, pn.phone_number, pn.friendly_name, pn.sid, pn.provider, pn.directory_id, pn.business_id, pn.forwarding_number, pn.webhook_url, pn.call_logging, pn.monthly_cost, pn.status, pn.created_at FROM twilio_numbers pn JOIN directories d ON d.id = pn.directory_id WHERE d.slug = \x241 ORDER BY pn.created_at DESC LIMIT \x242 OFFSET \x243 "
    )
    .bind(&slug)
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(numbers))
}
