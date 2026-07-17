//! Visitor tracking handlers for Multi-Directory API.
//! Tracks anonymous visitors, sessions, events, and business owner claims.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

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

    Ok((StatusCode::CREATED, Json(json!(cb))))
}
