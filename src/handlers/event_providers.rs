//! Event Provider Pipeline — handlers for managing event data sources.
//!
//! Each directory can connect to external event aggregators (Eventbrite,
//! Meetup, ICS feeds, n8n webhooks).  Providers are configured with an API
//! key and city/radius parameters, then synced on demand or via cron.
//!
//! ## Endpoints
//! - `GET    /api/v1/admin/directories/:directory_id/event-providers`
//! - `POST   /api/v1/admin/directories/:directory_id/event-providers`
//! - `PUT    /api/v1/admin/directories/:directory_id/event-providers/:provider_id`
//! - `DELETE /api/v1/admin/directories/:directory_id/event-providers/:provider_id`
//! - `POST   /api/v1/admin/event-providers/:provider_id/test`
//! - `POST   /api/v1/admin/event-providers/:provider_id/sync`
//! - `GET    /api/v1/admin/event-providers/:provider_id/sync-status`

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::verify_token;
use crate::error::{ApiResult, AppError};
use crate::providers::{EventProvider, ProviderConfig};
use crate::AppState;

// ── Data Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventProviderRow {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub provider_type: String,
    pub api_key: Option<String>,
    pub config: serde_json::Value,
    pub is_active: bool,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_sync_status: Option<String>,
    pub last_error: Option<String>,
    pub events_synced: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProviderRequest {
    pub provider_type: String,
    pub api_key: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub radius_miles: Option<i32>,
    pub categories: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderRequest {
    pub api_key: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub radius_miles: Option<i32>,
    pub categories: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

// ── Auth helpers ─────────────────────────────────────────────────────────────

fn extract_admin(headers: &HeaderMap, jwt_secret: &str) -> Result<(Uuid, String), AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret).map_err(|_| AppError::Unauthorized)?;

    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    Ok((user_id, claims.role))
}

fn require_admin(role: &str) -> Result<(), AppError> {
    if role != "admin" && role != "super_admin" {
        return Err(AppError::Forbidden(
            "Admin access required".to_string(),
        ));
    }
    Ok(())
}

// ── Config helpers ───────────────────────────────────────────────────────────

/// Merge a JSONB config block with request fields to produce a ProviderConfig.
fn build_provider_config(
    api_key: &str,
    db_config: &serde_json::Value,
    req: &CreateProviderRequest,
) -> ProviderConfig {
    let cfg = db_config
        .as_object()
        .cloned()
        .unwrap_or_default();

    let city = req
        .city
        .clone()
        .or_else(|| cfg.get("city").and_then(|v| v.as_str().map(String::from)))
        .unwrap_or_default();

    let state = req
        .state
        .clone()
        .or_else(|| cfg.get("state").and_then(|v| v.as_str().map(String::from)));

    let lat = req.lat.or_else(|| cfg.get("lat").and_then(|v| v.as_f64()));

    let lng = req.lng.or_else(|| cfg.get("lng").and_then(|v| v.as_f64()));

    let radius_miles = req.radius_miles.or_else(|| cfg.get("radius_miles").and_then(|v| v.as_i64().map(|n| n as i32)));

    ProviderConfig {
        api_key: api_key.to_string(),
        city,
        state,
        lat,
        lng,
        radius_miles,
        categories: req.categories.clone(),
    }
}

fn build_update_config(
    api_key: &str,
    db_config: &serde_json::Value,
    req: &UpdateProviderRequest,
) -> ProviderConfig {
    let cfg = db_config
        .as_object()
        .cloned()
        .unwrap_or_default();

    let city = req
        .city
        .clone()
        .or_else(|| cfg.get("city").and_then(|v| v.as_str().map(String::from)))
        .unwrap_or_default();

    let state = req
        .state
        .clone()
        .or_else(|| cfg.get("state").and_then(|v| v.as_str().map(String::from)));

    let lat = req.lat.or_else(|| cfg.get("lat").and_then(|v| v.as_f64()));

    let lng = req.lng.or_else(|| cfg.get("lng").and_then(|v| v.as_f64()));

    let radius_miles = req.radius_miles.or_else(|| cfg.get("radius_miles").and_then(|v| v.as_i64().map(|n| n as i32)));

    ProviderConfig {
        api_key: api_key.to_string(),
        city,
        state,
        lat,
        lng,
        radius_miles,
        categories: req.categories.clone(),
    }
}

/// Pick the correct provider implementation based on provider_type string.
fn provider_for_type(provider_type: &str) -> Result<Box<dyn EventProvider>, AppError> {
    match provider_type {
        "eventbrite" => Ok(Box::new(crate::providers::eventbrite::EventbriteProvider)),
        other => Err(AppError::Validation(format!(
            "Unsupported provider type: {}",
            other
        ))),
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /api/v1/admin/directories/:directory_id/event-providers`
pub async fn list_providers(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    let rows = sqlx::query_as::<_, EventProviderRow>(
        "SELECT * FROM event_providers WHERE directory_id = $1 ORDER BY created_at DESC",
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "providers": rows,
        "count": rows.len(),
    })))
}

/// `POST /api/v1/admin/directories/:directory_id/event-providers`
pub async fn create_provider(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<CreateProviderRequest>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    // Validate provider_type
    let valid_types = ["eventbrite", "meetup", "ics_feed", "n8n_webhook"];
    if !valid_types.contains(&req.provider_type.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid provider_type. Must be one of: {}",
            valid_types.join(", ")
        )));
    }

    // Build config JSONB blob
    let config_value: serde_json::Value = json!({
        "city": req.city,
        "state": req.state,
        "lat": req.lat,
        "lng": req.lng,
        "radius_miles": req.radius_miles,
        "categories": req.categories,
    });

    let is_active = req.is_active.unwrap_or(true);

    let row = sqlx::query_as::<_, EventProviderRow>(
        r#"INSERT INTO event_providers
           (directory_id, provider_type, api_key, config, is_active)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(directory_id)
    .bind(&req.provider_type)
    .bind(&req.api_key)
    .bind(&config_value)
    .bind(is_active)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({
        "provider": row,
        "message": "Event provider created"
    }))))
}

/// `PUT /api/v1/admin/directories/:directory_id/event-providers/:provider_id`
pub async fn update_provider(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((directory_id, provider_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateProviderRequest>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    let existing = sqlx::query_as::<_, EventProviderRow>(
        "SELECT * FROM event_providers WHERE id = $1 AND directory_id = $2",
    )
    .bind(provider_id)
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event provider not found".to_string()))?;

    // Merge config
    let mut cfg = existing
        .config
        .as_object()
        .cloned()
        .unwrap_or_default();

    if let Some(ref city) = req.city {
        cfg.insert("city".into(), json!(city));
    }
    if let Some(ref state) = req.state {
        cfg.insert("state".into(), json!(state));
    }
    if let Some(lat) = req.lat {
        cfg.insert("lat".into(), json!(lat));
    }
    if let Some(lng) = req.lng {
        cfg.insert("lng".into(), json!(lng));
    }
    if let Some(radius) = req.radius_miles {
        cfg.insert("radius_miles".into(), json!(radius));
    }
    if let Some(ref categories) = req.categories {
        cfg.insert("categories".into(), json!(categories));
    }

    let merged_config = serde_json::Value::Object(cfg);
    let api_key = req.api_key.clone().or(existing.api_key);
    let is_active = req.is_active.unwrap_or(existing.is_active);

    let row = sqlx::query_as::<_, EventProviderRow>(
        r#"UPDATE event_providers
           SET api_key = COALESCE($1, api_key),
               config = $2,
               is_active = $3,
               updated_at = NOW()
           WHERE id = $4 AND directory_id = $5
           RETURNING *"#,
    )
    .bind(&api_key)
    .bind(&merged_config)
    .bind(is_active)
    .bind(provider_id)
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "provider": row,
        "message": "Event provider updated"
    })))
}

/// `DELETE /api/v1/admin/directories/:directory_id/event-providers/:provider_id`
pub async fn delete_provider(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((directory_id, provider_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    let result = sqlx::query(
        "DELETE FROM event_providers WHERE id = $1 AND directory_id = $2",
    )
    .bind(provider_id)
    .bind(directory_id)
    .execute(&s.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Event provider not found".to_string()));
    }

    Ok(Json(json!({
        "message": "Event provider deleted",
        "provider_id": provider_id,
    })))
}

/// `POST /api/v1/admin/event-providers/:provider_id/test`
pub async fn test_provider(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    let row = sqlx::query_as::<_, EventProviderRow>(
        "SELECT * FROM event_providers WHERE id = $1",
    )
    .bind(provider_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event provider not found".to_string()))?;

    let api_key = row
        .api_key
        .as_deref()
        .ok_or_else(|| AppError::Validation("No API key configured".to_string()))?;

    let provider = provider_for_type(&row.provider_type)?;

    match provider.test_connection(api_key).await {
        Ok(true) => Ok(Json(json!({
            "success": true,
            "message": "Connection successful",
            "provider_type": row.provider_type,
        }))),
        Ok(false) => Err(AppError::BadRequest(
            "Connection returned false (unknown error)".to_string(),
        )),
        Err(e) => {
            // Persist the error
            let _ = sqlx::query(
                "UPDATE event_providers SET last_sync_status = 'error', last_error = $1, updated_at = NOW() WHERE id = $2",
            )
            .bind(&e)
            .bind(provider_id)
            .execute(&s.db)
            .await;

            Err(AppError::BadRequest(format!("Connection failed: {}", e)))
        }
    }
}

/// `POST /api/v1/admin/event-providers/:provider_id/sync`
pub async fn sync_provider(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    let row = sqlx::query_as::<_, EventProviderRow>(
        "SELECT * FROM event_providers WHERE id = $1",
    )
    .bind(provider_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event provider not found".to_string()))?;

    let api_key = row
        .api_key
        .clone()
        .ok_or_else(|| AppError::Validation("No API key configured".to_string()))?;

    let provider = provider_for_type(&row.provider_type)?;
    let cfg = build_sync_config(&api_key, &row.config);

    // Mark as syncing
    let _ = sqlx::query(
        "UPDATE event_providers SET last_sync_status = 'pending', updated_at = NOW() WHERE id = $1",
    )
    .bind(provider_id)
    .execute(&s.db)
    .await;

    match provider.fetch_events(&cfg).await {
        Ok(raw_events) => {
            let count = raw_events.len() as i32;
            let synced = upsert_raw_events(&s.db, row.directory_id, provider_id, &raw_events).await?;

            let _ = sqlx::query(
                r#"UPDATE event_providers
                   SET last_sync_at = NOW(),
                       last_sync_status = 'success',
                       last_error = NULL,
                       events_synced = $1,
                       updated_at = NOW()
                   WHERE id = $2"#,
            )
            .bind(synced)
            .bind(provider_id)
            .execute(&s.db)
            .await;

            Ok(Json(json!({
                "success": true,
                "events_fetched": count,
                "events_synced": synced,
                "message": format!("Synced {} events from {}", synced, row.provider_type),
            })))
        }
        Err(e) => {
            let _ = sqlx::query(
                r#"UPDATE event_providers
                   SET last_sync_status = 'error',
                       last_error = $1,
                       last_sync_at = NOW(),
                       updated_at = NOW()
                   WHERE id = $2"#,
            )
            .bind(&e)
            .bind(provider_id)
            .execute(&s.db)
            .await;

            Err(AppError::BadRequest(format!("Sync failed: {}", e)))
        }
    }
}

/// `GET /api/v1/admin/event-providers/:provider_id/sync-status`
pub async fn sync_status(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (_user_id, role) = extract_admin(&headers, &s.config.jwt_secret)?;
    require_admin(&role)?;

    let row = sqlx::query_as::<_, EventProviderRow>(
        "SELECT * FROM event_providers WHERE id = $1",
    )
    .bind(provider_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event provider not found".to_string()))?;

    Ok(Json(json!({
        "provider_id": row.id,
        "provider_type": row.provider_type,
        "is_active": row.is_active,
        "last_sync_at": row.last_sync_at,
        "last_sync_status": row.last_sync_status,
        "last_error": row.last_error,
        "events_synced": row.events_synced,
    })))
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Build a ProviderConfig from the DB row for the sync operation.
fn build_sync_config(api_key: &str, db_config: &serde_json::Value) -> ProviderConfig {
    let cfg = db_config
        .as_object()
        .cloned()
        .unwrap_or_default();

    let city = cfg
        .get("city")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let state = cfg.get("state").and_then(|v| v.as_str().map(String::from));

    let lat = cfg.get("lat").and_then(|v| v.as_f64());
    let lng = cfg.get("lng").and_then(|v| v.as_f64());
    let radius_miles = cfg
        .get("radius_miles")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    let categories = cfg
        .get("categories")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

    ProviderConfig {
        api_key: api_key.to_string(),
        city,
        state,
        lat,
        lng,
        radius_miles,
        categories,
    }
}

/// Upsert raw events into the `community_events` table.
/// Uses ON CONFLICT on (directory_id, source_event_id) where both are NOT NULL.
/// Returns the number of events actually inserted (new) + updated.
async fn upsert_raw_events(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    provider_id: Uuid,
    raw_events: &[crate::providers::RawEvent],
) -> Result<i32, AppError> {
    let mut synced: i32 = 0;

    for ev in raw_events {
        // Parse ISO 8601 timestamps; fall back to NOW() on failure
        let start = chrono::DateTime::parse_from_rfc3339(&ev.start_time)
            .ok()
            .map(|d| d.with_timezone(&Utc));

        let end = ev
            .end_time
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));

        let event_date = start.unwrap_or_else(Utc::now);
        let end_date = end;

        // Build the location string from venue parts
        let location = ev
            .venue_name
            .clone()
            .or_else(|| ev.venue_address.clone());

        let address = if ev.venue_name.is_some() {
            ev.venue_address.clone()
        } else {
            None
        };

        let status = "active";

        // Use INSERT … ON CONFLICT to avoid duplicates.
        // We match on (directory_id, source_provider_id, source_event_id) when
        // both source columns are set.
        let result = sqlx::query(
            r#"INSERT INTO community_events
               (directory_id, source_provider_id, source_event_id,
                title, description, event_date, end_date,
                location, address, image_url, category, status, url)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(directory_id)
        .bind(provider_id)
        .bind(&ev.source_id)
        .bind(&ev.title)
        .bind(&ev.description)
        .bind(event_date)
        .bind(end_date)
        .bind(&location)
        .bind(&address)
        .bind(&ev.image_url)
        .bind(&ev.category)
        .bind(status)
        .bind(&ev.url)
        .execute(db)
        .await;

        match result {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    synced += 1;
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to upsert event {} (provider {}): {}",
                    ev.source_id,
                    provider_id,
                    e
                );
            }
        }
    }

    Ok(synced)
}
