//! Industry taxonomy handlers — `template_categories`.
//!
//! An "industry" is the vertical a directory can be built on. The list is a real table in this
//! app's own database (migration 108) and is managed **entirely from the admin panel's
//! Industries section** — add, rename, re-describe, re-icon, reorder, publish/unpublish — with
//! no SQL and no code change (David's sellable standard: a buyer operates everything from the
//! panel).
//!
//! Two read surfaces:
//!   * `GET /api/v1/industries/available` — the *published* list (is_active = true), the contract
//!     consumers/directories read. Authenticated by the app's global auth guard.
//!   * `GET /api/v1/industries/catalogue`  — every row incl. unpublished, for the panel itself.
//! Writes (`POST`/`PUT /industries/catalogue…`) are operator-guarded because these are the
//! platform's own catalogue rows.
//!
//! The former per-user `user_industry_dashboards` scaffolding (a table with 0 rows, a column
//! `tenants.industry_slug` nothing read, and four routes with no caller) was retired in the same
//! change; see migration 113.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

/// A published industry — the shape the consumer-facing list returns.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IndustryOption {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
}

/// A full catalogue row — what the admin panel edits.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IndustryCatalogueRow {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIndustryRequest {
    /// Optional: derived from `name` when omitted.
    pub slug: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    /// Optional: appended to the end of the list when omitted.
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIndustryRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

/// Lowercase, ASCII alphanumerics, single dashes, trimmed. `"HVAC & Plumbing!"` -> `"hvac-plumbing"`.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true; // suppress a leading dash
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// GET /api/v1/industries/available
/// The published industries a directory can be built on (is_active = true, ordered). Source of
/// truth is this app's own `template_categories` table (migration 108) — a real, admin-editable
/// table, edited from the panel's Industries section. Errors propagate; there is no hardcoded
/// fallback.
pub async fn list_available_industries(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let industries = sqlx::query_as::<_, IndustryOption>(
        "SELECT slug, name, description, icon, sort_order::int FROM template_categories \
         WHERE is_active = true ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(industries)))
}

/// GET /api/v1/industries/catalogue  (operator)
/// Every catalogue row, published or not — the admin panel's management view.
pub async fn list_catalogue(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query_as::<_, IndustryCatalogueRow>(
        "SELECT id, slug, name, description, icon, sort_order, is_active, created_at, updated_at \
         FROM template_categories ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(rows)))
}

/// POST /api/v1/industries/catalogue  (operator)
/// Adds an industry. The slug is derived from the name unless one is supplied.
pub async fn create_industry(
    State(s): State<AppState>,
    Json(req): Json<CreateIndustryRequest>,
) -> ApiResult<impl IntoResponse> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("name is required".to_string()));
    }

    let slug = match req.slug.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(explicit) => slugify(explicit),
        None => slugify(&name),
    };
    if slug.is_empty() {
        return Err(AppError::Validation(
            "name must contain at least one letter or digit".to_string(),
        ));
    }

    // Friendly duplicate error instead of a raw unique-violation 500.
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT slug FROM template_categories WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&s.db)
            .await?;
    if exists.is_some() {
        return Err(AppError::Duplicate(format!(
            "An industry with slug '{slug}' already exists"
        )));
    }

    let icon = match req.icon.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => "📁".to_string(),
    };
    let description = req
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string);

    let row = sqlx::query_as::<_, IndustryCatalogueRow>(
        "INSERT INTO template_categories (slug, name, description, icon, sort_order) \
         VALUES ($1, $2, $3, $4, \
                 COALESCE($5, (SELECT COALESCE(MAX(sort_order) + 1, 0) FROM template_categories))) \
         RETURNING id, slug, name, description, icon, sort_order, is_active, created_at, updated_at",
    )
    .bind(&slug)
    .bind(&name)
    .bind(description)
    .bind(icon)
    .bind(req.sort_order)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(row))))
}

/// PUT /api/v1/industries/catalogue/:slug  (operator)
/// Edits any field; a field left out is unchanged. `is_active` publishes/unpublishes — the
/// panel's "published" checkbox is the single control for that, so there is deliberately no
/// separate DELETE route (an endpoint with no caller would be its own defect).
pub async fn update_industry(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<UpdateIndustryRequest>,
) -> ApiResult<impl IntoResponse> {
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let icon = req
        .icon
        .as_deref()
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .map(str::to_string);
    let description = req
        .description
        .as_deref()
        .map(str::trim)
        .map(str::to_string);

    let row = sqlx::query_as::<_, IndustryCatalogueRow>(
        "UPDATE template_categories SET \
            name = COALESCE($2, name), \
            description = COALESCE($3, description), \
            icon = COALESCE($4, icon), \
            sort_order = COALESCE($5, sort_order), \
            is_active = COALESCE($6, is_active), \
            updated_at = NOW() \
         WHERE slug = $1 \
         RETURNING id, slug, name, description, icon, sort_order, is_active, created_at, updated_at",
    )
    .bind(&slug)
    .bind(name)
    .bind(description)
    .bind(icon)
    .bind(req.sort_order)
    .bind(req.is_active)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("No industry with slug '{slug}'")))?;

    Ok(Json(json!(row)))
}
