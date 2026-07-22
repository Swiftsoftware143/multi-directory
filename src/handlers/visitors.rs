//! Visitor tracking handlers for Multi-Directory API.
//! Tracks anonymous visitors, sessions, events, and business owner claims.

use axum::{
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use std::env;

use crate::AppState;
use crate::auth::models::Claims;
use crate::auth::middleware::verify_token;
use crate::error::{AppError, ApiResult};

// ── Auth Helpers (used by handlers that are before the auth_guard middleware) ──

/// Extract and verify visitor JWT from Authorization header. Returns 401 if missing/invalid.
pub fn extract_visitor_id(headers: &HeaderMap, jwt_secret: &str) -> Result<Uuid, AppError> {
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

/// Extract visitor ID from JWT if present, returns None if no auth header or invalid token.
pub fn extract_visitor_id_optional(headers: &HeaderMap, jwt_secret: &str) -> Option<Uuid> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())?;
    
    let token = auth_header.strip_prefix("Bearer ")?;
    
    let claims = verify_token(token, jwt_secret).ok()?;
    
    Uuid::parse_str(&claims.sub).ok()
}

// ── Data Types ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Visitor {
    pub id: Uuid,
    pub fingerprint: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub language: Option<String>,
    pub screen_resolution: Option<String>,
    pub timezone: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub isp: Option<String>,
    pub is_claimed_owner: Option<bool>,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct VisitorSession {
    pub id: Uuid,
    pub visitor_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub referrer: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_term: Option<String>,
    pub utm_content: Option<String>,
    pub landing_page: Option<String>,
    pub exit_page: Option<String>,
    pub pages_viewed: Option<i32>,
    pub scroll_depth_pct: Option<i32>,
    pub time_on_page_secs: Option<i32>,
    pub is_bounce: Option<bool>,
    pub duration_secs: Option<i32>,
    pub entry_url: Option<String>,
    pub exit_url: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct TrackVisitorRequest {
    pub fingerprint: Option<String>,
    pub directory_id: Option<Uuid>,
    pub language: Option<String>,
    pub screen_resolution: Option<String>,
    pub timezone: Option<String>,
    pub referrer: Option<String>,
    pub page_url: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_term: Option<String>,
    pub utm_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrackPageViewRequest {
    pub session_id: Uuid,
    pub page_url: String,
    pub referrer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrackEventRequest {
    pub session_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub event_type: String,
    pub event_value: Option<String>,
    pub page_url: Option<String>,
    pub scroll_depth: Option<i32>,
    pub duration_ms: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct EndSessionRequest {
    pub exit_page: Option<String>,
    pub pages_viewed: Option<i32>,
    pub scroll_depth_pct: Option<i32>,
    pub duration_secs: Option<i32>,
    pub is_bounce: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct VisitorSummary {
    pub total_visitors: i64,
    pub unique_visitors_30d: i64,
    pub total_sessions: i64,
    pub avg_session_duration: f64,
    pub bounce_rate: f64,
    pub avg_scroll_depth: f64,
    pub locations: Vec<LocationCount>,
    pub devices: Vec<DeviceCount>,
    pub daily_visitors: Vec<DailyVisitorCount>,
}

#[derive(Debug, Serialize)]
pub struct LocationCount {
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DeviceCount {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DailyVisitorCount {
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct BusinessVisitorSummary {
    pub total_views: i64,
    pub phone_clicks: i64,
    pub website_clicks: i64,
    pub direction_clicks: i64,
    pub views_this_week: i64,
    pub views_last_week: i64,
    pub weekly_change_pct: f64,
    pub top_search_queries: Vec<SearchQuery>,
    pub visitor_locations: Vec<LocationCount>,
    pub views_over_time: Vec<DailyVisitorCount>,
    // Category-level stats
    pub category_name: Option<String>,
    pub category_total_views: i64,
    pub category_share_pct: f64,
    pub category_rank: i32,
}

#[derive(Debug, Serialize)]
pub struct SearchQuery {
    pub query: String,
    pub count: i64,
}

// ── Handlers ──

/// POST /api/v1/visitors/track — called on every page load (no auth needed)
pub async fn track_visitor(
    State(s): State<AppState>,
    Json(req): Json<TrackVisitorRequest>,
) -> ApiResult<impl IntoResponse> {
    // Get IP from request (inject via extension in middleware or use a placeholder)
    let ip_addr = "auto".to_string();
    let ua = "auto".to_string();

    // Upsert visitor by fingerprint
    let visitor = if let Some(ref fp) = req.fingerprint {
        let existing = sqlx::query_as::<_, Visitor>(
            "SELECT * FROM visitors WHERE fingerprint = $1"
        )
        .bind(fp)
        .fetch_optional(&s.db)
        .await?;

        if let Some(v) = existing {
            sqlx::query("UPDATE visitors SET last_seen_at = NOW(), user_agent = COALESCE($1, user_agent), ip_address = COALESCE($2, ip_address), language = COALESCE($3, language), screen_resolution = COALESCE($4, screen_resolution), timezone = COALESCE($5, timezone) WHERE id = $6")
                .bind(&ua)
                .bind(&ip_addr)
                .bind(&req.language)
                .bind(&req.screen_resolution)
                .bind(&req.timezone)
                .bind(v.id)
                .execute(&s.db)
                .await?;
            v
        } else {
            sqlx::query_as::<_, Visitor>(
                "INSERT INTO visitors (fingerprint, user_agent, ip_address, language, screen_resolution, timezone) VALUES ($1, $2, $3, $4, $5, $6) RETURNING *"
            )
            .bind(fp)
            .bind(&ua)
            .bind(&ip_addr)
            .bind(&req.language)
            .bind(&req.screen_resolution)
            .bind(&req.timezone)
            .fetch_one(&s.db)
            .await?
        }
    } else {
        // No fingerprint — create anonymous visitor
        sqlx::query_as::<_, Visitor>(
            "INSERT INTO visitors (user_agent, ip_address, language, screen_resolution, timezone) VALUES ($1, $2, $3, $4, $5) RETURNING *"
        )
        .bind(&ua)
        .bind(&ip_addr)
        .bind(&req.language)
        .bind(&req.screen_resolution)
        .bind(&req.timezone)
        .fetch_one(&s.db)
        .await?
    };

    // Create a new session
    let session = sqlx::query_as::<_, VisitorSession>(
        "INSERT INTO visitor_sessions (visitor_id, directory_id, referrer, utm_source, utm_medium, utm_campaign, utm_term, utm_content, landing_page, entry_url) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) RETURNING *"
    )
    .bind(visitor.id)
    .bind(req.directory_id)
    .bind(&req.referrer)
    .bind(&req.utm_source)
    .bind(&req.utm_medium)
    .bind(&req.utm_campaign)
    .bind(&req.utm_term)
    .bind(&req.utm_content)
    .bind(&req.page_url)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "visitor_id": visitor.id,
        "session_id": session.id,
    })))
}

/// POST /api/v1/visitors/page-view — track a page view within a session
pub async fn track_page_view(
    State(s): State<AppState>,
    Json(req): Json<TrackPageViewRequest>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE visitor_sessions SET pages_viewed = pages_viewed + 1, exit_page = $2, exit_url = $2 WHERE id = $1"
    )
    .bind(req.session_id)
    .bind(&req.page_url)
    .execute(&s.db)
    .await?;

    Ok(StatusCode::OK)
}

/// POST /api/v1/visitors/event — track a visitor event (click, scroll, etc)
pub async fn track_visitor_event(
    State(s): State<AppState>,
    Json(req): Json<TrackEventRequest>,
) -> ApiResult<impl IntoResponse> {
    // Get visitor_id from session
    let visitor_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT visitor_id FROM visitor_sessions WHERE id = $1"
    )
    .bind(req.session_id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    sqlx::query(
        "INSERT INTO visitor_events (visitor_id, session_id, directory_id, business_id, category_id, event_type, event_value, metadata, page_url, scroll_depth, duration_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9, $10, $11)"
    )
    .bind(visitor_id)
    .bind(req.session_id)
    .bind(req.directory_id)
    .bind(req.business_id)
    .bind(req.category_id)
    .bind(&req.event_type)
    .bind(&req.event_value)
    .bind(&serde_json::Value::Null)
    .bind(&req.page_url)
    .bind(req.scroll_depth.unwrap_or(0))
    .bind(req.duration_ms.unwrap_or(0))
    .execute(&s.db)
    .await?;

    Ok(StatusCode::CREATED)
}

/// POST /api/v1/visitors/session/:id/end — end a session
pub async fn end_session(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<EndSessionRequest>,
) -> ApiResult<impl IntoResponse> {
    let is_bounce = req.is_bounce.unwrap_or(true);
    sqlx::query(
        "UPDATE visitor_sessions SET ended_at = NOW(), exit_page = $2, exit_url = $2, pages_viewed = COALESCE($3, pages_viewed), scroll_depth_pct = COALESCE($4, scroll_depth_pct), duration_secs = COALESCE($5, duration_secs), is_bounce = $6 WHERE id = $1"
    )
    .bind(id)
    .bind(&req.exit_page)
    .bind(req.pages_viewed)
    .bind(req.scroll_depth_pct)
    .bind(req.duration_secs)
    .bind(is_bounce)
    .execute(&s.db)
    .await?;

    Ok(StatusCode::OK)
}

/// GET /api/v1/visitors/summary — aggregated visitor stats (auth required)
pub async fn get_visitor_summary(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let total_visitors = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM visitors")
        .fetch_one(&s.db).await.unwrap_or(0);

    let unique_visitors_30d = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT visitor_id) FROM visitor_sessions WHERE started_at >= NOW() - INTERVAL '30 days'"
    ).fetch_one(&s.db).await.unwrap_or(0);

    let total_sessions = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM visitor_sessions")
        .fetch_one(&s.db).await.unwrap_or(0);

    let avg_session = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(duration_secs) FROM visitor_sessions WHERE duration_secs > 0"
    ).fetch_one(&s.db).await.unwrap_or(None).unwrap_or(0.0);

    let bounces = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_sessions WHERE is_bounce = true"
    ).fetch_one(&s.db).await.unwrap_or(0);

    let bounce_rate = if total_sessions > 0 { (bounces as f64 / total_sessions as f64) * 100.0 } else { 0.0 };

    let avg_scroll = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(scroll_depth_pct) FROM visitor_sessions WHERE scroll_depth_pct > 0"
    ).fetch_one(&s.db).await.unwrap_or(None).unwrap_or(0.0);

    // Locations
    let locations = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, i64)>(
        "SELECT city, region, country, COUNT(*) as count FROM visitors WHERE city IS NOT NULL GROUP BY city, region, country ORDER BY count DESC LIMIT 20"
    ).fetch_all(&s.db).await.unwrap_or_default();

    let locs: Vec<LocationCount> = locations.into_iter()
        .map(|(c, r, co, cnt)| LocationCount { city: c, region: r, country: co, count: cnt })
        .collect();

    // Daily visitors last 30 days
    let daily = sqlx::query_as::<_, (String, i64)>(
        "SELECT to_char(started_at::date, 'YYYY-MM-DD') as date, COUNT(DISTINCT visitor_id) as count FROM visitor_sessions WHERE started_at >= NOW() - INTERVAL '30 days' GROUP BY started_at::date ORDER BY date"
    ).fetch_all(&s.db).await.unwrap_or_default();

    let days: Vec<DailyVisitorCount> = daily.into_iter()
        .map(|(d, c)| DailyVisitorCount { date: d, count: c })
        .collect();

    Ok(Json(json!(VisitorSummary {
        total_visitors,
        unique_visitors_30d,
        total_sessions,
        avg_session_duration: avg_session,
        bounce_rate,
        avg_scroll_depth: avg_scroll,
        locations: locs,
        devices: vec![],
        daily_visitors: days,
    })))
}

/// GET /api/v1/visitors/export/:directory_id — download visitor data as JSON
pub async fn export_visitors(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let sessions = sqlx::query_as::<_, VisitorSession>(
        "SELECT vs.* FROM visitor_sessions vs WHERE vs.directory_id = $1 ORDER BY vs.started_at DESC LIMIT 1000"
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "directory_id": directory_id,
        "exported_at": Utc::now(),
        "total_sessions": sessions.len(),
        "sessions": sessions,
    })))
}

/// GET /api/v1/visitors/business/:business_id — per-business visitor summary for owner dashboard
pub async fn business_visitor_summary(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let total_views = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_events WHERE business_id = $1 AND event_type = 'listing_view'"
    )
    .bind(business_id)
    .fetch_one(&s.db).await.unwrap_or(0);

    let phone_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_events WHERE business_id = $1 AND event_type = 'phone_click'"
    )
    .bind(business_id)
    .fetch_one(&s.db).await.unwrap_or(0);

    let website_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_events WHERE business_id = $1 AND event_type = 'website_click'"
    )
    .bind(business_id)
    .fetch_one(&s.db).await.unwrap_or(0);

    let direction_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_events WHERE business_id = $1 AND event_type = 'direction_click'"
    )
    .bind(business_id)
    .fetch_one(&s.db).await.unwrap_or(0);

    let views_this_week = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_events WHERE business_id = $1 AND event_type = 'listing_view' AND created_at >= DATE_TRUNC('week', NOW())"
    )
    .bind(business_id)
    .fetch_one(&s.db).await.unwrap_or(0);

    let views_last_week = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_events WHERE business_id = $1 AND event_type = 'listing_view' AND created_at >= DATE_TRUNC('week', NOW()) - INTERVAL '7 days' AND created_at < DATE_TRUNC('week', NOW())"
    )
    .bind(business_id)
    .fetch_one(&s.db).await.unwrap_or(0);

    let weekly_change_pct = if views_last_week > 0 {
        ((views_this_week as f64 - views_last_week as f64) / views_last_week as f64) * 100.0
    } else {
        0.0
    };

    // Locations of viewers
    let locations = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>, i64)>(
        r#"SELECT v.city, v.region, v.country, COUNT(*) as count
           FROM visitor_events ve
           JOIN visitors v ON v.id = ve.visitor_id
           WHERE ve.business_id = $1 AND v.city IS NOT NULL
           GROUP BY v.city, v.region, v.country
           ORDER BY count DESC LIMIT 10"#
    )
    .bind(business_id)
    .fetch_all(&s.db).await.unwrap_or_default();

    let locs: Vec<LocationCount> = locations.into_iter()
        .map(|(c, r, co, cnt)| LocationCount { city: c, region: r, country: co, count: cnt })
        .collect();

    // Views over time (last 30 days)
    let daily = sqlx::query_as::<_, (String, i64)>(
        r#"SELECT to_char(ve.created_at::date, 'YYYY-MM-DD') as date, COUNT(*) as count
           FROM visitor_events ve
           WHERE ve.business_id = $1 AND ve.event_type = 'listing_view' AND ve.created_at >= NOW() - INTERVAL '30 days'
           GROUP BY ve.created_at::date ORDER BY date"#
    )
    .bind(business_id)
    .fetch_all(&s.db).await.unwrap_or_default();

    let days: Vec<DailyVisitorCount> = daily.into_iter()
        .map(|(d, c)| DailyVisitorCount { date: d, count: c })
        .collect();

    // ── Category-level stats ──
    // Get category info for this business
    let biz = sqlx::query_as::<_, crate::models::Business>(
        "SELECT * FROM businesses WHERE id = $1"
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?;

    let (category_name, category_total_views, category_share_pct, category_rank) =
        if let Some(ref biz_row) = biz {
            if let Some(cat_id) = biz_row.category_id {
                // Get category name
                let cat_name: Option<String> = sqlx::query_scalar(
                    "SELECT name FROM directory_categories WHERE id = $1"
                )
                .bind(cat_id)
                .fetch_optional(&s.db)
                .await?
                .flatten();

                // Count all listing_view events in this category (last 30 days)
                let cat_total: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM visitor_events ve
                     JOIN businesses b ON b.id = ve.business_id
                     WHERE b.category_id = $1
                       AND ve.event_type = 'listing_view'
                       AND ve.created_at >= NOW() - INTERVAL '30 days'"
                )
                .bind(cat_id)
                .fetch_one(&s.db)
                .await
                .unwrap_or(0);

                // Calculate share
                let share = if cat_total > 0 {
                    (total_views as f64 / cat_total as f64) * 100.0
                } else {
                    0.0
                };

                // Rank this business among others in the same category (by listing views)
                let rank: Option<i32> = sqlx::query_scalar(
                    "SELECT rn FROM (
                       SELECT b.id, ROW_NUMBER() OVER (ORDER BY COUNT(ve.id) DESC) as rn
                       FROM businesses b
                       LEFT JOIN visitor_events ve ON ve.business_id = b.id AND ve.event_type = 'listing_view'
                       WHERE b.category_id = $1
                       GROUP BY b.id
                     ) ranked WHERE id = $2"
                )
                .bind(cat_id)
                .bind(business_id)
                .fetch_optional(&s.db)
                .await?
                .flatten();

                (cat_name, cat_total, share, rank.unwrap_or(0))
            } else {
                (None, 0i64, 0.0f64, 0i32)
            }
        } else {
            (None, 0i64, 0.0f64, 0i32)
        };

    Ok(Json(json!(BusinessVisitorSummary {
        total_views,
        phone_clicks,
        website_clicks,
        direction_clicks,
        views_this_week,
        views_last_week,
        weekly_change_pct,
        top_search_queries: vec![],
        visitor_locations: locs,
        views_over_time: days,
        category_name,
        category_total_views,
        category_share_pct,
        category_rank,
    })))
}

/// GET /api/v1/visitors/category-summary/:directory_id — per-category visitor stats for admin
pub async fn category_visitor_summary(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query_as::<_, CategoryVisitorSummaryRow>(
        r#"SELECT dc.id, dc.name, dc.parent_id,
                  COALESCE(COUNT(DISTINCT ve.visitor_id), 0) as unique_visitors,
                  COALESCE(COUNT(*), 0) as total_events,
                  COALESCE(COUNT(DISTINCT ve.business_id), 0) as businesses_clicked,
                  COALESCE(COUNT(*) FILTER (WHERE ve.event_type = 'listing_view'), 0) as listing_views,
                  COALESCE(COUNT(*) FILTER (WHERE ve.event_type = 'phone_click'), 0) as phone_clicks,
                  COALESCE(COUNT(*) FILTER (WHERE ve.event_type = 'website_click'), 0) as website_clicks
           FROM directory_categories dc
           LEFT JOIN businesses b ON b.category_id = dc.id
           LEFT JOIN visitor_events ve ON ve.business_id = b.id
           WHERE dc.directory_id = $1
           GROUP BY dc.id, dc.name, dc.parent_id
           ORDER BY unique_visitors DESC"#
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(rows)))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CategoryVisitorSummaryRow {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub unique_visitors: i64,
    pub total_events: i64,
    pub businesses_clicked: i64,
    pub listing_views: i64,
    pub phone_clicks: i64,
    pub website_clicks: i64,
}

// ── Claimed Business Handlers ──

#[derive(Debug, Deserialize)]
pub struct ClaimBusinessRequest {
    pub owner_email: String,
    pub owner_name: Option<String>,
    pub owner_phone: Option<String>,
    pub website: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ClaimedBusiness {
    pub id: Uuid,
    pub business_id: Uuid,
    pub owner_email: String,
    pub owner_name: Option<String>,
    pub owner_phone: Option<String>,
    pub verification_method: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
    pub is_active: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
}

// ── Visitor Favorites (Saved Places) ──

/// POST /api/v1/visitor/favorites/{business_id} — toggle favorite (add if not exists, remove if exists)
pub async fn toggle_favorite(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    // Manually verify JWT from Authorization header (route is before auth_guard)
    let visitor_id = extract_visitor_id(&headers, &s.config.jwt_secret)?;

    // Get the business's directory_id
    let biz_info = sqlx::query_as::<_, (Uuid,)>(
        "SELECT directory_id FROM businesses WHERE id = $1"
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;

    let directory_id = biz_info.0;

    // Check if already favorited
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_favorites WHERE visitor_account_id = $1 AND business_id = $2"
    )
    .bind(visitor_id)
    .bind(business_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let saved = if existing > 0 {
        // Remove
        sqlx::query(
            "DELETE FROM visitor_favorites WHERE visitor_account_id = $1 AND business_id = $2"
        )
        .bind(visitor_id)
        .bind(business_id)
        .execute(&s.db)
        .await?;
        false
    } else {
        // Add
        sqlx::query(
            "INSERT INTO visitor_favorites (visitor_account_id, business_id, directory_id) VALUES ($1, $2, $3)"
        )
        .bind(visitor_id)
        .bind(business_id)
        .bind(directory_id)
        .execute(&s.db)
        .await?;
        true
    };

    Ok(Json(json!({
        "saved": saved,
        "business_id": business_id,
    })))
}

/// GET /api/v1/visitor/favorites — list all saved businesses for the logged-in visitor
pub async fn list_favorites(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    // Manually verify JWT from Authorization header (route is before auth_guard)
    let visitor_id = extract_visitor_id(&headers, &s.config.jwt_secret)?;

    let favorites = sqlx::query_as::<_, FavoriteBusinessRow>(
        r#"SELECT
            vf.id,
            vf.created_at as saved_at,
            b.id as business_id,
            b.name as business_name,
            b.slug as business_slug,
            b.city,
            b.state,
            dc.name as category_name,
            b.images,
            b.rating,
            b.review_count,
            b.phone,
            d.slug as directory_slug
        FROM visitor_favorites vf
        JOIN businesses b ON b.id = vf.business_id
        LEFT JOIN directory_categories dc ON dc.id = b.category_id
        JOIN directories d ON d.id = vf.directory_id
        WHERE vf.visitor_account_id = $1
        ORDER BY vf.created_at DESC"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "favorites": favorites,
        "count": favorites.len(),
    })))
}

/// GET /api/v1/visitor/favorites/check/{business_id} — check if a business is saved
pub async fn check_favorite(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    // Try to get visitor ID from JWT if present, otherwise return saved=false
    let visitor_id = match extract_visitor_id_optional(&headers, &s.config.jwt_secret) {
        Some(id) => id,
        None => {
            return Ok(Json(json!({
                "saved": false,
                "business_id": business_id,
            })));
        }
    };

    let saved = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_favorites WHERE visitor_account_id = $1 AND business_id = $2"
    )
    .bind(visitor_id)
    .bind(business_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0) > 0;

    Ok(Json(json!({
        "saved": saved,
        "business_id": business_id,
    })))
}

/// GET /api/v1/visitor/favorites/count/{business_id} — public bookmark count for a business
pub async fn get_bookmark_count(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_favorites WHERE business_id = $1"
    )
    .bind(business_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    Ok(Json(json!({
        "count": count,
        "business_id": business_id,
    })))
}

/// POST /api/v1/bookmarks/toggle — toggle bookmark via Query params
/// Alternative endpoint that uses query params instead of path + claims.
/// Expects visitor_account_id and business_id as query params.
/// This is called from the business detail page JS.
#[derive(Debug, Deserialize)]
pub struct ToggleBookmarkQuery {
    pub visitor_account_id: Option<Uuid>,
    pub business_id: Uuid,
}
pub async fn toggle_bookmark(
    State(s): State<AppState>,
    Json(req): Json<ToggleBookmarkQuery>,
) -> ApiResult<impl IntoResponse> {
    // If no visitor account id is provided, try claims extension (JWT auth)
    let visitor_id = req.visitor_account_id;
    
    let visitor_id = match visitor_id {
        Some(id) => id,
        None => {
            return Err(AppError::Unauthorized);
        }
    };

    // Get the business's directory_id
    let biz_info = sqlx::query_as::<_, (Uuid,)>(
        "SELECT directory_id FROM businesses WHERE id = $1"
    )
    .bind(req.business_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;

    let directory_id = biz_info.0;

    // Check if already favorited
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_favorites WHERE visitor_account_id = $1 AND business_id = $2"
    )
    .bind(visitor_id)
    .bind(req.business_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let bookmarked = if existing > 0 {
        // Remove
        sqlx::query(
            "DELETE FROM visitor_favorites WHERE visitor_account_id = $1 AND business_id = $2"
        )
        .bind(visitor_id)
        .bind(req.business_id)
        .execute(&s.db)
        .await?;
        false
    } else {
        // Add
        sqlx::query(
            "INSERT INTO visitor_favorites (visitor_account_id, business_id, directory_id) VALUES ($1, $2, $3)"
        )
        .bind(visitor_id)
        .bind(req.business_id)
        .bind(directory_id)
        .execute(&s.db)
        .await?;
        true
    };

    // Get updated count
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_favorites WHERE business_id = $1"
    )
    .bind(req.business_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    Ok(Json(json!({
        "bookmarked": bookmarked,
        "count": count,
        "business_id": req.business_id,
    })))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FavoriteBusinessRow {
    pub id: Uuid,
    pub saved_at: chrono::DateTime<Utc>,
    pub business_id: Uuid,
    pub business_name: String,
    pub business_slug: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub category_name: Option<String>,
    pub images: Option<serde_json::Value>,
    pub rating: Option<f64>,
    pub review_count: Option<i32>,
    pub phone: Option<String>,
    pub directory_slug: Option<String>,
}

// ── Business Claim Handlers ──

/// POST /api/v1/businesses/:id/claim — business owner claims their listing
pub async fn claim_business(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    Json(req): Json<ClaimBusinessRequest>,
) -> ApiResult<impl IntoResponse> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM claimed_businesses WHERE business_id = $1"
    )
    .bind(business_id)
    .fetch_one(&s.db)
    .await?;

    if exists > 0 {
        return Err(AppError::Validation("Business already claimed".to_string()));
    }

    let cb = sqlx::query_as::<_, ClaimedBusiness>(
        "INSERT INTO claimed_businesses (business_id, owner_email, owner_name, owner_phone) VALUES ($1, $2, $3, $4) RETURNING *"
    )
    .bind(business_id)
    .bind(&req.owner_email)
    .bind(&req.owner_name)
    .bind(&req.owner_phone)
    .fetch_one(&s.db)
    .await?;

    // Push to CoreSwift CRM (fire-and-forget, log on failure)
    let db = s.db.clone();
    let biz_id = business_id;
    let email = req.owner_email.clone();
    let name = req.owner_name.clone();
    let phone = req.owner_phone.clone();
    tokio::spawn(async move {
        match crate::coreswift::push_claimed_business(&db, biz_id, &email, name.as_deref(), phone.as_deref()).await {
            Ok(_) => tracing::info!("[claim] CoreSwift push OK for business {biz_id}"),
            Err(e) => tracing::warn!("[claim] CoreSwift push failed for business {biz_id}: {e}"),
        }
    });

    // Fire cross-platform tag sync (fire-and-forget)
    // Look up business city + directory slug, then fire the tag sync
    let ts_db = s.db.clone();
    let ts_biz_id = business_id;
    let ts_email = req.owner_email.clone();
    let ts_name = req.owner_name.clone();
    let ts_phone = req.owner_phone.clone();
    tokio::spawn(async move {
        let biz_info = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT b.directory_id, COALESCE(b.city, d.slug, '') FROM businesses b LEFT JOIN directories d ON d.id = b.directory_id WHERE b.id = $1"
        )
        .bind(ts_biz_id)
        .fetch_optional(&ts_db)
        .await;

        if let Ok(Some((dir_id, city_or_slug))) = biz_info {
            let dir_slug: String = sqlx::query_scalar(
                "SELECT slug FROM directories WHERE id = $1"
            )
            .bind(dir_id)
            .fetch_optional(&ts_db)
            .await
            .unwrap_or(None)
            .flatten()
            .unwrap_or_default();

            let city = if city_or_slug.is_empty() { dir_slug.replace("-", " ") } else { city_or_slug };
            let tags = vec!["Business".to_string(), city.clone()];
            let city_list_name = if city.is_empty() {
                String::new()
            } else {
                format!("{} - Businesses", city)
            };

            crate::handlers::tag_sync::fire_tag_sync(
                &ts_db,
                ts_email,
                Some(ts_name.unwrap_or_else(|| "Business".to_string())),
                Some("Owner".to_string()),
                ts_phone,
                tags,
                Some(city_list_name),
                Some("businesses".to_string()),
                Some(dir_slug),
                Some("business_signup".to_string()),
                None,
                None,
            );
        } else {
            tracing::warn!("[claim] Could not resolve directory info for business {ts_biz_id}, skipping tag sync");
        }
    });

        // Create a CRM deal record in the default pipeline
    let _ = create_claim_deal(&s.db, business_id, &req.owner_name, &req.owner_email, &req.owner_phone).await;

    // Auto-fetch business images from Google Places (fire-and-forget)
    let db_fetch = s.db.clone();
    let biz_id_fetch = business_id;
    tokio::spawn(async move {
        match fetch_business_images_on_claim(&db_fetch, biz_id_fetch).await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!("[claim] Fetched {count} images for business {biz_id_fetch}");
                }
            }
            Err(e) => tracing::warn!("[claim] Image fetch failed for business {biz_id_fetch}: {e}"),
        }
    });

    // ── Auto-approval: Check if owner email matches business domain ──
    // If the claimant's email domain matches the business website domain,
    // or the business email field matches the claimant email, auto-approve.
    // The claimant submits a website URL in the claim form — use that plus the DB's website.
    let owner_email_domain = req.owner_email.split('@').nth(1).unwrap_or("").to_lowercase();
    
    // Fetch existing business info from DB
    let biz_domain_info = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT website, email FROM businesses WHERE id = $1"
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or((None, None));
    
    let (biz_website, biz_email) = biz_domain_info;
    
    // The submitted website from the claim form takes priority over the DB website
    let submitted_website = req.website.as_ref()
        .map(|w| w.trim())
        .filter(|w| !w.is_empty())
        .or_else(|| biz_website.as_ref().map(|w| w.as_str()));
    
    let mut auto_approved = false;
    let mut temp_password = String::new();
    let mut no_website = submitted_website.is_none();
    
    // Check business email field match
    if let Some(ref be) = biz_email {
        if be.to_lowercase() == req.owner_email.to_lowercase() {
            auto_approved = true;
            tracing::info!("[claim] Auto-approved {} — email matches business email field", req.owner_email);
        }
    }
    
    // Check website domain match (without url crate — manual extraction)
    if !auto_approved {
        if let Some(ref w) = submitted_website {
            // Extract host from website URL
            let w_lower = w.to_lowercase();
            let host = w_lower
                .strip_prefix("https://")
                .or_else(|| w_lower.strip_prefix("http://"))
                .or_else(|| w_lower.strip_prefix("ftp://"))
                .unwrap_or(&w_lower);
            // Take just the hostname (before first /)
            let host_clean = host.split('/').next().unwrap_or(host);
            // Remove www. prefix
            let host_clean = host_clean.strip_prefix("www.").unwrap_or(host_clean);
            // Check if email domain matches the website host, or is a subdomain of it
            if !owner_email_domain.is_empty() && (host_clean == owner_email_domain 
                || owner_email_domain.ends_with(&format!(".{}", host_clean)))
            {
                auto_approved = true;
                tracing::info!("[claim] Auto-approved {} — domain {} matches website {}", 
                    req.owner_email, owner_email_domain, host_clean);
            }
        }
    }
    
    // Save the submitted website to the business listing if provided
    if let Some(ref w) = req.website {
        let w = w.trim();
        if !w.is_empty() {
            let _ = sqlx::query(
                "UPDATE businesses SET website = $1, updated_at = NOW() WHERE id = $2 AND (website IS NULL OR website = '')"
            )
            .bind(w)
            .bind(business_id)
            .execute(&s.db)
            .await;
        }
    }
    
    if auto_approved {
        // Get directory_id for the business
        let dir_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT directory_id FROM businesses WHERE id = $1"
        )
        .bind(business_id)
        .fetch_optional(&s.db)
        .await?
        .flatten();
        
        if let Some(directory_id) = dir_id {
            // Upsert business_verifications with approved status
            let _ = sqlx::query(
                r#"INSERT INTO business_verifications (business_id, directory_id, method, status, verified_at)
                   VALUES ($1, $2, 'auto_domain', 'approved', NOW())
                   ON CONFLICT (business_id) DO UPDATE
                   SET status = 'approved', verified_at = NOW(), method = 'auto_domain', updated_at = NOW()"#
            )
            .bind(business_id)
            .bind(directory_id)
            .execute(&s.db)
            .await;
            
            // Update claimed_businesses verified_at
            let _ = sqlx::query(
                "UPDATE claimed_businesses SET verified_at = NOW(), updated_at = NOW() WHERE id = $1"
            )
            .bind(cb.id)
            .execute(&s.db)
            .await;
            
            // Create visitor account with temp password
            use argon2::{Argon2, PasswordHasher};
            use argon2::password_hash::SaltString;
            use rand::rngs::OsRng;
            
            // Generate 8-char password from random bytes
            use rand::RngCore;
            let mut bytes = [0u8; 6];
            OsRng.fill_bytes(&mut bytes);
            temp_password = hex::encode(bytes);
            
            let salt = SaltString::generate(&mut OsRng);
            let password_hash = Argon2::default()
                .hash_password(temp_password.as_bytes(), &salt)
                .map(|h| h.to_string())
                .unwrap_or_default();
            
            if !password_hash.is_empty() {
                let owner_name = req.owner_name.clone().unwrap_or_default();
                let owner_phone = req.owner_phone.clone().unwrap_or_default();
                let _ = sqlx::query(
                    r#"INSERT INTO visitor_accounts (email, password_hash, name, phone, directory_id, is_active, business_type)
                       VALUES ($1, $2, $3, $4, $5, true, 'merchant')
                       ON CONFLICT (email) DO NOTHING"#
                )
                .bind(&req.owner_email)
                .bind(&password_hash)
                .bind(&owner_name)
                .bind(&owner_phone)
                .bind(directory_id)
                .execute(&s.db)
                .await;
            }
        }
    }

    Ok((StatusCode::CREATED, Json(json!({
        "id": cb.id,
        "business_id": cb.business_id,
        "owner_email": cb.owner_email,
        "owner_name": cb.owner_name,
        "owner_phone": cb.owner_phone,
        "is_active": cb.is_active,
        "created_at": cb.created_at,
        "auto_approved": auto_approved,
        "temp_password": if auto_approved { Some(&temp_password) } else { None },
        "no_website": if auto_approved { false } else { no_website },
        "message": if auto_approved {
            "Your business has been verified! Check your email for login credentials."
        } else if no_website {
            "A website URL is required to claim a listing. Please provide your website."
        } else {
            "We couldn't automatically verify your ownership. Our team will review your claim within 24 hours."
        }
    }))))
}

/// Create a deal record in the default pipeline when a business is claimed.
async fn create_claim_deal(
    db: &sqlx::PgPool,
    business_id: Uuid,
    owner_name: &Option<String>,
    owner_email: &str,
    owner_phone: &Option<String>,
) -> Result<(), String> {
    // Get the business info
    let biz = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
        "SELECT directory_id, name, COALESCE(city, '') FROM businesses WHERE id = $1"
    )
    .bind(business_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error looking up business: {e}"))?
    .ok_or_else(|| format!("Business {business_id} not found"))?;
    
    let (directory_id, biz_name, biz_city) = biz;
    
    // Find the default pipeline for this directory (or the global one)
    let pipeline = sqlx::query_as::<_, (Uuid, serde_json::Value)>(
        r#"SELECT id, COALESCE(stages, '[]'::jsonb) FROM crm_pipelines 
           WHERE directory_id = $1 OR directory_id IS NULL 
           ORDER BY CASE WHEN directory_id = $1 THEN 0 ELSE 1 END, default_pipeline DESC
           LIMIT 1"#
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error finding pipeline: {e}"))?;
    
    let (pipeline_id, stages_json) = match pipeline {
        Some(p) => p,
        None => return Ok(()), // No pipeline configured, silently skip
    };
    
    // First stage is the default
    let first_stage: String = stages_json
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .unwrap_or("Lead")
        .to_string();
    
    let title = format!("{} - Claimed{}", biz_name, 
        if biz_city.is_empty() { String::new() } else { format!(" ({})", biz_city) }
    );
    
    let deal_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO crm_deal_records (id, title, value, currency, pipeline_id, stage, status, directory_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
    )
    .bind(deal_id)
    .bind(&title)
    .bind(None::<rust_decimal::Decimal>) // no value yet
    .bind("USD")
    .bind(pipeline_id)
    .bind(&first_stage)
    .bind("open")
    .bind(directory_id)
    .execute(db)
    .await
    .map_err(|e| format!("Failed to create deal record: {e}"))?;
    
    tracing::info!("[claim] Created deal {deal_id} for business {business_id} at stage '{first_stage}'");
    Ok(())
}

/// Auto-fetch business images from Google Places when a business is claimed.
/// Queries Places API by business name + city, pulls photo_references, constructs URLs.
async fn fetch_business_images_on_claim(db: &sqlx::PgPool, business_id: Uuid) -> Result<usize, String> {
    let api_key = match env::var("GOOGLE_PLACES_API_KEY") {
        Ok(k) => k,
        Err(_) => return Err("GOOGLE_PLACES_API_KEY not set".to_string()),
    };

    // Get business info
    let biz = sqlx::query_as::<_, (String, String)>(
        r#"SELECT name, COALESCE(city, '') FROM businesses WHERE id = $1"#
    )
    .bind(business_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| "Business not found".to_string())?;

    let (name, city) = biz;
    let search_query = format!("{} {}", name, city).trim().to_string();
    if search_query.is_empty() {
        return Err("No search query available".to_string());
    }

    // Search for place_id
    fn urlenc(s: &str) -> String {
        s.replace(' ', "%20")
            .replace('&', "%26")
            .replace('?', "%3F")
            .replace('#', "%23")
            .replace(',', "%2C")
            .replace('"', "%22")
            .replace('<', "%3C")
            .replace('>', "%3E")
            .replace('{', "%7B")
            .replace('}', "%7D")
            .replace('|', "%7C")
            .replace('\\', "%5C")
            .replace('^', "%5E")
            .replace('~', "%7E")
            .replace('[', "%5B")
            .replace(']', "%5D")
            .replace('`', "%60")
    }

    let search_url = format!(
        "https://maps.googleapis.com/maps/api/place/findplacefromtext/json?input={}&inputtype=textquery&fields=place_id&key={}",
        urlenc(&search_query), api_key
    );

    let resp = reqwest::get(&search_url).await.map_err(|e| format!("Places search request failed: {e}"))?;
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("Places parse failed: {e}"))?;

    let place_id = body["candidates"]
        .as_array()
        .and_then(|c| c.first())
        .and_then(|c| c["place_id"].as_str())
        .map(|s| s.to_string());

    let place_id = match place_id {
        Some(p) => p,
        None => return Ok(0), // No match, skip silently
    };

    // Get place details with photos
    let details_url = format!(
        "https://maps.googleapis.com/maps/api/place/details/json?place_id={}&fields=photos&key={}",
        place_id, api_key
    );

    let resp = reqwest::get(&details_url).await.map_err(|e| format!("Places details request failed: {e}"))?;
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("Places details parse failed: {e}"))?;

    let photos = body["result"]["photos"]
        .as_array()
        .map(|arr| {
            arr.iter().filter_map(|p| {
                let ref_ = p["photo_reference"].as_str()?;
                Some(format!(
                    "https://maps.googleapis.com/maps/api/place/photo?maxwidth=800&photo_reference={}&key={}",
                    urlenc(ref_), api_key
                ))
            }).collect::<Vec<String>>()
        })
        .unwrap_or_default();

    if photos.is_empty() {
        return Ok(0);
    }

    let count = photos.len();
    let photos_json = serde_json::to_value(&photos).unwrap_or_default();

    // Get existing images and merge
    let existing: serde_json::Value = sqlx::query_scalar(
        r#"SELECT COALESCE(images, '[]'::jsonb) FROM businesses WHERE id = $1"#
    )
    .bind(business_id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("DB error reading existing images: {e}"))?;

    sqlx::query(
        r#"UPDATE businesses SET images = $1, updated_at = NOW() WHERE id = $2"#
    )
    .bind(&photos_json)
    .bind(business_id)
    .execute(db)
    .await
    .map_err(|e| format!("DB error updating images: {e}"))?;

    Ok(count)
}


// ── City Request / Poll Feature ──

#[derive(Debug, Deserialize)]
pub struct CityRequestInput {
    pub city_name: String,
    pub state: Option<String>,
    pub email: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CityRequestVote {
    pub id: Uuid,
    pub city_name: String,
    pub state: String,
    pub votes: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AdminCityRequest {
    pub id: Uuid,
    pub city_name: String,
    pub state: String,
    pub votes: i32,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}

/// Submit a city request (creates or upvotes)
pub async fn request_city(
    State(app_state): State<AppState>,
    Json(req): Json<CityRequestInput>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let city_name = req.city_name.trim();
    if city_name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "City name is required"}))));
    }
    
    // Check if city already exists in directory
    let slug = city_name.to_lowercase()
        .replace(' ', "-")
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "");
    
    let existing_dir: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&app_state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?
    .flatten();
    
    if existing_dir.is_some() {
        return Err((StatusCode::CONFLICT, Json(json!({"error": "This city is already listed on ZaarHub!"}))));
    }
    
    // Upsert vote
    let state = req.state.as_deref().unwrap_or("FL");
    let email = req.email.as_ref().map(|e| e.trim()).filter(|e| !e.is_empty());
    
    let result = sqlx::query_as::<_, (Uuid, i32)>(
        r#"INSERT INTO city_requests (city_name, state, email, votes)
           VALUES ($1, $2, $3, 1)
           ON CONFLICT (city_name, state) DO UPDATE
           SET votes = city_requests.votes + 1,
               updated_at = NOW()
           RETURNING id, votes"#
    )
    .bind(&city_name)
    .bind(state)
    .bind(email)
    .fetch_optional(&app_state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    
    // If we have a directory_id and the INSERT path includes it, we need to handle the ON CONFLICT
    // The query above doesn't use directory_id since it's simple upsert. 
    // The real directory_id scoping happens on the admin/managed side.
    
    match result {
        Some((id, votes)) => {
            Ok(Json(json!({
                "id": id,
                "city_name": city_name,
                "state": state,
                "votes": votes,
                "message": "Thanks! Your vote has been counted."
            })))
        },
        None => {
            Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Could not process request"}))))
        }
    }
}

/// Get all requested cities with vote counts (ordered by popularity)
pub async fn get_city_requests(
    State(app_state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let requests = sqlx::query_as::<_, CityRequestVote>(
        r#"SELECT id, city_name, state, votes, created_at
           FROM city_requests
           WHERE status = 'pending'
           ORDER BY votes DESC, created_at DESC
           LIMIT 50"#
    )
    .fetch_all(&app_state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    
    Ok(Json(json!({"requests": requests})))
}

/// Admin: get all city requests for a specific directory (including processed)
pub async fn admin_get_city_requests(
    State(app_state): State<AppState>,
    Path(dir_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let requests = sqlx::query_as::<_, AdminCityRequest>(
        r#"SELECT id, city_name, state, votes, status, created_at, processed_at
           FROM city_requests
           WHERE directory_id = $1
           ORDER BY votes DESC, created_at DESC"#
    )
    .bind(dir_id)
    .fetch_all(&app_state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    
    Ok(Json(json!({"requests": requests})))
}

/// Admin: mark a city request as added (processed)
pub async fn admin_mark_city_added(
    State(app_state): State<AppState>,
    Path((_dir_id, request_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let result = sqlx::query(
        r#"UPDATE city_requests
           SET status = 'added',
               processed_at = NOW()
           WHERE id = $1"#
    )
    .bind(request_id)
    .execute(&app_state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "City request not found"}))));
    }
    
    Ok(Json(json!({"message": "City marked as added", "id": request_id})))
}
