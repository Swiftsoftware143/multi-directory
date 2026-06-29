//! Legal pages CRUD handlers for Multi-Directory API.

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
pub struct LegalPage {
    pub id: Uuid,
    pub title: String,
    pub page_type: String,
    pub content: String,
    pub published: Option<bool>,
    pub is_global: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLegalPageRequest {
    pub title: String,
    pub page_type: Option<String>,
    pub content: String,
    pub published: Option<bool>,
    pub is_global: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLegalPageRequest {
    pub title: Option<String>,
    pub page_type: Option<String>,
    pub content: Option<String>,
    pub published: Option<bool>,
    pub is_global: Option<bool>,
}

/// GET /api/v1/legal-pages — list all legal pages
pub async fn list_legal_pages(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let pages = sqlx::query_as::<_, LegalPage>(
        "SELECT id, title, page_type, content, published, is_global, created_at, updated_at FROM legal_pages ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(pages))
}

/// POST /api/v1/legal-pages — create a legal page
pub async fn create_legal_page(
    State(s): State<AppState>,
    Json(req): Json<CreateLegalPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let page = sqlx::query_as::<_, LegalPage>(
        "INSERT INTO legal_pages (title, page_type, content, published, is_global) VALUES (\x241, \x242, \x243, \x244, \x245) RETURNING id, title, page_type, content, published, is_global, created_at, updated_at "
    )
    .bind(&req.title)
    .bind(req.page_type.as_deref().unwrap_or("custom"))
    .bind(&req.content)
    .bind(req.published.unwrap_or(true))
    .bind(req.is_global.unwrap_or(false))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(page)))
}

/// GET /api/v1/legal-pages/:id — get single legal page
pub async fn get_legal_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let page = sqlx::query_as::<_, LegalPage>(
        "SELECT id, title, page_type, content, published, is_global, created_at, updated_at FROM legal_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Legal page not found".into()))?;

    Ok(Json(page))
}

/// PUT /api/v1/legal-pages/:id — update legal page
pub async fn update_legal_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLegalPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, LegalPage>(
        "SELECT id, title, page_type, content, published, is_global, created_at, updated_at FROM legal_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Legal page not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let page_type = req.page_type.unwrap_or(existing.page_type);
    let content = req.content.unwrap_or(existing.content);
    let published = req.published.unwrap_or(existing.published.unwrap_or(true));
    let is_global = req.is_global.unwrap_or(existing.is_global.unwrap_or(false));

    let page = sqlx::query_as::<_, LegalPage>(
        "UPDATE legal_pages SET title = \x241, page_type = \x242, content = \x243, published = \x244, is_global = \x245, updated_at = NOW() WHERE id = \x246 RETURNING id, title, page_type, content, published, is_global, created_at, updated_at "
    )
    .bind(&title)
    .bind(&page_type)
    .bind(&content)
    .bind(published)
    .bind(is_global)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(page))
}

/// DELETE /api/v1/legal-pages/:id — delete legal page
pub async fn delete_legal_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM legal_pages WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Legal page not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}
