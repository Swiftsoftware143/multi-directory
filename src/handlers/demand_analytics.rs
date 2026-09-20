//! Demand-curve analytics (T1) — the saleable data product.
//!
//! Answers, from real telemetry: **zip/city → service category → time of day
//! (and day of week) → demand volume**, plus the commercially useful rollups
//! (top category per area, peak hour per category, search-vs-view mix, unmet
//! demand, and supply so demand can be compared against it).
//!
//! Nothing here is hardwired:
//!   * windows, bucket size, top-N and export cap come from
//!     `demand_analytics_settings` (editable in the admin Demand Analytics card);
//!   * categories and directories come from the DB (`directory_categories`,
//!     `directories`) — a new category appears in the filter without a deploy;
//!   * everything is scoped by an optional `directory_id`, so the same endpoint
//!     serves one directory or the whole network.
//!
//! Thin data must never error: every aggregate COALESCEs to zero, the summary
//! reports `has_data`, and the UI renders a "not enough data yet" state.
//! All aggregation is over `visitor_events` (left-joined to `businesses` for geo
//! and `directory_categories` for the service category) so events with no
//! business attached are still counted rather than silently dropped.

use axum::{
    extract::{Query, State},
    http::header,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

// ─────────────────────────────────────────────────────────────────────────────
// Settings
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DemandSettings {
    pub id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub window_options_days: Vec<i32>,
    pub default_window_days: i32,
    pub bucket_size: String,
    pub top_categories: i32,
    pub export_row_limit: i32,
}

impl DemandSettings {
    /// Code-side defaults so a missing/partial settings row degrades instead of
    /// 500ing — the DB row is an override, never a hard requirement.
    fn fallback() -> Self {
        Self {
            id: None,
            directory_id: None,
            window_options_days: vec![7, 30, 90, 180, 365],
            default_window_days: 90,
            bucket_size: "hour".to_string(),
            top_categories: 5,
            export_row_limit: 5000,
        }
    }

    fn sanitize(mut self) -> Self {
        let fb = Self::fallback();
        if self.window_options_days.is_empty() {
            self.window_options_days = fb.window_options_days;
        }
        if self.default_window_days <= 0 {
            self.default_window_days = fb.default_window_days;
        }
        if !matches!(
            self.bucket_size.as_str(),
            "hour" | "day_of_week" | "day" | "week"
        ) {
            self.bucket_size = fb.bucket_size;
        }
        if self.top_categories <= 0 {
            self.top_categories = fb.top_categories;
        }
        if self.export_row_limit <= 0 {
            self.export_row_limit = fb.export_row_limit;
        }
        self
    }
}

/// Resolve the effective settings for a scope: the directory override if one
/// exists, otherwise the global row (directory_id IS NULL), otherwise defaults.
async fn effective_settings(
    db: &sqlx::PgPool,
    directory_id: Option<Uuid>,
) -> Result<DemandSettings, AppError> {
    let row: Option<DemandSettings> = if let Some(dir) = directory_id {
        sqlx::query_as::<_, DemandSettings>(
            "SELECT id, directory_id, window_options_days, default_window_days, bucket_size, \
                    top_categories, export_row_limit \
             FROM demand_analytics_settings WHERE directory_id = $1 LIMIT 1",
        )
        .bind(dir)
        .fetch_optional(db)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[demand] settings lookup failed, using defaults: {e}");
            None
        })
    } else {
        None
    };

    let row = match row {
        Some(r) => Some(r),
        None => sqlx::query_as::<_, DemandSettings>(
            "SELECT id, directory_id, window_options_days, default_window_days, bucket_size, \
                    top_categories, export_row_limit \
             FROM demand_analytics_settings WHERE directory_id IS NULL LIMIT 1",
        )
        .fetch_optional(db)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[demand] global settings lookup failed, using defaults: {e}");
            None
        }),
    };

    Ok(row.unwrap_or_else(DemandSettings::fallback).sanitize())
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared filter set
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct DemandFilterQuery {
    /// Scope to one directory. Omit for the whole network.
    pub directory_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    /// Free-text area match against zip OR city.
    pub area: Option<String>,
    pub city: Option<String>,
    pub zip: Option<String>,
    pub days: Option<i32>,
    pub bucket: Option<String>,
    pub limit: Option<i32>,
}

#[derive(Debug, Clone)]
struct Filters {
    days: i32,
    directory_id: Option<Uuid>,
    category_id: Option<Uuid>,
    area: Option<String>,
    city: Option<String>,
    zip: Option<String>,
}

impl Filters {
    fn from_query(q: &DemandFilterQuery, settings: &DemandSettings) -> Self {
        let days = q
            .days
            .unwrap_or(settings.default_window_days)
            .clamp(1, 3650);
        let norm = |s: &Option<String>| {
            s.as_ref()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self {
            days,
            directory_id: q.directory_id,
            category_id: q.category_id,
            area: norm(&q.area),
            city: norm(&q.city),
            zip: norm(&q.zip),
        }
    }

    fn bucket(&self, q: &DemandFilterQuery, settings: &DemandSettings) -> String {
        let b = q
            .bucket
            .clone()
            .unwrap_or_else(|| settings.bucket_size.clone());
        match b.as_str() {
            "hour" | "day_of_week" | "day" | "week" => b,
            _ => "hour".to_string(),
        }
    }

    fn limit(&self, q: &DemandFilterQuery, settings: &DemandSettings) -> i32 {
        q.limit
            .unwrap_or(settings.export_row_limit)
            .clamp(1, 100_000)
    }

    /// The WHERE clause shared by every aggregation. Bind order:
    /// $1 days, $2 directory_id, $3 category_id, $4 area, $5 city, $6 zip.
    fn where_clause() -> &'static str {
        "WHERE ve.created_at >= NOW() - make_interval(days => $1::int) \
         AND ($2::uuid IS NULL OR ve.directory_id = $2) \
         AND ($3::uuid IS NULL OR ve.category_id = $3) \
         AND ($4::text IS NULL OR COALESCE(b.zip, b.city, '') ILIKE '%' || $4 || '%') \
         AND ($5::text IS NULL OR b.city ILIKE '%' || $5 || '%') \
         AND ($6::text IS NULL OR b.zip ILIKE '%' || $6 || '%')"
    }
}

/// Bind the six standard filter parameters, in the order `where_clause` expects.
fn bind_filters<'q, O>(
    q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    f: &'q Filters,
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    q.bind(f.days)
        .bind(f.directory_id)
        .bind(f.category_id)
        .bind(f.area.as_deref())
        .bind(f.city.as_deref())
        .bind(f.zip.as_deref())
}

const JOINS: &str = "FROM visitor_events ve \
     LEFT JOIN businesses b ON b.id = ve.business_id \
     LEFT JOIN directory_categories dc ON dc.id = ve.category_id";

/// Standard zero-result predicate. Only an *explicit* zero counts: a search with
/// no `result_count` metadata is unknown, not zero — claiming otherwise would
/// invent unmet demand that was never measured. The regex guard keeps a
/// non-numeric metadata value from throwing a cast error at query time.
const ZERO_RESULT_PREDICATE: &str =
    "(ve.event_type IN ('search_zero', 'no_results', 'zero_results') \
     OR (ve.event_type IN ('search', 'search_submit') \
         AND ve.metadata->>'result_count' ~ '^[0-9]+$' \
         AND (ve.metadata->>'result_count')::int = 0))";

// ─────────────────────────────────────────────────────────────────────────────
// Rows
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DemandMatrixRow {
    pub area: String,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub category: String,
    pub category_id: Option<Uuid>,
    pub bucket_label: String,
    pub bucket_key: i32,
    pub hour_of_day: i32,
    pub day_of_week: i32,
    pub events: i64,
    pub unique_visitors: i64,
    pub sessions: i64,
    pub avg_scroll: Option<f64>,
    pub views: i64,
    pub searches: i64,
    pub phone_clicks: i64,
    pub website_clicks: i64,
    pub direction_clicks: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AreaCategoryRow {
    pub area: String,
    pub category: String,
    pub events: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PeakHourRow {
    pub category: String,
    pub hour_of_day: i32,
    pub events: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MixRow {
    pub searches: i64,
    pub views: i64,
    pub total_events: i64,
    pub other_events: i64,
    pub phone_clicks: i64,
    pub website_clicks: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UnmetDemandRow {
    pub category: String,
    pub term: Option<String>,
    pub zero_result_searches: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SupplyRow {
    pub category: String,
    pub businesses: i64,
    pub claimed: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SummaryRow {
    pub total_events: i64,
    pub distinct_areas: i64,
    pub distinct_categories: i64,
    pub distinct_visitors: i64,
    pub first_event_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_event_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CategoryOption {
    pub id: Uuid,
    pub name: String,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DirectoryOption {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Queries
// ─────────────────────────────────────────────────────────────────────────────

async fn matrix_rows(
    db: &sqlx::PgPool,
    f: &Filters,
    bucket: &str,
    limit: i32,
) -> Result<Vec<DemandMatrixRow>, AppError> {
    let sql = format!(
        "SELECT \
            COALESCE(NULLIF(b.zip, ''), NULLIF(b.city, ''), 'unknown') AS area, \
            b.city AS city, b.state AS state, b.zip AS zip, \
            COALESCE(dc.name, 'Uncategorized') AS category, \
            ve.category_id AS category_id, \
            CASE $7::text \
              WHEN 'hour' THEN to_char(ve.created_at, 'HH24:00') \
              WHEN 'day_of_week' THEN to_char(ve.created_at, 'Dy') \
              WHEN 'day' THEN to_char(ve.created_at, 'YYYY-MM-DD') \
              WHEN 'week' THEN to_char(date_trunc('week', ve.created_at), 'IYYY-\"W\"IW') \
              ELSE to_char(ve.created_at, 'HH24:00') END AS bucket_label, \
            (CASE $7::text \
              WHEN 'hour' THEN EXTRACT(HOUR FROM ve.created_at)::int \
              WHEN 'day_of_week' THEN EXTRACT(DOW FROM ve.created_at)::int \
              WHEN 'day' THEN (EXTRACT(EPOCH FROM date_trunc('day', ve.created_at)) / 86400)::int \
              WHEN 'week' THEN (EXTRACT(EPOCH FROM date_trunc('week', ve.created_at)) / 604800)::int \
              ELSE EXTRACT(HOUR FROM ve.created_at)::int END) AS bucket_key, \
            EXTRACT(HOUR FROM ve.created_at)::int AS hour_of_day, \
            EXTRACT(DOW FROM ve.created_at)::int AS day_of_week, \
            COUNT(*)::bigint AS events, \
            COUNT(DISTINCT ve.visitor_id)::bigint AS unique_visitors, \
            COUNT(DISTINCT ve.session_id)::bigint AS sessions, \
            AVG(ve.scroll_depth)::float8 AS avg_scroll, \
            COUNT(*) FILTER (WHERE ve.event_type IN ('page_view', 'listing_view', 'business_view'))::bigint AS views, \
            COUNT(*) FILTER (WHERE ve.event_type IN ('search', 'search_submit'))::bigint AS searches, \
            COUNT(*) FILTER (WHERE ve.event_type = 'phone_click')::bigint AS phone_clicks, \
            COUNT(*) FILTER (WHERE ve.event_type = 'website_click')::bigint AS website_clicks, \
            COUNT(*) FILTER (WHERE ve.event_type = 'direction_click')::bigint AS direction_clicks \
         {} {} \
         GROUP BY 1, 2, 3, 4, 5, 6, 7, 8, 9, 10 \
         ORDER BY events DESC, area ASC, category ASC \
         LIMIT $8",
        JOINS,
        Filters::where_clause()
    );

    let rows = bind_filters(sqlx::query_as::<_, DemandMatrixRow>(&sql), f)
        .bind(bucket.to_string())
        .bind(limit)
        .fetch_all(db)
        .await?;
    Ok(rows)
}

/// Top category per area (one row per area, biggest demand first).
async fn top_category_per_area(
    db: &sqlx::PgPool,
    f: &Filters,
    top_n: i32,
) -> Result<Vec<AreaCategoryRow>, AppError> {
    let sql = format!(
        "SELECT DISTINCT ON (area) area, category, events FROM ( \
            SELECT COALESCE(NULLIF(b.zip, ''), NULLIF(b.city, ''), 'unknown') AS area, \
                   COALESCE(dc.name, 'Uncategorized') AS category, \
                   COUNT(*)::bigint AS events \
            {} {} GROUP BY 1, 2 \
         ) s ORDER BY area ASC, events DESC LIMIT $7",
        JOINS,
        Filters::where_clause()
    );

    let rows = bind_filters(sqlx::query_as::<_, AreaCategoryRow>(&sql), f)
        .bind(top_n)
        .fetch_all(db)
        .await?;
    Ok(rows)
}

/// Peak hour of day per category.
async fn peak_hour_per_category(
    db: &sqlx::PgPool,
    f: &Filters,
    top_n: i32,
) -> Result<Vec<PeakHourRow>, AppError> {
    let sql = format!(
        "SELECT DISTINCT ON (category) category, hour_of_day, events FROM ( \
            SELECT COALESCE(dc.name, 'Uncategorized') AS category, \
                   EXTRACT(HOUR FROM ve.created_at)::int AS hour_of_day, \
                   COUNT(*)::bigint AS events \
            {} {} GROUP BY 1, 2 \
         ) s ORDER BY category ASC, events DESC LIMIT $7",
        JOINS,
        Filters::where_clause()
    );

    let rows = bind_filters(sqlx::query_as::<_, PeakHourRow>(&sql), f)
        .bind(top_n)
        .fetch_all(db)
        .await?;
    Ok(rows)
}

/// Search-vs-view mix. Counts are disjoint (FILTER, not CASE-guessed).
async fn mix(db: &sqlx::PgPool, f: &Filters) -> Result<MixRow, AppError> {
    let sql = format!(
        "SELECT \
            COUNT(*) FILTER (WHERE ve.event_type IN ('search', 'search_submit'))::bigint AS searches, \
            COUNT(*) FILTER (WHERE ve.event_type IN ('page_view', 'listing_view', 'business_view'))::bigint AS views, \
            COUNT(*)::bigint AS total_events, \
            COUNT(*) FILTER (WHERE ve.event_type NOT IN \
                ('search', 'search_submit', 'page_view', 'listing_view', 'business_view', \
                 'phone_click', 'website_click'))::bigint AS other_events, \
            COUNT(*) FILTER (WHERE ve.event_type = 'phone_click')::bigint AS phone_clicks, \
            COUNT(*) FILTER (WHERE ve.event_type = 'website_click')::bigint AS website_clicks \
         {} {}",
        JOINS,
        Filters::where_clause()
    );

    let row = bind_filters(sqlx::query_as::<_, MixRow>(&sql), f)
        .fetch_one(db)
        .await?;
    Ok(row)
}

/// Unmet demand: searches that explicitly returned nothing, by category + term.
async fn unmet_demand(
    db: &sqlx::PgPool,
    f: &Filters,
    top_n: i32,
) -> Result<Vec<UnmetDemandRow>, AppError> {
    let sql = format!(
        "SELECT COALESCE(dc.name, 'Uncategorized') AS category, ve.event_value AS term, \
                COUNT(*)::bigint AS zero_result_searches \
         {} {} AND {} \
         GROUP BY 1, 2 ORDER BY zero_result_searches DESC LIMIT $7",
        JOINS,
        Filters::where_clause(),
        ZERO_RESULT_PREDICATE
    );

    let rows = bind_filters(sqlx::query_as::<_, UnmetDemandRow>(&sql), f)
        .bind(top_n)
        .fetch_all(db)
        .await?;
    Ok(rows)
}

/// Supply side — how many businesses exist per category, so demand can be read
/// against supply. Business-scoped, not event-scoped, so it ignores the window.
async fn supply(db: &sqlx::PgPool, directory_id: Option<Uuid>) -> Result<Vec<SupplyRow>, AppError> {
    let rows = sqlx::query_as::<_, SupplyRow>(
        "SELECT COALESCE(dc.name, 'Uncategorized') AS category, \
                COUNT(*)::bigint AS businesses, \
                COUNT(*) FILTER (WHERE b.claimed)::bigint AS claimed \
         FROM businesses b \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE COALESCE(b.is_active, true) \
           AND ($1::uuid IS NULL OR b.directory_id = $1) \
         GROUP BY 1 ORDER BY businesses DESC",
    )
    .bind(directory_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

async fn summary(db: &sqlx::PgPool, f: &Filters) -> Result<SummaryRow, AppError> {
    let sql = format!(
        "SELECT COUNT(*)::bigint AS total_events, \
            COUNT(DISTINCT COALESCE(NULLIF(b.zip, ''), NULLIF(b.city, ''), 'unknown'))::bigint AS distinct_areas, \
            COUNT(DISTINCT COALESCE(dc.name, 'Uncategorized'))::bigint AS distinct_categories, \
            COUNT(DISTINCT ve.visitor_id)::bigint AS distinct_visitors, \
            MIN(ve.created_at) AS first_event_at, \
            MAX(ve.created_at) AS last_event_at \
         {} {}",
        JOINS,
        Filters::where_clause()
    );

    let row = bind_filters(sqlx::query_as::<_, SummaryRow>(&sql), f)
        .fetch_one(db)
        .await?;
    Ok(row)
}

async fn category_options(
    db: &sqlx::PgPool,
    directory_id: Option<Uuid>,
) -> Result<Vec<CategoryOption>, AppError> {
    let rows = sqlx::query_as::<_, CategoryOption>(
        "SELECT id, name, directory_id FROM directory_categories \
         WHERE ($1::uuid IS NULL OR directory_id = $1) \
         ORDER BY name ASC",
    )
    .bind(directory_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

async fn directory_options(db: &sqlx::PgPool) -> Result<Vec<DirectoryOption>, AppError> {
    let rows = sqlx::query_as::<_, DirectoryOption>(
        "SELECT id, name, slug FROM directories ORDER BY name ASC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/analytics/demand/config
///
/// Everything the Demand Analytics card needs to build its own filters: the
/// window choices, default window, bucket options and the DB-driven category and
/// directory lists. No client-side constants.
pub async fn demand_analytics_config(
    State(s): State<AppState>,
    Query(q): Query<DemandFilterQuery>,
) -> ApiResult<impl IntoResponse> {
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let categories = category_options(&s.db, q.directory_id)
        .await
        .unwrap_or_default();
    let directories = directory_options(&s.db).await.unwrap_or_default();

    Ok(Json(json!({
        "settings": settings,
        "bucket_options": ["hour", "day_of_week", "day", "week"],
        "categories": categories,
        "directories": directories,
    })))
}

/// PUT /api/v1/analytics/demand/config
///
/// Admin edit of the settings row (this is what keeps the windows and caps from
/// being hardwired). Writes the directory override when a directory_id is given,
/// otherwise the global row.
#[derive(Debug, Deserialize)]
pub struct DemandConfigUpdate {
    pub directory_id: Option<Uuid>,
    pub window_options_days: Option<Vec<i32>>,
    pub default_window_days: Option<i32>,
    pub bucket_size: Option<String>,
    pub top_categories: Option<i32>,
    pub export_row_limit: Option<i32>,
}

/// Upsert a settings row for a scope. `directory_id = None` targets the single
/// global row (the one with a NULL directory_id).
async fn save_settings(
    db: &sqlx::PgPool,
    directory_id: Option<Uuid>,
    windows: &[i32],
    default_window: i32,
    bucket: &str,
    top_categories: i32,
    export_row_limit: i32,
) -> Result<(), AppError> {
    // Every value is passed as a typed bind — no string building, no interpolation.
    sqlx::query(
        "INSERT INTO demand_analytics_settings \
            (directory_id, window_options_days, default_window_days, bucket_size, \
             top_categories, export_row_limit, updated_at) \
         SELECT $1, $2, $3, $4, $5, $6, NOW() \
         WHERE NOT EXISTS ( \
            SELECT 1 FROM demand_analytics_settings \
            WHERE directory_id IS NOT DISTINCT FROM $1 \
         )",
    )
    .bind(directory_id)
    .bind(windows)
    .bind(default_window)
    .bind(bucket)
    .bind(top_categories)
    .bind(export_row_limit)
    .execute(db)
    .await?;

    sqlx::query(
        "UPDATE demand_analytics_settings SET \
            window_options_days = $2, default_window_days = $3, bucket_size = $4, \
            top_categories = $5, export_row_limit = $6, updated_at = NOW() \
         WHERE directory_id IS NOT DISTINCT FROM $1",
    )
    .bind(directory_id)
    .bind(windows)
    .bind(default_window)
    .bind(bucket)
    .bind(top_categories)
    .bind(export_row_limit)
    .execute(db)
    .await?;

    Ok(())
}

pub async fn update_demand_analytics_config(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<DemandConfigUpdate>,
) -> ApiResult<impl IntoResponse> {
    if claims.role != "admin" && claims.role != "super_admin" {
        return Err(AppError::Forbidden(
            "Admin role required to change demand analytics settings".into(),
        ));
    }

    let current = effective_settings(&s.db, req.directory_id).await?;
    let windows: Vec<i32> = req
        .window_options_days
        .clone()
        .filter(|w| !w.is_empty() && w.iter().all(|d| *d > 0))
        .unwrap_or_else(|| current.window_options_days.clone());
    let default_window = req
        .default_window_days
        .filter(|d| *d > 0)
        .unwrap_or(current.default_window_days);
    let bucket = req
        .bucket_size
        .clone()
        .filter(|b| matches!(b.as_str(), "hour" | "day_of_week" | "day" | "week"))
        .unwrap_or_else(|| current.bucket_size.clone());
    let top_categories = req
        .top_categories
        .filter(|n| *n > 0)
        .unwrap_or(current.top_categories);
    let export_row_limit = req
        .export_row_limit
        .filter(|n| *n > 0)
        .unwrap_or(current.export_row_limit);

    save_settings(
        &s.db,
        req.directory_id,
        &windows,
        default_window,
        &bucket,
        top_categories,
        export_row_limit,
    )
    .await?;

    let effective = effective_settings(&s.db, req.directory_id).await?;

    Ok(Json(json!({
        "ok": true,
        "settings": effective,
    })))
}

/// GET /api/v1/analytics/demand/matrix
pub async fn demand_matrix(
    State(s): State<AppState>,
    Query(q): Query<DemandFilterQuery>,
) -> ApiResult<impl IntoResponse> {
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let f = Filters::from_query(&q, &settings);
    let bucket = f.bucket(&q, &settings);
    let limit = f.limit(&q, &settings);

    let rows = matrix_rows(&s.db, &f, &bucket, limit).await?;
    let summary_row = summary(&s.db, &f).await?;

    Ok(Json(json!({
        "scope": {
            "directory_id": f.directory_id,
            "category_id": f.category_id,
            "area": f.area,
            "city": f.city,
            "zip": f.zip,
            "days": f.days,
            "bucket": bucket,
            "limit": limit,
        },
        "summary": summary_row,
        "has_data": summary_row.total_events > 0,
        "row_count": rows.len(),
        "rows": rows,
    })))
}

/// GET /api/v1/analytics/demand/rollups
pub async fn demand_rollups(
    State(s): State<AppState>,
    Query(q): Query<DemandFilterQuery>,
) -> ApiResult<impl IntoResponse> {
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let f = Filters::from_query(&q, &settings);
    let top_n = settings.top_categories.clamp(1, 100);

    let summary_row = summary(&s.db, &f).await?;
    let top_categories = top_category_per_area(&s.db, &f, top_n).await?;
    let peak_hours = peak_hour_per_category(&s.db, &f, top_n).await?;
    let mix_row = mix(&s.db, &f).await?;
    let unmet = unmet_demand(&s.db, &f, top_n).await?;
    let supply_rows = supply(&s.db, f.directory_id).await?;

    Ok(Json(json!({
        "scope": {
            "directory_id": f.directory_id,
            "category_id": f.category_id,
            "area": f.area,
            "city": f.city,
            "zip": f.zip,
            "days": f.days,
            "top_n": top_n,
        },
        "summary": summary_row,
        "has_data": summary_row.total_events > 0,
        "top_category_per_area": top_categories,
        "peak_hour_per_category": peak_hours,
        "search_vs_view": mix_row,
        "unmet_demand": unmet,
        "supply_by_category": supply_rows,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
    pub scope: Option<String>,
}

fn csv_row(w: &mut csv::Writer<Vec<u8>>, fields: Vec<String>) -> Result<(), AppError> {
    w.write_record(fields)
        .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))
}

fn csv_response(w: csv::Writer<Vec<u8>>, name: &str) -> ApiResult<axum::response::Response> {
    let data = String::from_utf8(
        w.into_inner()
            .map_err(|e| AppError::Internal(format!("CSV flush error: {e}")))?,
    )
    .map_err(|e| AppError::Internal(format!("CSV encoding error: {e}")))?;

    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename={name}.csv"),
        )
        .body(axum::body::Body::from(data))
        .map_err(|e| AppError::Internal(format!("CSV response build error: {e}")))
}

/// GET /api/v1/analytics/demand/export?format=csv|json&scope=matrix|rollups|supply
///
/// The licenceable artefact: the current view as a downloadable file. CSV is built
/// in-process (no CDN, no external service) and the very same query functions feed
/// the JSON path and the UI, so an export can never drift from what is displayed.
pub async fn demand_export(
    State(s): State<AppState>,
    Query(q): Query<DemandFilterQuery>,
    Query(extra): Query<ExportQuery>,
) -> ApiResult<axum::response::Response> {
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let f = Filters::from_query(&q, &settings);
    let bucket = f.bucket(&q, &settings);
    let limit = f.limit(&q, &settings);
    let format = extra
        .format
        .clone()
        .unwrap_or_else(|| "csv".to_string())
        .to_lowercase();
    let scope = extra
        .scope
        .clone()
        .unwrap_or_else(|| "matrix".to_string())
        .to_lowercase();

    if format == "json" {
        let payload = match scope.as_str() {
            "supply" => json!({ "supply_by_category": supply(&s.db, f.directory_id).await? }),
            "rollups" => json!({
                "top_category_per_area": top_category_per_area(&s.db, &f, settings.top_categories).await?,
                "peak_hour_per_category": peak_hour_per_category(&s.db, &f, settings.top_categories).await?,
                "search_vs_view": mix(&s.db, &f).await?,
                "unmet_demand": unmet_demand(&s.db, &f, settings.top_categories).await?,
                "supply_by_category": supply(&s.db, f.directory_id).await?,
            }),
            _ => json!({ "rows": matrix_rows(&s.db, &f, &bucket, limit).await? }),
        };
        return Ok(Json(json!({
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "scope": scope,
            "days": f.days,
            "bucket": bucket,
            "data": payload,
        }))
        .into_response());
    }

    let mut w = csv::Writer::from_writer(Vec::new());

    match scope.as_str() {
        "supply" => {
            csv_row(
                &mut w,
                vec!["category".into(), "businesses".into(), "claimed".into()],
            )?;
            for r in supply(&s.db, f.directory_id).await? {
                csv_row(
                    &mut w,
                    vec![r.category, r.businesses.to_string(), r.claimed.to_string()],
                )?;
            }
            csv_response(w, "demand_supply")
        }
        "rollups" => {
            csv_row(
                &mut w,
                vec![
                    "rollup".into(),
                    "key_1".into(),
                    "key_2".into(),
                    "value".into(),
                ],
            )?;
            for r in top_category_per_area(&s.db, &f, settings.top_categories).await? {
                csv_row(
                    &mut w,
                    vec![
                        "top_category_per_area".into(),
                        r.area,
                        r.category,
                        r.events.to_string(),
                    ],
                )?;
            }
            for r in peak_hour_per_category(&s.db, &f, settings.top_categories).await? {
                csv_row(
                    &mut w,
                    vec![
                        "peak_hour_per_category".into(),
                        r.category,
                        format!("{:02}:00", r.hour_of_day),
                        r.events.to_string(),
                    ],
                )?;
            }
            for r in supply(&s.db, f.directory_id).await? {
                csv_row(
                    &mut w,
                    vec![
                        "supply_by_category".into(),
                        r.category,
                        String::new(),
                        r.businesses.to_string(),
                    ],
                )?;
            }
            csv_response(w, "demand_rollups")
        }
        _ => {
            csv_row(
                &mut w,
                vec![
                    "area".into(),
                    "city".into(),
                    "state".into(),
                    "zip".into(),
                    "category".into(),
                    "bucket".into(),
                    "hour_of_day".into(),
                    "day_of_week".into(),
                    "events".into(),
                    "unique_visitors".into(),
                    "sessions".into(),
                    "avg_scroll".into(),
                    "views".into(),
                    "searches".into(),
                    "phone_clicks".into(),
                    "website_clicks".into(),
                    "direction_clicks".into(),
                ],
            )?;
            for r in matrix_rows(&s.db, &f, &bucket, limit).await? {
                csv_row(
                    &mut w,
                    vec![
                        r.area,
                        r.city.unwrap_or_default(),
                        r.state.unwrap_or_default(),
                        r.zip.unwrap_or_default(),
                        r.category,
                        r.bucket_label,
                        r.hour_of_day.to_string(),
                        r.day_of_week.to_string(),
                        r.events.to_string(),
                        r.unique_visitors.to_string(),
                        r.sessions.to_string(),
                        r.avg_scroll.map(|v| format!("{v:.2}")).unwrap_or_default(),
                        r.views.to_string(),
                        r.searches.to_string(),
                        r.phone_clicks.to_string(),
                        r.website_clicks.to_string(),
                        r.direction_clicks.to_string(),
                    ],
                )?;
            }
            csv_response(w, "demand_matrix")
        }
    }
}
