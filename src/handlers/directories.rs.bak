//! Directory and category CRUD handlers.
//!
//! Updated with template engine support:
//! - Create/Update accept template + color_scheme
//! - GET /api/v1/directories/:slug/render returns HTML rendered with template
//! - GET /api/v1/directories/:slug/preview returns template preview

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult, validate_pagination};
use crate::models::*;
use crate::template_engine;

/// GET /api/v1/directories
pub async fn list_directories(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
    let per_page = params.get("per_page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories")
        .fetch_one(&s.db)
        .await?;

    let directories = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 "
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!(PaginatedResponse {
        data: directories,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// GET /api/v1/directories/:slug
pub async fn get_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    Ok(Json(json!(directory)))
}

/// POST /api/v1/directories
pub async fn create_directory(
    State(s): State<AppState>,
    Json(req): Json<CreateDirectoryRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() || req.slug.is_empty() {
        return Err(AppError::Validation("Name and slug are required".to_string()));
    }

    // Check slug uniqueness
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE slug = \x241 "
    )
    .bind(&req.slug)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!("Directory slug '{}' already exists", req.slug)));
    }

    let template = req.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let template = if template_engine::is_valid_template(template) { template } else { template_engine::TEMPLATE_LOCAL_BUSINESS };

    let color_scheme = req.color_scheme
        .clone()
        .unwrap_or_else(template_engine::default_color_scheme);

    let directory = sqlx::query_as::<_, Directory>(
        r#"INSERT INTO directories (name, slug, description, status, template, color_scheme)
           VALUES (\x241, \x242, \x243, \x244, \x245, \x246::jsonb)
           RETURNING *"#
    )
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(&req.status.unwrap_or_else(|| "draft".to_string()))
    .bind(template)
    .bind(&color_scheme.to_string())
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(directory))))
}

/// PUT /api/v1/directories/:slug
pub async fn update_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<UpdateDirectoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let new_name = req.name.unwrap_or(existing.name);
    let new_slug = req.slug.unwrap_or(existing.slug.clone());
    let new_description = req.description.or(existing.description);
    let new_status = req.status.or(existing.status);
    let new_template = req.template.unwrap_or(existing.template.unwrap_or_else(|| template_engine::TEMPLATE_LOCAL_BUSINESS.to_string()));
    let new_color_scheme = req.color_scheme.or(existing.color_scheme);

    if new_slug != slug {
        let slug_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM directories WHERE slug = \x241 AND id != \x242 "
        )
        .bind(&new_slug)
        .bind(existing.id)
        .fetch_one(&s.db)
        .await?;

        if slug_exists > 0 {
            return Err(AppError::Duplicate(format!("Slug '{}' already in use", new_slug)));
        }
    }

    let directory = sqlx::query_as::<_, Directory>(
        "UPDATE directories SET name = \x241, slug = \x242, description = \x243, status = \x244, template = \x245, color_scheme = \x246::jsonb, updated_at = NOW() WHERE id = \x247 RETURNING *"
    )
    .bind(&new_name)
    .bind(&new_slug)
    .bind(&new_description)
    .bind(&new_status)
    .bind(&new_template)
    .bind(&new_color_scheme.map(|v| v.to_string()).unwrap_or_default())
    .bind(existing.id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(directory)))
}

/// DELETE /api/v1/directories/:slug
pub async fn delete_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM directories WHERE slug = \x241")
        .bind(&slug)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Directory '{}' not found", slug)));
    }

    Ok((StatusCode::OK, Json(json!({"message": "Directory deleted successfully"}))))
}

/// GET /api/v1/directories/:slug/render — render directory page with template
pub async fn render_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let categories = sqlx::query_as::<_, DirectoryCategory>(
        "SELECT * FROM directory_categories WHERE directory_id = \x241 ORDER BY sort_order ASC, name ASC "
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    let businesses = sqlx::query_as::<_, Business>(
        "SELECT * FROM businesses WHERE directory_id = \x241 AND is_active = true ORDER BY rating DESC NULLS LAST, name ASC "
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    // Load business meta
    let mut meta_map = HashMap::new();
    for biz in &businesses {
        if let Ok(meta) = sqlx::query_as::<_, BusinessMeta>(
            "SELECT * FROM business_meta WHERE business_id = \x241 AND template = \x242 "
        )
        .bind(biz.id)
        .bind(directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS))
        .fetch_optional(&s.db)
        .await
        {
            if let Some(m) = meta {
                meta_map.insert(biz.id, m.meta_data);
            }
        }
    }

    let template_id = directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let cats_val = serde_json::to_value(&categories).unwrap_or_default();
    let biz_val = serde_json::to_value(&businesses).unwrap_or_default();
    let ctx = template_engine::build_template_context(
        &dir_val,
        &biz_val,
        &cats_val,
        None,
        None,
    );
    let html = engine.render_directory_page(template_id, &ctx)
        .map_err(|e| AppError::Internal(e))?;

    Ok(axum::response::Html(html))
}

/// GET /api/v1/templates — list available templates
pub async fn list_templates() -> ApiResult<impl IntoResponse> {
    let templates = template_engine::get_available_templates();
    Ok(Json(json!(templates)))
}

// ── Categories ───────────────────────────────────────────────────────────────

/// GET /api/v1/directories/:slug/categories
pub async fn list_categories(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let categories = sqlx::query_as::<_, DirectoryCategory>(
        "SELECT * FROM directory_categories WHERE directory_id = \x241 ORDER BY sort_order ASC, name ASC "
    )
    .bind(dir.id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(categories)))
}

/// POST /api/v1/directories/:slug/categories
pub async fn create_category(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() || req.slug.is_empty() {
        return Err(AppError::Validation("Name and slug are required".to_string()));
    }

    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let category = sqlx::query_as::<_, DirectoryCategory>(
        r#"INSERT INTO directory_categories (directory_id, name, slug, sort_order)
           VALUES (\x241, \x242, \x243, \x244)
           RETURNING *"#
    )
    .bind(dir.id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(req.sort_order.unwrap_or(0))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(category))))
}

/// PUT /api/v1/directories/:slug/categories/:category_id
pub async fn update_category(
    State(s): State<AppState>,
    Path((slug, category_id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let existing = sqlx::query_as::<_, DirectoryCategory>(
        "SELECT * FROM directory_categories WHERE id = \x241 AND directory_id = \x242 "
    )
    .bind(category_id)
    .bind(dir.id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Category not found".to_string()))?;

    let new_name = req.name.unwrap_or(existing.name);
    let new_slug = req.slug.unwrap_or(existing.slug);
    let new_sort_order = req.sort_order.unwrap_or(existing.sort_order.unwrap_or(0));

    let category = sqlx::query_as::<_, DirectoryCategory>(
        "UPDATE directory_categories SET name = \x241, slug = \x242, sort_order = \x243
           WHERE id = \x244 RETURNING *"
    )
    .bind(&new_name)
    .bind(&new_slug)
    .bind(new_sort_order)
    .bind(category_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(category)))
}

/// DELETE /api/v1/directories/:slug/categories/:category_id
pub async fn delete_category(
    State(s): State<AppState>,
    Path((_slug, category_id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let cur = sqlx::query("DELETE FROM directory_categories WHERE id = \x241")
        .bind(category_id)
        .execute(&s.db)
        .await?;

    if cur.rows_affected() == 0 {
        return Err(AppError::NotFound("Category not found".to_string()));
    }

    Ok(Json(json!({"message": "Category deleted successfully"})))
}
