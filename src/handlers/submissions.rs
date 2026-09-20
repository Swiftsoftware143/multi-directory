//! Submission CRUD handlers for Multi-Directory API.
//! Public form submissions that admins can approve (→ create business) or reject.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::tenant_scope::{assert_submission_admin, caller_tenant, is_platform_operator};
use crate::AppState;

// ── Rate limiter for the anonymous public submit-a-business form ─────────────
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref SUBMIT_LIMITER: Mutex<HashMap<String, Vec<Instant>>> = Mutex::new(HashMap::new());
}

const SUBMIT_MAX_PER_HOUR_PER_KEY: usize = 3;
const SUBMIT_MAX_PER_HOUR_GLOBAL: usize = 60;

/// Sliding-window guard for `POST /submissions` (the only anonymous write in
/// the visitor flow). Keyed by submitter email, with a global hourly ceiling.
fn check_submission_rate(key: &str) -> Result<(), AppError> {
    let now = Instant::now();
    let window = Duration::from_secs(3600);
    let mut map = SUBMIT_LIMITER
        .lock()
        .map_err(|_| AppError::Internal("submission rate limiter unavailable".into()))?;

    for v in map.values_mut() {
        v.retain(|t| now.duration_since(*t) < window);
    }

    let global = map.get("__global__").map(|v| v.len()).unwrap_or(0);
    let per_key = map.get(key).map(|v| v.len()).unwrap_or(0);
    if global >= SUBMIT_MAX_PER_HOUR_GLOBAL || per_key >= SUBMIT_MAX_PER_HOUR_PER_KEY {
        return Err(AppError::TooManyRequests(
            "submission rate limit exceeded — try again later".into(),
        ));
    }

    map.entry(key.to_string()).or_default().push(now);
    map.entry("__global__".to_string()).or_default().push(now);
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Submission {
    pub id: Uuid,
    pub business_name: String,
    pub category: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
    pub submitted_by: Option<String>,
    pub submitter_email: Option<String>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub admin_notes: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSubmissionRequest {
    pub business_name: String,
    pub category: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
    pub submitted_by: Option<String>,
    pub submitter_email: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSubmissionRequest {
    pub business_name: Option<String>,
    pub category: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
    pub submitted_by: Option<String>,
    pub submitter_email: Option<String>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub admin_notes: Option<String>,
}

/// GET /api/v1/submissions — list all submissions (admin view)
pub async fn list_submissions(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    // Round 13 IDOR audit: this used to return every submission on the platform
    // (submitter names, emails, phone numbers) to any authenticated tenant.
    // The platform operator sees the whole review queue; a tenant sees only the
    // submissions addressed to a directory their tenant owns.
    let submissions = if is_platform_operator(&claims) {
        sqlx::query_as::<_, Submission>(
            "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions ORDER BY created_at DESC "
        )
        .fetch_all(&s.db)
        .await?
    } else {
        let tid = caller_tenant(&claims)?;
        sqlx::query_as::<_, Submission>(
            "SELECT s.id, s.business_name, s.category, s.address, s.city, s.state, s.zip, s.phone, s.email, s.website, s.description, s.submitted_by, s.submitter_email, s.directory_id, s.status, s.admin_notes, s.created_at, s.updated_at \
             FROM submissions s \
             JOIN directories d ON d.id = s.directory_id \
             JOIN users u ON u.id = d.owner_id \
             WHERE u.tenant_id = $1 ORDER BY s.created_at DESC "
        )
        .bind(tid)
        .fetch_all(&s.db)
        .await?
    };

    Ok(Json(submissions))
}

/// POST /api/v1/submissions — public form, create submission (no auth required)
pub async fn create_submission(
    State(s): State<AppState>,
    Json(req): Json<CreateSubmissionRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.business_name.trim().is_empty() {
        return Err(AppError::Validation("business_name is required".into()));
    }

    // Anonymous write path — throttle per submitter email + global ceiling.
    let rate_key = req
        .submitter_email
        .as_deref()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "anon".to_string());
    check_submission_rate(&rate_key)?;

    let submission = sqlx::query_as::<_, Submission>(
        "INSERT INTO submissions (business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413) RETURNING id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at "
    )
    .bind(&req.business_name)
    .bind(&req.category)
    .bind(&req.address)
    .bind(&req.city)
    .bind(&req.state)
    .bind(&req.zip)
    .bind(&req.phone)
    .bind(&req.email)
    .bind(&req.website)
    .bind(&req.description)
    .bind(&req.submitted_by)
    .bind(&req.submitter_email)
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(submission)))
}

/// GET /api/v1/submissions/:id — get single submission
pub async fn get_submission(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let submission = sqlx::query_as::<_, Submission>(
        "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Submission not found".into()))?;
    // Round 13 IDOR audit: submitter PII is visible only to the directory's tenant.
    assert_submission_admin(&s.db, &claims, id).await?;

    Ok(Json(submission))
}

/// PUT /api/v1/submissions/:id — update submission (for admin review)
pub async fn update_submission(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSubmissionRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Submission>(
        "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Submission not found".into()))?;
    // Round 13 IDOR audit: cross-tenant write — only the review queue's owner may edit.
    assert_submission_admin(&s.db, &claims, id).await?;

    let business_name = req.business_name.unwrap_or(existing.business_name);
    let category = req.category.or(existing.category);
    let address = req.address.or(existing.address);
    let city = req.city.or(existing.city);
    let state = req.state.or(existing.state);
    let zip = req.zip.or(existing.zip);
    let phone = req.phone.or(existing.phone);
    let email = req.email.or(existing.email);
    let website = req.website.or(existing.website);
    let description = req.description.or(existing.description);
    let submitted_by = req.submitted_by.or(existing.submitted_by);
    let submitter_email = req.submitter_email.or(existing.submitter_email);
    let directory_id = req.directory_id.or(existing.directory_id);
    let status = req.status.or(existing.status);
    let admin_notes = req.admin_notes.or(existing.admin_notes);

    let submission = sqlx::query_as::<_, Submission>(
        "UPDATE submissions SET business_name = \x241, category = \x242, address = \x243, city = \x244, state = \x245, zip = \x246, phone = \x247, email = \x248, website = \x249, description = \x2410, submitted_by = \x2411, submitter_email = \x2412, directory_id = \x2413, status = \x2414, admin_notes = \x2415, updated_at = NOW() WHERE id = \x2416 RETURNING id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at "
    )
    .bind(&business_name)
    .bind(&category)
    .bind(&address)
    .bind(&city)
    .bind(&state)
    .bind(&zip)
    .bind(&phone)
    .bind(&email)
    .bind(&website)
    .bind(&description)
    .bind(&submitted_by)
    .bind(&submitter_email)
    .bind(directory_id)
    .bind(&status)
    .bind(&admin_notes)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(submission))
}

/// DELETE /api/v1/submissions/:id — delete submission
pub async fn delete_submission(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    // Round 13 IDOR audit: cross-tenant delete.
    assert_submission_admin(&s.db, &claims, id).await?;
    let result = sqlx::query("DELETE FROM submissions WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Submission not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/submissions/:id/approve — approve → auto-create business
pub async fn approve_submission(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let submission = sqlx::query_as::<_, Submission>(
        "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Submission not found".into()))?;
    // Round 13 IDOR audit: approving injects a business into the target directory.
    assert_submission_admin(&s.db, &claims, id).await?;

    if submission.status.as_deref() == Some("approved") {
        return Err(AppError::BadRequest(
            "Submission is already approved".into(),
        ));
    }

    // Generate a slug from the business name
    let slug = slugify(&submission.business_name);

    // Create the business record
    // We need to insert into businesses (name, slug, directory_id, etc.)
    let business = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO businesses (name, slug, description, address, city, state, zip, phone, email, website, directory_id) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411) RETURNING id "
    )
    .bind(&submission.business_name)
    .bind(&slug)
    .bind(&submission.description)
    .bind(&submission.address)
    .bind(&submission.city)
    .bind(&submission.state)
    .bind(&submission.zip)
    .bind(&submission.phone)
    .bind(&submission.email)
    .bind(&submission.website)
    .bind(submission.directory_id)
    .fetch_one(&s.db)
    .await?;

    // Update submission status to approved
    let updated = sqlx::query_as::<_, Submission>(
        "UPDATE submissions SET status = 'approved', updated_at = NOW() WHERE id = \x241 RETURNING id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at "
    )
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(serde_json::json!({
        "submission": updated,
        "business_id": business.0,
        "message": "Submission approved and business created "
    })))
}

/// POST /api/v1/submissions/:id/reject — reject with optional notes
pub async fn reject_submission(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    // Round 13 IDOR audit: reject mutates another tenant's submission if left unscoped —
    // proven live (a tenant admin with no directory flipped a stranger's row to 'rejected').
    assert_submission_admin(&s.db, &claims, id).await?;
    let admin_notes = body
        .get("admin_notes")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let submission = sqlx::query_as::<_, Submission>(
        "UPDATE submissions SET status = 'rejected', admin_notes = COALESCE(\x241, admin_notes), updated_at = NOW() WHERE id = \x242 RETURNING id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at "
    )
    .bind(&admin_notes)
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Submission not found".into()))?;

    Ok(Json(submission))
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
