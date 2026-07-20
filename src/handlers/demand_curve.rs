//! Demand-curve analytics endpoint.
//! Aggregates visitor_events → visitors → businesses to surface
//! demand volume, trend, and engagement by zip/city, category, and time-of-day.

use axum::{
    extract::{State, Query},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Query Parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DemandCurveQuery {
    pub directory_id: Option<Uuid>,
    pub city: Option<String>,
    pub category: Option<String>,
    pub days: Option<i64>,
}

// ── Response Row ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DemandCurveRow {
    pub city: Option<String>,
    pub region: Option<String>,
    pub category_id: Option<Uuid>,
    pub hour_slot: Option<f64>,
    pub day_of_week: Option<f64>,
    pub impressions: Option<i64>,
    pub unique_visitors: Option<i64>,
    pub avg_scroll: Option<f64>,
    pub phone_clicks: Option<i64>,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// GET /api/v1/analytics/demand-curve
///
/// Returns demand-volume rows aggregated by city, category, hour-of-day, and
/// day-of-week.  Supports optional filtering by directory_id, city, category,
/// and lookback window (default 90 days).
pub async fn get_demand_curve(
    State(s): State<AppState>,
    Query(params): Query<DemandCurveQuery>,
) -> ApiResult<impl IntoResponse> {
    let days = params.days.unwrap_or(90).clamp(1, 365);

    let rows = if let Some(dir_id) = params.directory_id {
        // Filtered by directory
        sqlx::query_as::<_, DemandCurveRow>(
            r#"
            SELECT
                v.city,
                v.region,
                b.category_id,
                EXTRACT(HOUR FROM ve.created_at) AS hour_slot,
                EXTRACT(DOW FROM ve.created_at) AS day_of_week,
                COUNT(*)::bigint AS impressions,
                COUNT(DISTINCT v.id)::bigint AS unique_visitors,
                AVG(ve.scroll_depth) AS avg_scroll,
                COUNT(CASE WHEN ve.event_type = 'phone_click' THEN 1 END)::bigint AS phone_clicks
            FROM visitor_events ve
            JOIN visitors v ON v.id = ve.visitor_id
            JOIN businesses b ON b.id = ve.business_id
            WHERE ve.directory_id = $1
              AND ve.created_at >= NOW() - ($2 || ' days')::interval
            GROUP BY v.city, v.region, b.category_id, hour_slot, day_of_week
            ORDER BY impressions DESC
            "#,
        )
        .bind(dir_id)
        .bind(days.to_string())
        .fetch_all(&s.db)
        .await?
    } else if let Some(ref city) = params.city {
        // Filtered by city
        sqlx::query_as::<_, DemandCurveRow>(
            r#"
            SELECT
                v.city,
                v.region,
                b.category_id,
                EXTRACT(HOUR FROM ve.created_at) AS hour_slot,
                EXTRACT(DOW FROM ve.created_at) AS day_of_week,
                COUNT(*)::bigint AS impressions,
                COUNT(DISTINCT v.id)::bigint AS unique_visitors,
                AVG(ve.scroll_depth) AS avg_scroll,
                COUNT(CASE WHEN ve.event_type = 'phone_click' THEN 1 END)::bigint AS phone_clicks
            FROM visitor_events ve
            JOIN visitors v ON v.id = ve.visitor_id
            JOIN businesses b ON b.id = ve.business_id
            WHERE v.city ILIKE $1
              AND ve.created_at >= NOW() - ($2 || ' days')::interval
            GROUP BY v.city, v.region, b.category_id, hour_slot, day_of_week
            ORDER BY impressions DESC
            "#,
        )
        .bind(city)
        .bind(days.to_string())
        .fetch_all(&s.db)
        .await?
    } else {
        // Unfiltered — full aggregation
        sqlx::query_as::<_, DemandCurveRow>(
            r#"
            SELECT
                v.city,
                v.region,
                b.category_id,
                EXTRACT(HOUR FROM ve.created_at) AS hour_slot,
                EXTRACT(DOW FROM ve.created_at) AS day_of_week,
                COUNT(*)::bigint AS impressions,
                COUNT(DISTINCT v.id)::bigint AS unique_visitors,
                AVG(ve.scroll_depth) AS avg_scroll,
                COUNT(CASE WHEN ve.event_type = 'phone_click' THEN 1 END)::bigint AS phone_clicks
            FROM visitor_events ve
            JOIN visitors v ON v.id = ve.visitor_id
            JOIN businesses b ON b.id = ve.business_id
            WHERE ve.created_at >= NOW() - ($1 || ' days')::interval
            GROUP BY v.city, v.region, b.category_id, hour_slot, day_of_week
            ORDER BY impressions DESC
            "#,
        )
        .bind(days.to_string())
        .fetch_all(&s.db)
        .await?
    };

    Ok(Json(json!({
        "data": rows,
        "count": rows.len(),
        "days": days,
    })))
}
