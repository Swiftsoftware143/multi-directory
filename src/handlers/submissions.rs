//! Submission CRUD handlers for Multi-Directory API.
//! Public form submissions that admins can approve (→ create business) or reject.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

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
) -> ApiResult<impl IntoResponse> {
    let submissions = sqlx::query_as::<_, Submission>(
        "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

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
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let submission = sqlx::query_as::<_, Submission>(
        "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Submission not found".into()))?;

    Ok(Json(submission))
}

/// PUT /api/v1/submissions/:id — update submission (for admin review)
pub async fn update_submission(
    State(s): State<AppState>,
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
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
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
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let submission = sqlx::query_as::<_, Submission>(
        "SELECT id, business_name, category, address, city, state, zip, phone, email, website, description, submitted_by, submitter_email, directory_id, status, admin_notes, created_at, updated_at FROM submissions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Submission not found".into()))?;

    if submission.status.as_deref() == Some("approved") {
        return Err(AppError::BadRequest("Submission is already approved".into()));
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
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let admin_notes = body.get("admin_notes").and_then(|v| v.as_str()).map(|s| s.to_string());

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
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
