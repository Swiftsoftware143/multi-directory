//! Business CRUD and search handlers.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult, validate_pagination};
use crate::models::*;

/// GET /api/v1/directories/:slug/businesses
pub async fn list_businesses(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(qs): Query<ListBusinessesQuery>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let (page, per_page) = validate_pagination(qs.page, qs.per_page);
    let offset = (page - 1) * per_page;
    let dir_id = dir.id;

    // Build dynamic query
    let mut where_clauses = vec!["b.directory_id = \x241".to_string()];
    let mut param_idx = 2;

    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            where_clauses.push(format!(
                "to_tsvector('english', b.name || ' ' || COALESCE(b.description, '')) @@ plainto_tsquery('english', ${})",
                param_idx
            ));
            param_idx += 1;
        }
    }

    if let Some(cat_id) = qs.category_id {
        where_clauses.push(format!("b.category_id = ${}", param_idx));
        param_idx += 1;
    }

    if let Some(ref city) = qs.city {
        if !city.is_empty() {
            where_clauses.push(format!("LOWER(b.city) = LOWER(${})", param_idx));
            param_idx += 1;
        }
    }

    if qs.lat.is_some() && qs.lng.is_some() && qs.radius.is_some() {
        where_clauses.push(format!(
            "b.latitude IS NOT NULL AND b.longitude IS NOT NULL AND \
             (6371 * acos(cos(radians(${})) * cos(radians(b.latitude)) * \
             cos(radians(b.longitude) - radians(${})) + sin(radians(${})) * sin(radians(b.latitude)))) < ${}",
            param_idx, param_idx + 1, param_idx, param_idx + 2
        ));
        param_idx += 3;
    }

    let where_str = where_clauses.join(" AND ");

    // Count query
    let count_sql = format!(
        "SELECT COUNT(*) FROM businesses b WHERE {}",
        where_str
    );

    // Build query params for count
    let mut count_q = sqlx::query_as::<_, (i64,)>(&count_sql).bind(dir_id);
    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            count_q = count_q.bind(q);
        }
    }
    if let Some(cat_id) = qs.category_id {
        count_q = count_q.bind(cat_id);
    }
    if let Some(ref city) = qs.city {
        if !city.is_empty() {
            count_q = count_q.bind(city);
        }
    }
    if qs.lat.is_some() && qs.lng.is_some() && qs.radius.is_some() {
        count_q = count_q.bind(qs.lat).bind(qs.lng).bind(qs.radius);
    }

    let (total,): (i64,) = count_q.fetch_one(&s.db).await?;

    // Sort
    let order_by = match qs.sort.as_deref() {
        Some("rating") => "b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST",
        Some("newest") => "b.created_at DESC",
        Some("oldest") => "b.created_at ASC",
        Some("name") => "b.name ASC",
        _ => "b.name ASC",
    };

    // Data query
    let data_sql = format!(
        "SELECT b.* FROM businesses b WHERE {} ORDER BY {} LIMIT ${} OFFSET ${}",
        where_str, order_by, param_idx, param_idx + 1
    );

    let mut data_q = sqlx::query_as::<_, Business>(&data_sql).bind(dir_id);
    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            data_q = data_q.bind(q);
        }
    }
    if let Some(cat_id) = qs.category_id {
        data_q = data_q.bind(cat_id);
    }
    if let Some(ref city) = qs.city {
        if !city.is_empty() {
            data_q = data_q.bind(city);
        }
    }
    if qs.lat.is_some() && qs.lng.is_some() && qs.radius.is_some() {
        data_q = data_q.bind(qs.lat).bind(qs.lng).bind(qs.radius);
    }

    data_q = data_q.bind(per_page).bind(offset);
    let businesses = data_q.fetch_all(&s.db).await?;

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!(PaginatedResponse {
        data: businesses,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// GET /api/v1/directories/:slug/businesses/:business_id_or_slug
pub async fn get_business(
    State(s): State<AppState>,
    Path((slug, business_id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    // Try by UUID first, then by slug
    let business = if let Ok(bid) = Uuid::parse_str(&business_id) {
        sqlx::query_as::<_, Business>(
            "SELECT * FROM businesses WHERE id = \x241 AND directory_id = \x242 "
        )
        .bind(bid)
        .bind(dir.id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, Business>(
            "SELECT * FROM businesses WHERE slug = \x241 AND directory_id = \x242 "
        )
        .bind(&business_id)
        .bind(dir.id)
        .fetch_optional(&s.db)
        .await?
    };

    let business = business.ok_or(AppError::NotFound("Business not found".to_string()))?;

    Ok(Json(json!(business)))
}

/// POST /api/v1/directories/:slug/businesses
pub async fn create_business(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateBusinessRequest>,
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

    // Check if business with this slug already exists in this directory
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM businesses WHERE directory_id = \x241 AND slug = \x242 "
    )
    .bind(dir.id)
    .bind(&req.slug)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!(
            "Business with slug '{}' already exists in this directory", req.slug
        )));
    }

    let business = sqlx::query_as::<_, Business>(
        r#"INSERT INTO businesses (directory_id, name, slug, description, category_id,
           address, city, state, zip, phone, email, website, latitude, longitude)
           VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413, \x2414)
           RETURNING *"#
    )
    .bind(dir.id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.category_id)
    .bind(&req.address)
    .bind(&req.city)
    .bind(&req.state)
    .bind(&req.zip)
    .bind(&req.phone)
    .bind(&req.email)
    .bind(&req.website)
    .bind(req.latitude)
    .bind(req.longitude)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(business))))
}

/// PUT /api/v1/directories/:slug/businesses/:business_id
pub async fn update_business(
    State(s): State<AppState>,
    Path((_slug, business_id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateBusinessRequest>,
) -> ApiResult<impl IntoResponse> {
    let _existing = sqlx::query_as::<_, Business>(
        "SELECT * FROM businesses WHERE id = \x241 "
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Business not found".to_string()))?;

    let business = sqlx::query_as::<_, Business>(
        r#"UPDATE businesses SET
           name = COALESCE(\x241, name),
           slug = COALESCE(\x242, slug),
           description = COALESCE(\x243, description),
           category_id = COALESCE(\x244, category_id),
           address = COALESCE(\x245, address),
           city = COALESCE(\x246, city),
           state = COALESCE(\x247, state),
           zip = COALESCE(\x248, zip),
           phone = COALESCE(\x249, phone),
           email = COALESCE(\x2410, email),
           website = COALESCE(\x2411, website),
           latitude = COALESCE(\x2412, latitude),
           longitude = COALESCE(\x2413, longitude),
           is_active = COALESCE(\x2414, is_active),
           updated_at = NOW()
           WHERE id = \x2415 RETURNING *"#
    )
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.category_id)
    .bind(&req.address)
    .bind(&req.city)
    .bind(&req.state)
    .bind(&req.zip)
    .bind(&req.phone)
    .bind(&req.email)
    .bind(&req.website)
    .bind(req.latitude)
    .bind(req.longitude)
    .bind(req.is_active)
    .bind(business_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(business)))
}

/// GET /api/v1/listings

#[derive(Debug, Serialize)]
pub struct ListBusinessResult {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub rating: Option<f64>,
    pub directory_name: Option<String>,
    pub directory_slug: Option<String>,
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for ListBusinessResult {
    fn from_row(row: &sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            slug: row.try_get("slug")?,
            description: row.try_get("description")?,
            category: row.try_get("category")?,
            city: row.try_get("city")?,
            state: row.try_get("state")?,
            phone: row.try_get("phone")?,
            website: row.try_get("website")?,
            rating: row.try_get("rating")?,
            directory_name: row.try_get("directory_name")?,
            directory_slug: row.try_get("directory_slug")?,
        })
    }
}

pub async fn list_all_businesses(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let businesses = sqlx::query_as::<_, ListBusinessResult>(
        "SELECT b.id, b.name, b.slug, b.description, cat.name AS category, \
                b.city, b.state, b.phone, b.website, b.rating, \
                d.name AS directory_name, d.slug AS directory_slug \
         FROM businesses b \
         LEFT JOIN directory_categories cat ON b.category_id = cat.id \
         LEFT JOIN directories d ON b.directory_id = d.id \
         ORDER BY b.name"
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(businesses))
}

/// DELETE /api/v1/directories/:slug/businesses/:business_id
pub async fn delete_business(
    State(s): State<AppState>,
    Path((_slug, business_id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM businesses WHERE id = \x241")
        .bind(business_id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Business not found".to_string()));
    }

    Ok(Json(json!({"message": "Business deleted successfully"})))
}

/// GET /api/v1/directories/:slug/businesses/suggestions?q=...
pub async fn search_suggestions(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<axum::Json<serde_json::Value>> {
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let q = params.get("q").cloned().unwrap_or_default();
    let limit = params.get("limit").and_then(|v| v.parse::<i64>().ok()).unwrap_or(10);

    if q.len() < 2 {
        let empty: Vec<BusinessSearchResult> = Vec::new();
        return Ok(Json(json!(empty)));
    }

    let results: Vec<BusinessSearchResult> = sqlx::query_as::<_, BusinessSearchResult>(
        r#"SELECT b.id, b.name, b.slug, b.city, b.s, dc.name as category_name
           FROM businesses b
           LEFT JOIN directory_categories dc ON b.category_id = dc.id
           WHERE b.directory_id = \x241
             AND (b.name ILIKE \x242 OR b.city ILIKE \x242)
           ORDER BY
             CASE WHEN b.name ILIKE \x242 THEN 0 ELSE 1 END,
             b.name ASC
           LIMIT \x243"#
    )
    .bind(dir.id)
    .bind(format!("%{}%", q))
    .bind(limit)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(serde_json::json!(results)))
}
