//! "Report a problem" on a business listing (card B90 cross-cutting) — the anonymous report
//! write plus the directory-scoped moderation queue a non-technical admin works from the panel.
//!
//! Competitor context: Angie's List / Nextdoor / Thumbtack all let a customer flag a listing as
//! closed, wrong or abusive and route it to a moderator. Until now Multi-Directory had no such
//! path, so a wrong listing could only be fixed by the agent running SQL (exactly the agent-only
//! gap card B84 forbids). This handler is the whole feature: POST is public and rate-limited,
//! everything under `/admin/reports` is directory-scoped and tenant-guarded.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::{validate_pagination, ApiResult, AppError};
use crate::handlers::tenant_scope::{
    assert_directory_admin, caller_tenant, claims_from_headers, is_platform_operator,
};
use crate::AppState;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref REPORT_LIMITER: Mutex<HashMap<String, Vec<Instant>>> = Mutex::new(HashMap::new());
}

const REPORT_MAX_PER_HOUR_PER_KEY: usize = 5;
const REPORT_MAX_PER_HOUR_GLOBAL: usize = 120;

/// Sliding-window guard for `POST /businesses/:id/report` (an anonymous write). Keyed by the
/// caller's address, with a global hourly ceiling, so a stuck client cannot flood the queue.
fn check_report_rate(key: &str) -> Result<(), AppError> {
    let now = Instant::now();
    let window = Duration::from_secs(3600);
    let mut map = REPORT_LIMITER
        .lock()
        .map_err(|_| AppError::Internal("report rate limiter unavailable".into()))?;

    for v in map.values_mut() {
        v.retain(|t| now.duration_since(*t) < window);
    }

    let global = map.get("__global__").map(|v| v.len()).unwrap_or(0);
    let per_key = map.get(key).map(|v| v.len()).unwrap_or(0);
    if global >= REPORT_MAX_PER_HOUR_GLOBAL || per_key >= REPORT_MAX_PER_HOUR_PER_KEY {
        return Err(AppError::TooManyRequests(
            "too many reports from this address — try again later".into(),
        ));
    }

    map.entry(key.to_string()).or_default().push(now);
    map.entry("__global__".to_string()).or_default().push(now);
    Ok(())
}

const REPORT_REASONS: [&str; 6] = [
    "closed",
    "wrong_info",
    "spam",
    "offensive",
    "duplicate",
    "other",
];

const REPORT_STATUSES: [&str; 4] = ["pending", "reviewed", "resolved", "dismissed"];

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct BusinessReport {
    pub id: Uuid,
    pub business_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub reason: String,
    pub details: Option<String>,
    pub reporter_name: Option<String>,
    pub reporter_email: Option<String>,
    pub status: String,
    pub resolution_note: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportRequest {
    pub reason: String,
    pub details: Option<String>,
    pub reporter_name: Option<String>,
    pub reporter_email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReportRequest {
    pub status: String,
    pub resolution_note: Option<String>,
}

fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// POST /api/v1/businesses/:id/report — public, rate-limited. Returns 404 for an unknown
/// business (so a probe learns nothing) and 422 for a reason outside the closed vocabulary.
pub async fn create_report(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CreateReportRequest>,
) -> ApiResult<impl IntoResponse> {
    let reason = req.reason.trim().to_lowercase();
    if !REPORT_REASONS.contains(&reason.as_str()) {
        return Err(AppError::Validation(
            "That report reason is not recognised.".into(),
        ));
    }

    let details: Option<String> = req
        .details
        .map(|d| d.trim().chars().take(2000).collect::<String>())
        .filter(|d| !d.is_empty());
    let reporter_name: Option<String> = req
        .reporter_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let reporter_email: Option<String> = req
        .reporter_email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());

    // Resolve the business (and its directory) before anything is written.
    let row = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT id, directory_id FROM businesses WHERE id = $1",
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?;
    let (biz_id, directory_id) =
        row.ok_or_else(|| AppError::NotFound("Business not found".into()))?;

    check_report_rate(&client_key(&headers))?;

    let report = sqlx::query_as::<_, BusinessReport>(
        r#"INSERT INTO business_reports
               (business_id, directory_id, reason, details, reporter_name, reporter_email, status)
           VALUES ($1, $2, $3, $4, $5, $6, 'pending')
           RETURNING *"#,
    )
    .bind(biz_id)
    .bind(directory_id)
    .bind(&reason)
    .bind(details)
    .bind(reporter_name)
    .bind(reporter_email)
    .fetch_one(&s.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Thanks — a moderator will review this listing.",
            "id": report.id,
        })),
    ))
}

/// GET /api/v1/admin/reports?status=&directory_id=&page=&per_page= — the moderation queue.
/// A directory owner sees only their own directories' reports; the platform operator sees all.
pub async fn list_reports(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;

    let requested_dir = params
        .get("directory_id")
        .and_then(|v| Uuid::parse_str(v).ok());
    // If a specific directory is requested, the caller must administer it (404 otherwise). The
    // tenant clause below then keeps a caller who omitted it inside their own directories.
    if let Some(d) = requested_dir {
        assert_directory_admin(&s.db, &claims, d).await?;
    }
    let is_op = is_platform_operator(&claims);
    let tenant = if is_op {
        Uuid::nil()
    } else {
        caller_tenant(&claims)?
    };
    let status_filter = params.get("status").cloned().filter(|v| !v.is_empty());

    let page = params
        .get("page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    // String concatenation at COMPILE time (concat! strips the spread of report scope), so the
    // statement is still a literal to the 5d gate — no run-time format!.
    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT COUNT(*) FROM business_reports r WHERE ",
        "($1::text IS NULL OR r.status = $1) ",
        "AND ($2::uuid IS NULL OR r.directory_id = $2) ",
        "AND ($3::bool OR r.directory_id IN (SELECT d.id FROM directories d ",
        "JOIN users u ON u.id = d.owner_id WHERE u.tenant_id = $4))"
    ))
    .bind(status_filter.as_deref())
    .bind(requested_dir)
    .bind(is_op)
    .bind(tenant)
    .fetch_one(&s.db)
    .await?;

    let rows = sqlx::query_as::<_, BusinessReport>(concat!(
        "SELECT r.* FROM business_reports r WHERE ",
        "($1::text IS NULL OR r.status = $1) ",
        "AND ($2::uuid IS NULL OR r.directory_id = $2) ",
        "AND ($3::bool OR r.directory_id IN (SELECT d.id FROM directories d ",
        "JOIN users u ON u.id = d.owner_id WHERE u.tenant_id = $4)) ",
        "ORDER BY r.created_at DESC LIMIT $5 OFFSET $6"
    ))
    .bind(status_filter.as_deref())
    .bind(requested_dir)
    .bind(is_op)
    .bind(tenant)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!({
        "data": rows,
        "page": page,
        "per_page": per_page,
        "total": total,
        "total_pages": total_pages,
    })))
}

/// GET /api/v1/admin/reports/stats?directory_id= — pending / resolved counts for the panel badge.
pub async fn report_stats(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;
    let requested_dir = params
        .get("directory_id")
        .and_then(|v| Uuid::parse_str(v).ok());
    if let Some(d) = requested_dir {
        assert_directory_admin(&s.db, &claims, d).await?;
    }
    let is_op = is_platform_operator(&claims);
    let tenant = if is_op {
        Uuid::nil()
    } else {
        caller_tenant(&claims)?
    };

    let row = sqlx::query(concat!(
        "SELECT ",
        "COUNT(*) FILTER (WHERE r.status = 'pending')   AS pending, ",
        "COUNT(*) FILTER (WHERE r.status = 'reviewed')  AS reviewed, ",
        "COUNT(*) FILTER (WHERE r.status = 'resolved')  AS resolved, ",
        "COUNT(*) FILTER (WHERE r.status = 'dismissed') AS dismissed, ",
        "COUNT(*)                                       AS total ",
        "FROM business_reports r WHERE ",
        "($1::uuid IS NULL OR r.directory_id = $1) ",
        "AND ($2::bool OR r.directory_id IN (SELECT d.id FROM directories d ",
        "JOIN users u ON u.id = d.owner_id WHERE u.tenant_id = $3))"
    ))
    .bind(requested_dir)
    .bind(is_op)
    .bind(tenant)
    .fetch_one(&s.db)
    .await?;

    use sqlx::Row;
    Ok(Json(json!({
        "pending": row.try_get::<i64, _>("pending").unwrap_or(0),
        "reviewed": row.try_get::<i64, _>("reviewed").unwrap_or(0),
        "resolved": row.try_get::<i64, _>("resolved").unwrap_or(0),
        "dismissed": row.try_get::<i64, _>("dismissed").unwrap_or(0),
        "total": row.try_get::<i64, _>("total").unwrap_or(0),
    })))
}

/// PATCH /api/v1/admin/reports/:id — set status (and an optional note) from the panel.
pub async fn update_report(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReportRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;
    let status = req.status.trim().to_lowercase();
    if !REPORT_STATUSES.contains(&status.as_str()) {
        return Err(AppError::Validation(
            "That report status is not recognised.".into(),
        ));
    }

    let dir: Option<Uuid> =
        sqlx::query_scalar("SELECT directory_id FROM business_reports WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Report not found".into()))?;

    match dir {
        Some(d) => assert_directory_admin(&s.db, &claims, d).await?,
        None => {
            if !is_platform_operator(&claims) {
                return Err(AppError::NotFound("Report not found".into()));
            }
        }
    }

    let note: Option<String> = req
        .resolution_note
        .map(|n| n.trim().chars().take(2000).collect::<String>())
        .filter(|n| !n.is_empty());
    let resolved_at = if status == "pending" {
        None
    } else {
        Some(Utc::now())
    };

    let report = sqlx::query_as::<_, BusinessReport>(
        r#"UPDATE business_reports
              SET status = $1, resolution_note = COALESCE($2, resolution_note), resolved_at = $3
            WHERE id = $4
        RETURNING *"#,
    )
    .bind(&status)
    .bind(note)
    .bind(resolved_at)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(report)))
}
