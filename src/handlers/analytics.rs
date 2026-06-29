//! Analytics tracking and reporting handlers for Multi-Directory API.

use axum::{
    extract::{Path, State, Query},
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

// ── Data Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AnalyticsEvent {
    pub id: Uuid,
    pub event_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub session_id: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct TrackEventRequest {
    pub event_type: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub referrer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub event_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnalyticsSummary {
    pub total_page_views: i64,
    pub total_listing_views: i64,
    pub total_phone_clicks: i64,
    pub total_website_clicks: i64,
    pub total_direction_clicks: i64,
    pub total_deal_claims: i64,
    pub total_submissions: i64,
    pub top_listings: Vec<TopListing>,
    pub daily_counts: Vec<DailyCount>,
}

#[derive(Debug, Serialize)]
pub struct TopListing {
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DailyCount {
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DirectoryAnalytics {
    pub directory_id: Uuid,
    pub total_events: i64,
    pub page_views: i64,
    pub listing_views: i64,
    pub phone_clicks: i64,
    pub website_clicks: i64,
    pub direction_clicks: i64,
    pub deal_claims: i64,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// POST /api/v1/analytics/track — public, no auth needed
pub async fn track_event(
    State(s): State<AppState>,
    Json(req): Json<TrackEventRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.event_type.is_empty() {
        return Err(AppError::Validation("event_type is required".to_string()));
    }

    let event = sqlx::query_as::<_, AnalyticsEvent>(
        "INSERT INTO analytics_events (event_type, entity_type, entity_id, directory_id, metadata, referrer) VALUES (\x241, \x242, \x243, \x244, \x245::jsonb, \x246) RETURNING id, event_type, entity_type, entity_id, directory_id, metadata, ip_address, user_agent, referrer, session_id, created_at "
    )
    .bind(&req.event_type)
    .bind(&req.entity_type)
    .bind(req.entity_id)
    .bind(req.directory_id)
    .bind(&req.metadata)
    .bind(&req.referrer)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(event))))
}

/// GET /api/v1/analytics/summary — requires auth (protected route)
pub async fn get_summary(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let total_page_views = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'page_view'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let total_listing_views = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'listing_view'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let total_phone_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'phone_click'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let total_website_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'website_click'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let total_direction_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'direction_click'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let total_deal_claims = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'deal_claim'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let total_submissions = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE event_type = 'submission'"
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    // Top listings by views
    let rows = sqlx::query_as::<_, (Option<Uuid>, Option<String>, i64)>(
        "SELECT entity_id, entity_type, COUNT(*) as count FROM analytics_events WHERE event_type IN ('listing_view', 'page_view') AND entity_id IS NOT NULL GROUP BY entity_id, entity_type ORDER BY count DESC LIMIT 10 "
    )
    .fetch_all(&s.db)
    .await.unwrap_or_default();

    let top_listings: Vec<TopListing> = rows.into_iter()
        .map(|(eid, etype, cnt)| TopListing { entity_id: eid, entity_type: etype, count: cnt })
        .collect();

    // Daily counts for last 14 days
    let daily_rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT to_char(created_at::date, 'YYYY-MM-DD') as date, COUNT(*) as count FROM analytics_events WHERE created_at >= NOW() - INTERVAL '14 days' GROUP BY created_at::date ORDER BY date ASC "
    )
    .fetch_all(&s.db)
    .await.unwrap_or_default();

    let daily_counts: Vec<DailyCount> = daily_rows.into_iter()
        .map(|(d, c)| DailyCount { date: d, count: c })
        .collect();

    Ok(Json(json!(AnalyticsSummary {
        total_page_views,
        total_listing_views,
        total_phone_clicks,
        total_website_clicks,
        total_direction_clicks,
        total_deal_claims,
        total_submissions,
        top_listings,
        daily_counts,
    })))
}

/// GET /api/v1/analytics/by-directory/:directory_id
pub async fn by_directory(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = \x241 "
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    if exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let total_events = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 "
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let page_views = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 AND event_type = 'page_view'"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let listing_views = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 AND event_type = 'listing_view'"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let phone_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 AND event_type = 'phone_click'"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let website_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 AND event_type = 'website_click'"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let direction_clicks = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 AND event_type = 'direction_click'"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    let deal_claims = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events WHERE directory_id = \x241 AND event_type = 'deal_claim'"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    Ok(Json(json!(DirectoryAnalytics {
        directory_id,
        total_events,
        page_views,
        listing_views,
        phone_clicks,
        website_clicks,
        direction_clicks,
        deal_claims,
    })))
}

/// GET /api/v1/analytics/events — list recent events with pagination
pub async fn list_events(
    State(s): State<AppState>,
    Query(params): Query<ListEventsQuery>,
) -> ApiResult<impl IntoResponse> {
    let limit = params.limit.unwrap_or(50).min(200).max(1);
    let offset = params.offset.unwrap_or(0).max(0);

    let events = if let Some(ref et) = params.event_type {
        sqlx::query_as::<_, AnalyticsEvent>(
            "SELECT id, event_type, entity_type, entity_id, directory_id, metadata, ip_address, user_agent, referrer, session_id, created_at FROM analytics_events WHERE event_type = \x241 ORDER BY created_at DESC LIMIT \x242 OFFSET \x243 "
        )
        .bind(et)
        .bind(limit)
        .bind(offset)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, AnalyticsEvent>(
            "SELECT id, event_type, entity_type, entity_id, directory_id, metadata, ip_address, user_agent, referrer, session_id, created_at FROM analytics_events ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 "
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&s.db)
        .await?
    };

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_events "
    )
    .fetch_one(&s.db)
    .await.unwrap_or(0);

    Ok(Json(json!({
        "data": events,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

/// DELETE /api/v1/analytics/events/old — purge events older than 90 days
pub async fn purge_old_events(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query(
        "DELETE FROM analytics_events WHERE created_at < NOW() - INTERVAL '90 days'"
    )
    .execute(&s.db)
    .await?;

    let deleted = result.rows_affected();

    Ok(Json(json!({
        "message": format!("Purged {} old analytics events", deleted),
        "deleted_count": deleted,
    })))
}
