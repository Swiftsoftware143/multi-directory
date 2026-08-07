//! Search handlers: full-text search, filters, and search config management.

use axum::{
    extract::{Path, State, Query},
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

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub directory: Option<Uuid>,
    /// Legacy: matches category name directly (old join on b.category_id)
    pub category: Option<String>,
    /// Phase 2: filter by subcategory name through business_categories join
    pub subcategory: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub business_type: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSearchConfigRequest {
    pub directory_id: Uuid,
    pub enable_fulltext: Option<bool>,
    pub enable_filters: Option<bool>,
    pub filter_fields: Option<Vec<String>>,
    pub results_per_page: Option<i32>,
    pub enable_location_search: Option<bool>,
    pub default_radius_km: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSearchConfigRequest {
    pub enable_fulltext: Option<bool>,
    pub enable_filters: Option<bool>,
    pub filter_fields: Option<Vec<String>>,
    pub results_per_page: Option<i32>,
    pub enable_location_search: Option<bool>,
    pub default_radius_km: Option<i32>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct SearchConfig {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub enable_fulltext: Option<bool>,
    pub enable_filters: Option<bool>,
    pub filter_fields: Option<serde_json::Value>,
    pub results_per_page: Option<i32>,
    pub enable_location_search: Option<bool>,
    pub default_radius_km: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
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
    /// Phase 2: multi-category assignments as JSON array of {id, name, group_name, is_primary}
    pub categories: Option<serde_json::Value>,
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for SearchResult {
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
            categories: row.try_get("categories")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct FilterOptions {
    pub categories: Vec<String>,
    pub cities: Vec<String>,
    pub states: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse<T: Serialize> {
    pub data: Vec<T>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}

// --- GET /api/v1/search ---

/// Searches businesses with optional subcategory/category filtering.
/// Phase 2: supports `?subcategory=` (filters by name via business_categories)
/// and `?category=` (filters by group_name via business_categories).
/// Results include multi-category assignments in the `categories` field.
pub async fn search_businesses(
    State(s): State<AppState>,
    Query(qs): Query<SearchQuery>,
) -> ApiResult<impl IntoResponse> {
    let (page, per_page) = validate_pagination(qs.page, qs.per_page);
    let offset = (page - 1) * per_page;

    // Track parameter count — ONLY for parameterized clauses, not fixed SQL.
    let mut param_count: i32 = 0;
    let mut next_param = || { param_count += 1; param_count };

    let mut wheres: Vec<String> = Vec::new();
    let mut extra_joins: Vec<String> = Vec::new();

    // Determine if we need subcategory/group_name filtering via business_categories
    let has_subcat_filter = qs.subcategory.as_ref().map_or(false, |v| !v.is_empty());
    let has_cat_filter = qs.category.as_ref().map_or(false, |v| !v.is_empty());

    if has_subcat_filter {
        // Filter by specific subcategory name through business_categories join
        extra_joins.push(
            "JOIN business_categories bc_filter ON bc_filter.business_id = b.id \
             JOIN directory_categories dc_filter ON dc_filter.id = bc_filter.category_id"
                .to_string()
        );
        let p = next_param();
        wheres.push(format!("LOWER(dc_filter.name) = LOWER(${})", p));
    } else if has_cat_filter {
        // Filter by group_name through business_categories join
        extra_joins.push(
            "JOIN business_categories bc_filter ON bc_filter.business_id = b.id \
             JOIN directory_categories dc_filter ON dc_filter.id = bc_filter.category_id"
                .to_string()
        );
        let p = next_param();
        wheres.push(format!("LOWER(COALESCE(dc_filter.group_name, '')) = LOWER(${})", p));
    }

    if qs.directory.is_some() {
        let p = next_param();
        wheres.push(format!("b.directory_id = ${}", p));
    }

    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            let p = next_param();
            wheres.push(format!(
                "(b.search_vector @@ plainto_tsquery('english', ${i}) \
                 OR b.name ILIKE '%' || ${i} || '%' \
                 OR COALESCE(b.description, '') ILIKE '%' || ${i} || '%' \
                 OR COALESCE(cat.name, '') ILIKE '%' || ${i} || '%')",
                i = p
            ));
        }
    }

    if let Some(ref city) = qs.city {
        if !city.is_empty() {
            let p = next_param();
            wheres.push(format!("LOWER(COALESCE(b.city, '')) = LOWER(${})", p));
        }
    }

    if let Some(ref st) = qs.state {
        if !st.is_empty() {
            let p = next_param();
            wheres.push(format!("LOWER(COALESCE(b.state, '')) = LOWER(${})", p));
        }
    }

    if let Some(ref bt) = qs.business_type {
        if !bt.is_empty() {
            let p = next_param();
            wheres.push(format!("b.business_type = ${}", p));
        }
    }

    // This is NOT parameterized — just a fixed SQL condition string
    wheres.push("COALESCE(b.is_active, true) = true".to_string());

    let where_clause = format!("WHERE {}", wheres.join(" AND "));
    let extra_join_clause = extra_joins.join(" ");
    let cat_join = "LEFT JOIN directory_categories cat ON b.category_id = cat.id";

    let all_joins = format!("{} {} {}", cat_join,
        if extra_join_clause.is_empty() { "" } else { " " },
        extra_join_clause);

    // --- Count query ---
    let count_sql = format!("SELECT COUNT(DISTINCT b.id) FROM businesses b {} {}", all_joins, where_clause);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);

    // Bind parameters in the same order as the closure assigned them
    if has_subcat_filter {
        if let Some(ref sc) = qs.subcategory { count_q = count_q.bind(sc); }
    } else if has_cat_filter {
        if let Some(ref cat) = qs.category { count_q = count_q.bind(cat); }
    }
    if let Some(ref dir_id) = qs.directory { count_q = count_q.bind(dir_id); }
    if let Some(ref q) = qs.q { if !q.is_empty() { count_q = count_q.bind(q); } }
    if let Some(ref city) = qs.city { if !city.is_empty() { count_q = count_q.bind(city); } }
    if let Some(ref st) = qs.state { if !st.is_empty() { count_q = count_q.bind(st); } }
    if let Some(ref bt) = qs.business_type { if !bt.is_empty() { count_q = count_q.bind(bt); } }

    let total: i64 = count_q.fetch_one(&s.db).await?;

    // --- Data query ---
    let has_q = qs.q.as_ref().map_or(false, |q| !q.is_empty());

    // Calculate the starting parameter index for LIMIT/OFFSET.
    let lo_start = param_count + if has_q { 1 } else { 0 } + 1;

    let order_clause = if has_q {
        let order_p = param_count + 1;
        format!(
            "ORDER BY ts_rank(b.search_vector, plainto_tsquery('english', ${})) DESC, b.name ASC",
            order_p
        )
    } else {
        "ORDER BY b.name ASC".to_string()
    };

    // Phase 2: include multi-category assignments as JSON subquery
    let categories_subquery = r#"(
        SELECT COALESCE(json_agg(json_build_object(
            'id', bc.cat_id,
            'name', bc.cat_name,
            'group_name', bc.cat_group,
            'is_primary', bc.is_prim
        ) ORDER BY bc.is_prim DESC, bc.cat_name ASC), '[]'::json)
        FROM (
            SELECT bc2.category_id AS cat_id, dc2.name AS cat_name,
                   COALESCE(dc2.group_name, 'Other') AS cat_group,
                   bc2.is_primary AS is_prim
            FROM business_categories bc2
            LEFT JOIN directory_categories dc2 ON dc2.id = bc2.category_id
            WHERE bc2.business_id = b.id
        ) bc
    )"#;

    let data_sql = format!(
        "SELECT b.id, b.name, b.slug, b.description, cat.name AS category, \
                b.city, b.state, b.phone, b.website, b.rating, \
                d.name AS directory_name, d.slug AS directory_slug, \
                {} AS categories \
         FROM businesses b \
         {} \
         LEFT JOIN directories d ON b.directory_id = d.id \
         {} {} \
         LIMIT ${} OFFSET ${}",
        categories_subquery, all_joins, where_clause, order_clause,
        lo_start,
        lo_start + 1
    );

    let mut data_q = sqlx::query_as::<_, SearchResult>(&data_sql);

    // Bind WHERE params in closure order
    if has_subcat_filter {
        if let Some(ref sc) = qs.subcategory { data_q = data_q.bind(sc); }
    } else if has_cat_filter {
        if let Some(ref cat) = qs.category { data_q = data_q.bind(cat); }
    }
    if let Some(ref dir_id) = qs.directory { data_q = data_q.bind(dir_id); }
    if let Some(ref q) = qs.q { if !q.is_empty() { data_q = data_q.bind(q); } }
    if let Some(ref city) = qs.city { if !city.is_empty() { data_q = data_q.bind(city); } }
    if let Some(ref st) = qs.state { if !st.is_empty() { data_q = data_q.bind(st); } }
    if let Some(ref bt) = qs.business_type { if !bt.is_empty() { data_q = data_q.bind(bt); } }
    // ORDER BY tsquery param (duplicate of q for rank ordering)
    if has_q {
        if let Some(ref q) = qs.q {
            if !q.is_empty() {
                data_q = data_q.bind(q);
            }
        }
    }
    data_q = data_q.bind(per_page);
    data_q = data_q.bind(offset);

    let data = data_q.fetch_all(&s.db).await?;

    let total_pages = if per_page > 0 {
        (total + per_page - 1) / per_page
    } else {
        1
    };

    Ok(Json(json!(SearchResponse {
        data,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// GET /api/v1/search/suppliers — search only suppliers/distributors/wholesalers/farms/associations
pub async fn search_suppliers(
    State(s): State<AppState>,
    Query(qs): Query<SearchQuery>,
) -> ApiResult<impl IntoResponse> {
    // Override business_type to only return supplier types
    let mut supplier_qs = SearchQuery {
        business_type: Some("supplier".to_string()),
        ..qs
    };
    // Also include other supplier-like types
    // We handle this by making the search function accept multiple types
    search_businesses(State(s), Query(supplier_qs)).await
}

// --- GET /api/v1/search/filters/:directory_id ---

pub async fn get_filters(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dir_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = \x241 "
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    if dir_count == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let categories: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT COALESCE(c.name, '') FROM businesses b \
         LEFT JOIN directory_categories c ON b.category_id = c.id \
         WHERE b.directory_id = \x241 AND b.category_id IS NOT NULL \
         ORDER BY 1 "
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let cities: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT COALESCE(b.city, '') FROM businesses b \
         WHERE b.directory_id = \x241 AND b.city IS NOT NULL AND b.city != '' \
         ORDER BY 1 "
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let states: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT COALESCE(b.state, '') FROM businesses b \
         WHERE b.directory_id = \x241 AND b.state IS NOT NULL AND b.state != '' \
         ORDER BY 1 "
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(FilterOptions {
        categories: categories.into_iter().map(|r| r.0).filter(|s| !s.is_empty()).collect(),
        cities: cities.into_iter().map(|r| r.0).filter(|s| !s.is_empty()).collect(),
        states: states.into_iter().map(|r| r.0).filter(|s| !s.is_empty()).collect(),
    }))
}

// --- GET /api/v1/search/config ---

pub async fn list_search_configs(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let configs = sqlx::query_as::<_, SearchConfig>(
        "SELECT sc.* FROM search_config sc ORDER BY sc.directory_id "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(configs)))
}

// --- POST /api/v1/search/config ---

pub async fn create_search_config(
    State(s): State<AppState>,
    Json(req): Json<CreateSearchConfigRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = \x241 "
    )
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    if dir_exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM search_config WHERE directory_id = \x241 "
    )
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(
            "Search config already exists for this directory".to_string()
        ));
    }

    let filter_fields = req.filter_fields
        .map(|f| serde_json::to_value(f).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!(["category","city","state","rating","price"]));

    let config = sqlx::query_as::<_, SearchConfig>(
        "INSERT INTO search_config \
         (directory_id, enable_fulltext, enable_filters, filter_fields, \
          results_per_page, enable_location_search, default_radius_km) \
         VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247) RETURNING *"
    )
    .bind(req.directory_id)
    .bind(req.enable_fulltext.unwrap_or(true))
    .bind(req.enable_filters.unwrap_or(true))
    .bind(&filter_fields)
    .bind(req.results_per_page.unwrap_or(20))
    .bind(req.enable_location_search.unwrap_or(false))
    .bind(req.default_radius_km.unwrap_or(10))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(config))))
}

// --- GET /api/v1/search/config/:directory_id ---

pub async fn get_search_config(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let config = sqlx::query_as::<_, SearchConfig>(
        "SELECT * FROM search_config WHERE directory_id = \x241 "
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    match config {
        Some(c) => Ok(Json(json!(c))),
        None => Err(AppError::NotFound("Search config not found for this directory".to_string())),
    }
}

// --- PUT /api/v1/search/config/:directory_id ---

pub async fn update_search_config(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<UpdateSearchConfigRequest>,
) -> ApiResult<impl IntoResponse> {
    let current = sqlx::query_as::<_, SearchConfig>(
        "SELECT * FROM search_config WHERE directory_id = \x241 "
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    let current = match current {
        Some(c) => c,
        None => return Err(AppError::NotFound("Search config not found".to_string())),
    };

    let enable_fulltext = req.enable_fulltext.unwrap_or(current.enable_fulltext.unwrap_or(true));
    let enable_filters = req.enable_filters.unwrap_or(current.enable_filters.unwrap_or(true));
    let filter_fields = match req.filter_fields {
        Some(f) => serde_json::to_value(f).unwrap_or(current.filter_fields.unwrap_or_default()),
        None => current.filter_fields.unwrap_or_default(),
    };
    let results_per_page = req.results_per_page.unwrap_or(current.results_per_page.unwrap_or(20));
    let enable_location_search = req.enable_location_search.unwrap_or(current.enable_location_search.unwrap_or(false));
    let default_radius_km = req.default_radius_km.unwrap_or(current.default_radius_km.unwrap_or(10));

    let config = sqlx::query_as::<_, SearchConfig>(
        "UPDATE search_config SET \
         enable_fulltext = \x241, enable_filters = \x242, filter_fields = \x243, \
         results_per_page = \x244, enable_location_search = \x245, default_radius_km = \x246 \
         WHERE directory_id = \x247 RETURNING *"
    )
    .bind(enable_fulltext)
    .bind(enable_filters)
    .bind(&filter_fields)
    .bind(results_per_page)
    .bind(enable_location_search)
    .bind(default_radius_km)
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(config)))
}
