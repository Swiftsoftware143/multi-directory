//! Phase 2: Multi-category system — filter options, category requests,
//! bulk assign, and business self-service category management.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult, validate_pagination};

// ── Filter Options ───────────────────────────────────────────────────────────

/// A single category entry for filter options.
#[derive(Debug, Serialize)]
pub struct CategoryOption {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub group_name: Option<String>,
}

/// Filter options grouped by parent category group.
#[derive(Debug, Serialize)]
pub struct FilterOptionsResponse {
    pub groups: Vec<CategoryGroup>,
}

#[derive(Debug, Serialize)]
pub struct CategoryGroup {
    pub group_name: String,
    pub categories: Vec<CategoryOption>,
}

/// GET /api/v1/categories/filter-options
///
/// Returns all categories from directory_categories grouped by group_name
/// for use in dropdown filter menus on the search page.
pub async fn get_filter_options(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        r#"SELECT id, name, slug, group_name
           FROM directory_categories
           ORDER BY COALESCE(group_name, 'zzz') ASC, name ASC"#
    )
    .fetch_all(&s.db)
    .await?;

    let mut groups: Vec<CategoryGroup> = Vec::new();
    let mut current_group: Option<CategoryGroup> = None;

    for (id, name, slug, group_name) in rows {
        let gn = group_name.unwrap_or_else(|| "Other".to_string());
        match current_group {
            Some(ref mut cg) if cg.group_name == gn => {
                cg.categories.push(CategoryOption {
                    id,
                    name,
                    slug,
                    group_name: Some(gn.clone()),
                });
            }
            _ => {
                if let Some(cg) = current_group.take() {
                    groups.push(cg);
                }
                current_group = Some(CategoryGroup {
                    group_name: gn.clone(),
                    categories: vec![CategoryOption {
                        id,
                        name,
                        slug,
                        group_name: Some(gn),
                    }],
                });
            }
        }
    }
    if let Some(cg) = current_group {
        groups.push(cg);
    }

    Ok(Json(FilterOptionsResponse { groups }))
}

// ── Get Business Categories ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BusinessCategoryEntry {
    pub id: Uuid,
    pub name: String,
    pub group_name: Option<String>,
    pub is_primary: bool,
}

/// GET /api/v1/businesses/:id/categories — returns all categories for a business
/// with group_name info.
pub async fn get_business_categories(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let cats = sqlx::query_as::<_, (Uuid, String, Option<String>, bool)>(
        r#"SELECT bc.category_id, dc.name, dc.group_name, bc.is_primary
           FROM business_categories bc
           LEFT JOIN directory_categories dc ON dc.id = bc.category_id
           WHERE bc.business_id = $1
           ORDER BY bc.is_primary DESC, dc.name ASC"#
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    let result: Vec<BusinessCategoryEntry> = cats
        .into_iter()
        .map(|(id, name, group_name, is_primary)| BusinessCategoryEntry {
            id,
            name,
            group_name,
            is_primary,
        })
        .collect();

    Ok(Json(result))
}

// ── Set/Replace Business Categories ──────────────────────────────────────────

const DEFAULT_MAX_CATEGORIES: usize = 3;

#[derive(Debug, Deserialize)]
pub struct SetCategoriesRequest {
    pub category_ids: Vec<Uuid>,
    pub primary_category_id: Option<Uuid>,
}

/// PUT /api/v1/businesses/:id/categories
///
/// Replaces all categories for a business with the given list.
/// Enforces a default cap of 3 categories (admin can override to 5
/// via max_categories field in business_meta or feature_config).
pub async fn set_business_categories(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetCategoriesRequest>,
) -> ApiResult<impl IntoResponse> {
    // Determine max categories allowed
    let max_cats = get_max_categories(&s.db, id).await?;

    if req.category_ids.len() > max_cats {
        return Err(AppError::BadRequest(format!(
            "Maximum {} categories allowed. Upgrade to add more.",
            max_cats
        )));
    }

    // Validate all category IDs exist
    let valid_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directory_categories WHERE id = ANY($1)"
    )
    .bind(&req.category_ids)
    .fetch_one(&s.db)
    .await?;

    if valid_count != req.category_ids.len() as i64 {
        return Err(AppError::BadRequest("One or more category IDs are invalid".to_string()));
    }

    // Transaction: delete existing, insert new
    let mut tx = s.db.begin().await?;

    sqlx::query("DELETE FROM business_categories WHERE business_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    for (i, cat_id) in req.category_ids.iter().enumerate() {
        let is_primary = Some(*cat_id) == req.primary_category_id
            || (req.primary_category_id.is_none() && i == 0);
        sqlx::query(
            "INSERT INTO business_categories (business_id, category_id, is_primary) VALUES ($1, $2, $3)"
        )
        .bind(id)
        .bind(cat_id)
        .bind(is_primary)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(Json(json!({"status": "updated", "category_count": req.category_ids.len()})))
}

// ── Delete Single Category ───────────────────────────────────────────────────

/// DELETE /api/v1/businesses/:id/categories/:category_id
///
/// Removes one category from a business. Enforces minimum of 1 category
/// if there are any assigned (business must have at least one category).
pub async fn delete_business_category(
    State(s): State<AppState>,
    Path((business_id, category_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let current_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM business_categories WHERE business_id = $1"
    )
    .bind(business_id)
    .fetch_one(&s.db)
    .await?;

    if current_count <= 1 {
        return Err(AppError::BadRequest(
            "Business must have at least one category assigned".to_string()
        ));
    }

    let deleted = sqlx::query(
        "DELETE FROM business_categories WHERE business_id = $1 AND category_id = $2"
    )
    .bind(business_id)
    .bind(category_id)
    .execute(&s.db)
    .await?;

    if deleted.rows_affected() == 0 {
        return Err(AppError::NotFound("Category assignment not found".to_string()));
    }

    Ok(Json(json!({"status": "removed"})))
}

// ── Bulk Assign Categories ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BulkCategoryRequest {
    pub business_ids: Vec<Uuid>,
    pub category_ids: Vec<Uuid>,
    /// "replace" clears existing categories; "append" adds to them
    pub mode: String,
}

/// POST /api/v1/businesses/bulk/categories
///
/// Bulk assign categories to multiple businesses.
/// Mode "replace" replaces all existing categories; "append" adds to them.
pub async fn bulk_assign_categories(
    State(s): State<AppState>,
    Json(req): Json<BulkCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.business_ids.is_empty() || req.category_ids.is_empty() {
        return Err(AppError::BadRequest(
            "business_ids and category_ids must not be empty".to_string()
        ));
    }

    let mode = req.mode.to_lowercase();
    if mode != "replace" && mode != "append" {
        return Err(AppError::BadRequest(
            "mode must be 'replace' or 'append'".to_string()
        ));
    }

    let mut tx = s.db.begin().await?;

    if mode == "replace" {
        sqlx::query("DELETE FROM business_categories WHERE business_id = ANY($1)")
            .bind(&req.business_ids)
            .execute(&mut *tx)
            .await?;
    }

    // Insert categories, skip duplicates for append mode
    for &bid in &req.business_ids {
        // Check max categories for this business
        let existing_count = if mode == "append" {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM business_categories WHERE business_id = $1"
            )
            .bind(bid)
            .fetch_one(&mut *tx)
            .await?
        } else {
            0
        };

        let max_cats = get_max_categories_in_tx(&mut *tx, bid).await?;
        let available = max_cats.saturating_sub(existing_count as usize);
        let to_insert = std::cmp::min(req.category_ids.len(), available);

        for (i, &cat_id) in req.category_ids.iter().take(to_insert).enumerate() {
            let is_primary = existing_count == 0 && i == 0;
            let result = sqlx::query(
                "INSERT INTO business_categories (business_id, category_id, is_primary) \
                 VALUES ($1, $2, $3) ON CONFLICT (business_id, category_id) DO NOTHING"
            )
            .bind(bid)
            .bind(cat_id)
            .bind(is_primary)
            .execute(&mut *tx)
            .await;

            if let Err(e) = result {
                tracing::warn!(
                    "Failed to insert category {} for business {}: {}",
                    cat_id, bid, e
                );
            }
        }
    }

    tx.commit().await?;

    Ok(Json(json!({
        "status": "completed",
        "mode": mode,
        "business_count": req.business_ids.len(),
        "category_count": req.category_ids.len()
    })))
}

// ── Category Requests (Business Owner Self-Service) ──────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateCategoryRequest {
    pub category_id: Uuid,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct CategoryRequest {
    pub id: Uuid,
    pub business_id: Uuid,
    pub category_id: Uuid,
    pub requested_by: Option<Uuid>,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// POST /api/v1/businesses/:id/category-requests
///
/// Business owner requests adding a new category. Creates a pending request
/// that admins must approve.
pub async fn create_category_request(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    Json(req): Json<CreateCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    // Check category exists
    let cat_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directory_categories WHERE id = $1"
    )
    .bind(req.category_id)
    .fetch_one(&s.db)
    .await?;

    if cat_exists == 0 {
        return Err(AppError::NotFound("Category not found".to_string()));
    }

    // Check not already assigned
    let already_assigned = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM business_categories WHERE business_id = $1 AND category_id = $2"
    )
    .bind(business_id)
    .bind(req.category_id)
    .fetch_one(&s.db)
    .await?;

    if already_assigned > 0 {
        return Err(AppError::Duplicate(
            "Business already has this category assigned".to_string()
        ));
    }

    // Check not already requested
    let already_requested = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM category_requests \
         WHERE business_id = $1 AND category_id = $2 AND status = 'pending'"
    )
    .bind(business_id)
    .bind(req.category_id)
    .fetch_one(&s.db)
    .await?;

    if already_requested > 0 {
        return Err(AppError::Duplicate(
            "A pending request for this category already exists".to_string()
        ));
    }

    let request = sqlx::query_as::<_, CategoryRequest>(
        "INSERT INTO category_requests (business_id, category_id, notes) \
         VALUES ($1, $2, $3) RETURNING *"
    )
    .bind(business_id)
    .bind(req.category_id)
    .bind(&req.notes)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(request))))
}

/// GET /api/v1/category-requests
///
/// Lists all pending (or filtered by status) category requests for admin review.
#[derive(Debug, Deserialize)]
pub struct ListCategoryRequestsQuery {
    pub status: Option<String>,
    pub business_id: Option<Uuid>,
}

pub async fn list_category_requests(
    State(s): State<AppState>,
    Query(qs): Query<ListCategoryRequestsQuery>,
) -> ApiResult<impl IntoResponse> {
    let status_filter = qs.status.unwrap_or_else(|| "pending".to_string());

    let requests: Vec<serde_json::Value> = if let Some(bid) = qs.business_id {
        sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<Uuid>, String, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT cr.id, cr.business_id, cr.category_id, cr.requested_by, cr.status, \
                    cr.notes, cr.created_at, cr.updated_at \
             FROM category_requests cr \
             WHERE cr.status = $1 AND cr.business_id = $2 \
             ORDER BY cr.created_at DESC"
        )
        .bind(&status_filter)
        .bind(bid)
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .map(|(id, business_id, category_id, requested_by, status, notes, created_at, updated_at)| {
            json!({
                "id": id,
                "business_id": business_id,
                "category_id": category_id,
                "requested_by": requested_by,
                "status": status,
                "notes": notes,
                "created_at": created_at,
                "updated_at": updated_at
            })
        })
        .collect()
    } else {
        sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<Uuid>, String, Option<String>, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT cr.id, cr.business_id, cr.category_id, cr.requested_by, cr.status, \
                    cr.notes, cr.created_at, cr.updated_at \
             FROM category_requests cr \
             WHERE cr.status = $1 \
             ORDER BY cr.created_at DESC"
        )
        .bind(&status_filter)
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .map(|(id, business_id, category_id, requested_by, status, notes, created_at, updated_at)| {
            json!({
                "id": id,
                "business_id": business_id,
                "category_id": category_id,
                "requested_by": requested_by,
                "status": status,
                "notes": notes,
                "created_at": created_at,
                "updated_at": updated_at
            })
        })
        .collect()
    };

    Ok(Json(requests))
}

/// POST /api/v1/category-requests/:id/approve
///
/// Admin approves a category request. Adds the category to the business
/// and marks the request as approved.
pub async fn approve_category_request(
    State(s): State<AppState>,
    Path(request_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let request = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT id, business_id, status FROM category_requests WHERE id = $1"
    )
    .bind(request_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Category request not found".to_string()))?;

    if request.2 != "pending" {
        return Err(AppError::BadRequest(
            format!("Request status is '{}', not 'pending'", request.2)
        ));
    }

    let mut tx = s.db.begin().await?;

    // Insert into business_categories
    sqlx::query(
        "INSERT INTO business_categories (business_id, category_id, is_primary) \
         VALUES ($1, (SELECT category_id FROM category_requests WHERE id = $2), false) \
         ON CONFLICT (business_id, category_id) DO NOTHING"
    )
    .bind(request.1)
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    // Update request status
    sqlx::query(
        "UPDATE category_requests SET status = 'approved', updated_at = now() WHERE id = $1"
    )
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(json!({"status": "approved"})))
}

/// POST /api/v1/category-requests/:id/deny
///
/// Admin denies a category request.
pub async fn deny_category_request(
    State(s): State<AppState>,
    Path(request_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let updated = sqlx::query(
        "UPDATE category_requests SET status = 'denied', updated_at = now() \
         WHERE id = $1 AND status = 'pending'"
    )
    .bind(request_id)
    .execute(&s.db)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "Category request not found or not pending".to_string()
        ));
    }

    Ok(Json(json!({"status": "denied"})))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Returns the maximum number of categories a business can have.
/// Default is 3; admins can set up to 5 via business_meta.
async fn get_max_categories(db: &sqlx::PgPool, business_id: Uuid) -> Result<usize, AppError> {
    get_max_categories_internal(db, business_id).await
}

/// Same helper but usable within a transaction.
async fn get_max_categories_in_tx(
    exec: impl sqlx::PgExecutor<'_>,
    business_id: Uuid,
) -> Result<usize, AppError> {
    get_max_categories_internal(exec, business_id).await
}

async fn get_max_categories_internal(
    exec: impl sqlx::PgExecutor<'_>,
    business_id: Uuid,
) -> Result<usize, AppError> {
    // Check business_meta for admin override
    let override_count: Option<i32> = sqlx::query_scalar::<_, Option<i32>>(
        r#"SELECT COALESCE(
            (meta_data->>'max_categories')::int, NULL
           ) FROM business_meta
           WHERE business_id = $1 AND template = 'default'
           LIMIT 1"#
    )
    .bind(business_id)
    .fetch_optional(exec)
    .await?
    .flatten();

    Ok(override_count.map(|c| c.clamp(1, 5) as usize).unwrap_or(DEFAULT_MAX_CATEGORIES))
}
