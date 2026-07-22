//! Community Events with RSVP.
//! Merchants can post events (optionally linked to their business).
//! Visitors can RSVP (going / maybe / not-going).
//! Admins manage events.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::verify_token;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// ── Data Types ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CommunityEvent {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub business_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub event_date: DateTime<Utc>,
    pub end_date: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub address: Option<String>,
    pub image_url: Option<String>,
    pub category: Option<String>,
    pub status: String,
    pub max_attendees: Option<i32>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct EventWithRsvpCount {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub business_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub event_date: DateTime<Utc>,
    pub end_date: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub address: Option<String>,
    pub image_url: Option<String>,
    pub category: Option<String>,
    pub status: String,
    pub max_attendees: Option<i32>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub rsvp_count: i64,
    pub user_rsvp_status: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EventRsvp {
    pub id: Uuid,
    pub event_id: Uuid,
    pub visitor_account_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEventRequest {
    pub directory_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub event_date: DateTime<Utc>,
    pub end_date: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub address: Option<String>,
    pub image_url: Option<String>,
    pub category: Option<String>,
    pub max_attendees: Option<i32>,
    pub business_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEventRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub event_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub address: Option<String>,
    pub image_url: Option<String>,
    pub category: Option<String>,
    pub max_attendees: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RsvpRequest {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EventListQuery {
    pub directory_id: Uuid,
    pub status: Option<String>,
    pub upcoming: Option<bool>,
    pub category: Option<String>,
    pub business_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RsvpWithVisitor {
    pub id: Uuid,
    pub event_id: Uuid,
    pub visitor_account_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub visitor_name: Option<String>,
    pub visitor_email: Option<String>,
}

// ── Auth helpers ──

/// Extract admin user ID from JWT (for admin-only operations).
fn extract_admin_id(headers: &HeaderMap, jwt_secret: &str) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    if claims.role != "admin" && claims.role != "super_admin" {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

/// Extract user/admin ID from JWT (any valid JWT). Useful when we check
/// authorization in the handler rather than relying on role filters.
fn extract_user_id(headers: &HeaderMap, jwt_secret: &str) -> Result<(Uuid, String), AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    let id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    Ok((id, claims.role))
}

/// Extract visitor account ID from JWT (for RSVP operations).
fn extract_visitor_account_id(headers: &HeaderMap, jwt_secret: &str) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

/// Extract visitor ID from JWT if present (optional auth).
fn extract_visitor_id_optional(headers: &HeaderMap, jwt_secret: &str) -> Option<Uuid> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())?;

    let token = auth_header.strip_prefix("Bearer ")?;
    let claims = verify_token(token, jwt_secret).ok()?;
    Uuid::parse_str(&claims.sub).ok()
}

/// Check if a user is admin/super_admin of the directory (or the event creator).
async fn user_can_manage_event(
    db: &sqlx::PgPool,
    user_id: Uuid,
    role: &str,
    event_id: Uuid,
) -> Result<bool, AppError> {
    // Admins and superadmins can manage any event
    if role == "admin" || role == "super_admin" {
        return Ok(true);
    }

    // Check if user is the event creator
    let is_creator = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM community_events WHERE id = $1 AND created_by = $2"
    )
    .bind(event_id)
    .bind(user_id)
    .fetch_one(db)
    .await
    .unwrap_or(0) > 0;

    Ok(is_creator)
}

/// Check whether the visitor has an active RSVP for an event.
/// Used in list/detail to show user_rsvp_status.
async fn fetch_user_rsvp_status(
    db: &sqlx::PgPool,
    event_id: Uuid,
    visitor_id: Option<Uuid>,
) -> Option<String> {
    let vid = visitor_id?;
    sqlx::query_scalar::<_, String>(
        "SELECT status FROM event_rsvps WHERE event_id = $1 AND visitor_account_id = $2"
    )
    .bind(event_id)
    .bind(vid)
    .fetch_optional(db)
    .await
    .ok()?
}

// ── Handlers ──

/// POST /api/v1/events — create a community event (admin or business owner JWT)
pub async fn create_event(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateEventRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.title.trim().is_empty() {
        return Err(AppError::Validation("Title is required".to_string()));
    }

    let (user_id, role) = extract_user_id(&headers, &s.config.jwt_secret)?;
    let is_admin = role == "admin" || role == "super_admin";
    let is_business_owner = role == "business_owner";

    if !is_admin && !is_business_owner {
        return Err(AppError::Forbidden(
            "Only admins and business owners can create events".to_string(),
        ));
    }

    // If business owner, verify they own the business_id (if provided)
    if is_business_owner {
        if let Some(biz_id) = req.business_id {
            let owns_biz = sqlx::query_scalar::<_, i64>(
                r#"SELECT COUNT(*) FROM claimed_businesses
                   WHERE business_id = $1 AND (user_id = $2 OR owner_email = (
                       SELECT email FROM users WHERE id = $2
                   ))"#
            )
            .bind(biz_id)
            .bind(user_id)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0)
                > 0;

            if !owns_biz {
                return Err(AppError::Forbidden(
                    "You don't own this business".to_string(),
                ));
            }
        }
    }

    // Verify directory exists
    let dir_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = $1",
    )
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0)
        > 0;

    if !dir_exists {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let event = sqlx::query_as::<_, CommunityEvent>(
        r#"INSERT INTO community_events
           (directory_id, business_id, title, description, event_date, end_date,
            location, address, image_url, category, max_attendees, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           RETURNING *"#,
    )
    .bind(req.directory_id)
    .bind(req.business_id)
    .bind(req.title.trim())
    .bind(req.description.as_deref().map(|s| s.trim()))
    .bind(req.event_date)
    .bind(req.end_date)
    .bind(req.location.as_deref().map(|s| s.trim()))
    .bind(req.address.as_deref().map(|s| s.trim()))
    .bind(req.image_url.as_deref().map(|s| s.trim()))
    .bind(req.category.as_deref().map(|s| s.trim().to_lowercase()))
    .bind(req.max_attendees)
    .bind(user_id)
    .fetch_one(&s.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "event": event,
            "message": "Event created successfully"
        })),
    ))
}

/// GET /api/v1/events?directory_id=X&status=active&upcoming=true&category=Y&business_id=Z
/// Public — no auth required. Returns RSVP counts per event.
pub async fn list_events(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<EventListQuery>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_id_optional(&headers, &s.config.jwt_secret);

    // Fetch events matching filters
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let status = q.status.as_deref().unwrap_or("active");

    let events_raw = if q.upcoming == Some(true) {
        sqlx::query_as::<_, CommunityEvent>(
            "SELECT * FROM community_events ce
             WHERE ce.directory_id = $1 AND ce.status = $2
               AND ce.event_date >= NOW()
               AND ($3::text IS NULL OR ce.category = $3)
               AND ($4::uuid IS NULL OR ce.business_id = $4)
             ORDER BY ce.event_date ASC
             LIMIT $5 OFFSET $6"
        )
        .bind(q.directory_id)
        .bind(status)
        .bind(&q.category)
        .bind(q.business_id)
        .bind(limit)
        .bind(offset)
    } else {
        sqlx::query_as::<_, CommunityEvent>(
            "SELECT * FROM community_events ce
             WHERE ce.directory_id = $1 AND ce.status = $2
               AND ($3::text IS NULL OR ce.category = $3)
               AND ($4::uuid IS NULL OR ce.business_id = $4)
             ORDER BY ce.event_date ASC
             LIMIT $5 OFFSET $6"
        )
        .bind(q.directory_id)
        .bind(status)
        .bind(&q.category)
        .bind(q.business_id)
        .bind(limit)
        .bind(offset)
    };
    let events_raw = events_raw.fetch_all(&s.db).await?;

    // Attach RSVP counts per event
    let mut events: Vec<EventWithRsvpCount> = Vec::new();
    for event in events_raw {
        let rsvp_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM event_rsvps WHERE event_id = $1 AND status IN ('going', 'maybe')"
        )
        .bind(event.id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        let user_rsvp_status =
            fetch_user_rsvp_status(&s.db, event.id, visitor_id).await;

        events.push(EventWithRsvpCount {
            id: event.id,
            directory_id: event.directory_id,
            business_id: event.business_id,
            title: event.title,
            description: event.description,
            event_date: event.event_date,
            end_date: event.end_date,
            location: event.location,
            address: event.address,
            image_url: event.image_url,
            category: event.category,
            status: event.status,
            max_attendees: event.max_attendees,
            created_by: event.created_by,
            created_at: event.created_at,
            updated_at: event.updated_at,
            rsvp_count,
            user_rsvp_status,
        });
    }

    Ok(Json(json!({
        "events": events,
        "count": events.len(),
    })))
}

/// GET /api/v1/events/:id — single event details, public.
/// Includes RSVP count and whether current visitor has RSVP'd.
pub async fn get_event(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(event_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_id_optional(&headers, &s.config.jwt_secret);

    let event = sqlx::query_as::<_, CommunityEvent>(
        "SELECT * FROM community_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event not found".to_string()))?;

    let rsvp_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM event_rsvps WHERE event_id = $1 AND status IN ('going', 'maybe')",
    )
    .bind(event.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let user_rsvp_status = fetch_user_rsvp_status(&s.db, event.id, visitor_id).await;

    let event_with = EventWithRsvpCount {
        id: event.id,
        directory_id: event.directory_id,
        business_id: event.business_id,
        title: event.title,
        description: event.description,
        event_date: event.event_date,
        end_date: event.end_date,
        location: event.location,
        address: event.address,
        image_url: event.image_url,
        category: event.category,
        status: event.status,
        max_attendees: event.max_attendees,
        created_by: event.created_by,
        created_at: event.created_at,
        updated_at: event.updated_at,
        rsvp_count,
        user_rsvp_status,
    };

    Ok(Json(json!({ "event": event_with })))
}

/// POST /api/v1/events/:id/rsvp — RSVP for an event (visitor JWT)
pub async fn rsvp_event(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(event_id): Path<Uuid>,
    Json(req): Json<RsvpRequest>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_account_id(&headers, &s.config.jwt_secret)?;

    // Verify event exists and is active
    let event = sqlx::query_as::<_, CommunityEvent>(
        "SELECT * FROM community_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event not found".to_string()))?;

    if event.status != "active" {
        return Err(AppError::BadRequest(
            "Event is not accepting RSVPs".to_string(),
        ));
    }

    let rsvp_status = req.status.as_deref().unwrap_or("going");

    // Validate status
    match rsvp_status {
        "going" | "maybe" | "not-going" => {}
        _ => {
            return Err(AppError::Validation(
                "Status must be 'going', 'maybe', or 'not-going'".to_string(),
            ))
        }
    }

    // Check max_attendees
    if rsvp_status == "going" || rsvp_status == "maybe" {
        if let Some(max) = event.max_attendees {
            let current = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM event_rsvps WHERE event_id = $1 AND status IN ('going', 'maybe') AND visitor_account_id != $2",
            )
            .bind(event_id)
            .bind(visitor_id)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);

            if current >= max as i64 {
                return Err(AppError::BadRequest(
                    "Event is at maximum capacity".to_string(),
                ));
            }
        }
    }

    // Upsert
    sqlx::query(
        r#"INSERT INTO event_rsvps (event_id, visitor_account_id, status)
           VALUES ($1, $2, $3)
           ON CONFLICT (event_id, visitor_account_id)
           DO UPDATE SET status = $3"#,
    )
    .bind(event_id)
    .bind(visitor_id)
    .bind(rsvp_status)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "message": "RSVP recorded",
        "event_id": event_id,
        "status": rsvp_status,
    })))
}

/// GET /api/v1/events/:id/attendees — list of RSVPs with visitor info
/// Requires event creator or admin JWT.
pub async fn list_attendees(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(event_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (user_id, role) = extract_user_id(&headers, &s.config.jwt_secret)?;
    let authorized = user_can_manage_event(&s.db, user_id, &role, event_id).await?;
    if !authorized {
        return Err(AppError::Forbidden(
            "Only the event creator or admin can view attendees".to_string(),
        ));
    }

    let attendees = sqlx::query_as::<_, RsvpWithVisitor>(
        r#"SELECT
            er.id,
            er.event_id,
            er.visitor_account_id,
            er.status,
            er.created_at,
            va.name AS visitor_name,
            va.email AS visitor_email
        FROM event_rsvps er
        JOIN visitor_accounts va ON va.id = er.visitor_account_id
        WHERE er.event_id = $1
        ORDER BY er.created_at DESC"#,
    )
    .bind(event_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "attendees": attendees,
        "count": attendees.len(),
    })))
}

/// POST /api/v1/events/:id/cancel — cancel an event (admin/creator only)
pub async fn cancel_event(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(event_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (user_id, role) = extract_user_id(&headers, &s.config.jwt_secret)?;
    let authorized = user_can_manage_event(&s.db, user_id, &role, event_id).await?;
    if !authorized {
        return Err(AppError::Forbidden(
            "Only the event creator or admin can cancel events".to_string(),
        ));
    }

    let event = sqlx::query_as::<_, CommunityEvent>(
        "SELECT * FROM community_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event not found".to_string()))?;

    if event.status == "cancelled" {
        return Err(AppError::BadRequest(
            "Event is already cancelled".to_string(),
        ));
    }

    sqlx::query(
        "UPDATE community_events SET status = 'cancelled', updated_at = NOW() WHERE id = $1",
    )
    .bind(event_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "message": "Event cancelled",
        "event_id": event_id,
    })))
}

/// POST /api/v1/events/:id/edit — update event fields (admin/creator only)
pub async fn edit_event(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(event_id): Path<Uuid>,
    Json(req): Json<UpdateEventRequest>,
) -> ApiResult<impl IntoResponse> {
    let (user_id, role) = extract_user_id(&headers, &s.config.jwt_secret)?;
    let authorized = user_can_manage_event(&s.db, user_id, &role, event_id).await?;
    if !authorized {
        return Err(AppError::Forbidden(
            "Only the event creator or admin can edit events".to_string(),
        ));
    }

    // Fetch current event as baseline
    let current = sqlx::query_as::<_, CommunityEvent>(
        "SELECT * FROM community_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Event not found".to_string()))?;

    // Merge: use new value if Some, otherwise keep existing
    let new_title = req.title.clone().unwrap_or(current.title);
    let new_description = req.description.clone().or(current.description);
    let new_event_date = req.event_date.unwrap_or(current.event_date);
    let new_end_date = req.end_date.or(current.end_date);
    let new_location = req.location.clone().or(current.location);
    let new_address = req.address.clone().or(current.address);
    let new_image_url = req.image_url.clone().or(current.image_url);
    let new_category = req.category.clone().or(current.category);
    let new_max_attendees = req.max_attendees.or(current.max_attendees);

    let updated = sqlx::query_as::<_, CommunityEvent>(
        r#"UPDATE community_events
           SET title = $1, description = $2, event_date = $3, end_date = $4,
               location = $5, address = $6, image_url = $7, category = $8,
               max_attendees = $9, updated_at = NOW()
           WHERE id = $10
           RETURNING *"#,
    )
    .bind(new_title.trim())
    .bind(new_description.as_deref().map(|s| s.trim()))
    .bind(new_event_date)
    .bind(new_end_date)
    .bind(new_location.as_deref().map(|s| s.trim()))
    .bind(new_address.as_deref().map(|s| s.trim()))
    .bind(new_image_url.as_deref().map(|s| s.trim()))
    .bind(new_category.as_deref().map(|s| s.trim().to_lowercase()))
    .bind(new_max_attendees)
    .bind(event_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "event": updated,
        "message": "Event updated successfully"
    })))
}

// ── Server-rendered events page ──

/// GET /api/v1/events-page — render events.hbs template with upcoming events
pub async fn events_page(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<EventListQuery>,
) -> ApiResult<impl IntoResponse> {
    use crate::template_engine;

    let visitor_id = extract_visitor_id_optional(&headers, &s.config.jwt_secret);
    let dir_id = q.directory_id;

    // Fetch directory info
    let dir_row = sqlx::query_as::<_, (Uuid, String, Option<String>, String, String, Option<serde_json::Value>, Option<String>)>(
        r#"SELECT id, name, description, slug, template, color_scheme, region
           FROM directories WHERE id = $1"#,
    )
    .bind(dir_id)
    .fetch_optional(&s.db)
    .await?;

    let (dir_uuid, dir_name, dir_desc, dir_slug, _dir_template, color_scheme, region) =
        match dir_row {
            Some(r) => r,
            None => return Err(AppError::NotFound("Directory not found".to_string())),
        };

    let base_status = q.status.as_deref().unwrap_or("active");

    // Fetch events with RSVP counts (separate queries for type safety)
    let events_raw = sqlx::query_as::<_, CommunityEvent>(
        r#"SELECT * FROM community_events ce
         WHERE ce.directory_id = $1
           AND ce.status = $2
           AND ce.event_date >= NOW()
         ORDER BY ce.event_date ASC
         LIMIT 50"#,
    )
    .bind(dir_id)
    .bind(base_status)
    .fetch_all(&s.db)
    .await?;

    // Build event list with RSVP counts and user RSVP status
    let mut event_list: Vec<serde_json::Value> = Vec::new();
    for event in events_raw {
        let rsvp_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM event_rsvps WHERE event_id = $1 AND status IN ('going', 'maybe')"
        )
        .bind(event.id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        let user_rsvp_status =
            fetch_user_rsvp_status(&s.db, event.id, visitor_id).await;

        event_list.push(json!({
            "id": event.id,
            "title": event.title,
            "description": event.description,
            "event_date": event.event_date,
            "end_date": event.end_date,
            "location": event.location,
            "address": event.address,
            "image_url": event.image_url,
            "category": event.category,
            "status": event.status,
            "max_attendees": event.max_attendees,
            "rsvp_count": rsvp_count,
            "user_rsvp_status": user_rsvp_status,
            "business_id": event.business_id,
        }));
    }

    let colors = template_engine::normalize_color_scheme(color_scheme);

    let context = json!({
        "directory": {
            "id": dir_uuid,
            "name": dir_name,
            "description": dir_desc,
            "slug": dir_slug,
            "region": region,
        },
        "colors": colors,
        "events": event_list,
        "has_events": !event_list.is_empty(),
    });

    let engine = s.template_engine.lock().unwrap();
    let html = engine
        .render_directory_page(template_engine::TEMPLATE_EVENTS, &context)
        .map_err(|e| AppError::Internal(e))?;

    Ok(axum::response::Html(html).into_response())
}
