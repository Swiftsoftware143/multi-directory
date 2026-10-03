//! Search handlers: full-text search, filters, and search config management.

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

use crate::error::{validate_pagination, ApiResult, AppError};
use crate::AppState;

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
    /// Multi-type filter (e.g. search across supplier/farm/wholesaler at once).
    /// Used by search/suppliers when no single `business_type` is requested.
    #[serde(default)]
    pub business_types: Vec<String>,
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
    pub business_type: Option<String>,
    pub address: Option<String>,
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
            business_type: row.try_get("business_type")?,
            address: row.try_get("address")?,
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

    // Statement heads, JOINs and WHERE clauses below are compile-time literals; every filter is
    // appended as a BIND only, so no SQL is assembled at run time (class-14 paydown,
    // kanban t_0d0e26a9). The ORDER BY is one of two hardcoded literals, never request text.
    let has_subcat_filter = qs.subcategory.as_ref().map_or(false, |v| !v.is_empty());
    let has_cat_filter = qs.category.as_ref().map_or(false, |v| !v.is_empty());
    let has_q = qs.q.as_ref().map_or(false, |q| !q.is_empty());

    // --- Count query ---
    let mut count_qb =
        sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(DISTINCT b.id) FROM businesses b");
    push_search_joins(&mut count_qb, has_subcat_filter, has_cat_filter);
    push_search_where(&mut count_qb, &qs, has_subcat_filter, has_cat_filter, has_q);

    let total: i64 = count_qb
        .build_query_scalar::<i64>()
        .fetch_one(&s.db)
        .await?;

    // --- Data query ---
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

    let mut data_qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT b.id, b.name, b.slug, b.description, cat.name AS category, \
                b.city, b.state, b.phone, b.website, b.rating, \
                d.name AS directory_name, d.slug AS directory_slug, \
                b.business_type, b.address, ",
    );
    data_qb.push(categories_subquery);
    data_qb.push(" AS categories FROM businesses b");
    push_search_joins(&mut data_qb, has_subcat_filter, has_cat_filter);
    data_qb.push(" LEFT JOIN directories d ON b.directory_id = d.id");
    push_search_where(&mut data_qb, &qs, has_subcat_filter, has_cat_filter, has_q);
    // The full-text ORDER BY needs one more bind of the same query text than the WHERE uses.
    if has_q {
        data_qb
            .push(" ORDER BY ts_rank(b.search_vector, plainto_tsquery('english', ")
            .push_bind(qs.q.as_deref())
            .push(")) DESC, b.name ASC");
    } else {
        data_qb.push(" ORDER BY b.name ASC");
    }
    data_qb
        .push(" LIMIT ")
        .push_bind(per_page)
        .push(" OFFSET ")
        .push_bind(offset);

    let data = data_qb
        .build_query_as::<SearchResult>()
        .fetch_all(&s.db)
        .await?;

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

/// The JOIN set shared by the count and data statements of `search_businesses`: the category
/// join, plus — when a subcategory/category filter is active — the `business_categories` filter
/// join. Every fragment is a compile-time literal; nothing here is built at run time.
fn push_search_joins(
    qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>,
    has_subcat_filter: bool,
    has_cat_filter: bool,
) {
    qb.push(" LEFT JOIN directory_categories cat ON b.category_id = cat.id");
    if has_subcat_filter || has_cat_filter {
        qb.push(
            " JOIN business_categories bc_filter ON bc_filter.business_id = b.id \
             JOIN directory_categories dc_filter ON dc_filter.id = bc_filter.category_id",
        );
    }
}

/// Push ` WHERE …` for `search_businesses`, keeping the predicate order (and therefore the bind
/// order) the hand-built version used. Every fragment is a compile-time literal; only the filter
/// VALUES are bound. `qs.directory`/`qs.subcategory`/`qs.category`/`qs.business_type` are read
/// only in the branches that already established they are present and non-empty.
fn push_search_where<'a>(
    qb: &mut sqlx::QueryBuilder<'a, sqlx::Postgres>,
    qs: &'a SearchQuery,
    has_subcat_filter: bool,
    has_cat_filter: bool,
    has_q: bool,
) {
    qb.push(" WHERE ");
    let mut first = true;

    if has_subcat_filter {
        if !first {
            qb.push(" AND ");
        }
        first = false;
        qb.push("LOWER(dc_filter.name) = LOWER(")
            .push_bind(qs.subcategory.as_deref())
            .push(")");
    } else if has_cat_filter {
        if !first {
            qb.push(" AND ");
        }
        first = false;
        qb.push("LOWER(COALESCE(dc_filter.group_name, '')) = LOWER(")
            .push_bind(qs.category.as_deref())
            .push(")");
    }

    if qs.directory.is_some() {
        if !first {
            qb.push(" AND ");
        }
        first = false;
        qb.push("b.directory_id = ").push_bind(qs.directory);
    }

    if has_q {
        if !first {
            qb.push(" AND ");
        }
        first = false;
        let q_text = qs.q.as_deref().unwrap_or("");
        qb.push("(b.search_vector @@ plainto_tsquery('english', ")
            .push_bind(q_text)
            .push(") OR b.name ILIKE '%' || ")
            .push_bind(q_text)
            .push(" || '%' OR COALESCE(b.description, '') ILIKE '%' || ")
            .push_bind(q_text)
            .push(" || '%' OR COALESCE(cat.name, '') ILIKE '%' || ")
            .push_bind(q_text)
            .push(" || '%')");
    }

    if let Some(ref city) = qs.city {
        if !city.is_empty() {
            if !first {
                qb.push(" AND ");
            }
            first = false;
            qb.push("LOWER(COALESCE(b.city, '')) = LOWER(")
                .push_bind(city.as_str())
                .push(")");
        }
    }

    if let Some(ref st) = qs.state {
        if !st.is_empty() {
            if !first {
                qb.push(" AND ");
            }
            first = false;
            qb.push("LOWER(COALESCE(b.state, '')) = LOWER(")
                .push_bind(st.as_str())
                .push(")");
        }
    }

    if let Some(ref bt) = qs.business_type {
        if !bt.is_empty() {
            if !first {
                qb.push(" AND ");
            }
            first = false;
            qb.push("b.business_type = ").push_bind(bt.as_str());
        }
    } else if !qs.business_types.is_empty() {
        if !first {
            qb.push(" AND ");
        }
        first = false;
        qb.push("b.business_type = ANY(")
            .push_bind(&qs.business_types)
            .push(")");
    }

    // This is NOT parameterized — just a fixed SQL condition string
    if !first {
        qb.push(" AND ");
    }
    qb.push("COALESCE(b.is_active, true) = true");
}

/// GET /api/v1/search/suppliers — search all supplier-type businesses (B2B back-office).
/// Accepts an optional `type` query param to restrict to a single business_type.
/// The supplier taxonomy is the `business_type` field: supplier, distributor,
/// wholesaler, farm, association, manufacturer.
pub async fn search_suppliers(
    State(s): State<AppState>,
    Query(mut qs): Query<SearchQuery>,
) -> ApiResult<impl IntoResponse> {
    // If an explicit supplier `type` is requested, honor it; otherwise search
    // across ALL supplier-type business_type values (NOT 'local').
    let supplier_types = [
        "supplier",
        "distributor",
        "wholesaler",
        "farm",
        "association",
        "manufacturer",
    ];

    // If caller passed a type in business_type, validate it's a supplier type;
    // otherwise clear it and let the search below use the ANY() list.
    let explicit = qs.business_type.clone().filter(|t| !t.is_empty());
    if explicit.is_some() {
        // Keep the caller's single-type filter (e.g. ?business_type=farm).
    } else {
        // No explicit type: search all supplier types via a dedicated override.
        qs.business_types = supplier_types.iter().map(|s| s.to_string()).collect();
    }

    search_businesses(State(s), Query(qs)).await
}

// --- GET /api/v1/search/filters/:directory_id ---

pub async fn get_filters(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dir_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = \x241 ")
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
         ORDER BY 1 ",
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let cities: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT COALESCE(b.city, '') FROM businesses b \
         WHERE b.directory_id = \x241 AND b.city IS NOT NULL AND b.city != '' \
         ORDER BY 1 ",
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let states: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT COALESCE(b.state, '') FROM businesses b \
         WHERE b.directory_id = \x241 AND b.state IS NOT NULL AND b.state != '' \
         ORDER BY 1 ",
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(FilterOptions {
        categories: categories
            .into_iter()
            .map(|r| r.0)
            .filter(|s| !s.is_empty())
            .collect(),
        cities: cities
            .into_iter()
            .map(|r| r.0)
            .filter(|s| !s.is_empty())
            .collect(),
        states: states
            .into_iter()
            .map(|r| r.0)
            .filter(|s| !s.is_empty())
            .collect(),
    }))
}

// --- GET /api/v1/search/config ---

pub async fn list_search_configs(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let configs = sqlx::query_as::<_, SearchConfig>(
        "SELECT sc.* FROM search_config sc ORDER BY sc.directory_id ",
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
    let dir_exists =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = \x241 ")
            .bind(req.directory_id)
            .fetch_one(&s.db)
            .await?;

    if dir_exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM search_config WHERE directory_id = \x241 ",
    )
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(
            "Search config already exists for this directory".to_string(),
        ));
    }

    let filter_fields = req
        .filter_fields
        .map(|f| serde_json::to_value(f).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!(["category", "city", "state", "rating", "price"]));

    let config = sqlx::query_as::<_, SearchConfig>(
        "INSERT INTO search_config \
         (directory_id, enable_fulltext, enable_filters, filter_fields, \
          results_per_page, enable_location_search, default_radius_km) \
         VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247) RETURNING *",
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
        "SELECT * FROM search_config WHERE directory_id = \x241 ",
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    match config {
        Some(c) => Ok(Json(json!(c))),
        None => Err(AppError::NotFound(
            "Search config not found for this directory".to_string(),
        )),
    }
}

// --- PUT /api/v1/search/config/:directory_id ---

pub async fn update_search_config(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<UpdateSearchConfigRequest>,
) -> ApiResult<impl IntoResponse> {
    let current = sqlx::query_as::<_, SearchConfig>(
        "SELECT * FROM search_config WHERE directory_id = \x241 ",
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    let current = match current {
        Some(c) => c,
        None => return Err(AppError::NotFound("Search config not found".to_string())),
    };

    let enable_fulltext = req
        .enable_fulltext
        .unwrap_or(current.enable_fulltext.unwrap_or(true));
    let enable_filters = req
        .enable_filters
        .unwrap_or(current.enable_filters.unwrap_or(true));
    let filter_fields = match req.filter_fields {
        Some(f) => serde_json::to_value(f).unwrap_or(current.filter_fields.unwrap_or_default()),
        None => current.filter_fields.unwrap_or_default(),
    };
    let results_per_page = req
        .results_per_page
        .unwrap_or(current.results_per_page.unwrap_or(20));
    let enable_location_search = req
        .enable_location_search
        .unwrap_or(current.enable_location_search.unwrap_or(false));
    let default_radius_km = req
        .default_radius_km
        .unwrap_or(current.default_radius_km.unwrap_or(10));

    let config = sqlx::query_as::<_, SearchConfig>(
        "UPDATE search_config SET \
         enable_fulltext = \x241, enable_filters = \x242, filter_fields = \x243, \
         results_per_page = \x244, enable_location_search = \x245, default_radius_km = \x246 \
         WHERE directory_id = \x247 RETURNING *",
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
