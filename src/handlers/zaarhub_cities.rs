/// ZaarHub City Pages & Directory Listing Handlers
/// Phase 4 — Public API endpoints (no auth required)
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

// ── Query parameters ───────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListingQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
    pub category: Option<String>,
    pub sort: Option<String>, // rating, name, featured
    pub search: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub city: Option<String>,
    /// Category name or slug — the search screen's category filter.
    pub category: Option<String>,
    /// Caller position for the "Near me" filter (decimal degrees).
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    /// Search radius in METRES, exactly as the page's radius selector sends it (1000..25000).
    pub radius: Option<i32>,
    pub page: Option<i32>,
    pub per_page: Option<i32>,
}

#[derive(Deserialize)]
pub struct FeaturedQuery {
    pub page: Option<i32>,
    pub per_page: Option<i32>,
}

// ── Helper ─────────────────────────────────────────────────────

fn default_page(page: Option<i32>) -> i32 {
    page.unwrap_or(1).max(1)
}
fn default_per_page(per_page: Option<i32>) -> i32 {
    per_page.unwrap_or(20).clamp(1, 100)
}
fn offset(page: i32, per_page: i32) -> i32 {
    (page - 1) * per_page
}

/// Utility: build listing JSON from a sqlx::Row
fn listing_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "city_page_id": r.try_get::<Uuid, _>("city_page_id").map(|v| v.to_string()).unwrap_or_default(),
        "business_name": r.try_get::<String, _>("business_name").unwrap_or_default(),
        "category": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "subcategory": r.try_get::<Option<String>, _>("subcategory").unwrap_or_default(),
        "description": r.try_get::<Option<String>, _>("description").unwrap_or_default(),
        "address": r.try_get::<Option<String>, _>("address").unwrap_or_default(),
        "phone": r.try_get::<Option<String>, _>("phone").unwrap_or_default(),
        "website": r.try_get::<Option<String>, _>("website").unwrap_or_default(),
        "logo_url": r.try_get::<Option<String>, _>("logo_url").unwrap_or_default(),
        "cover_image_url": r.try_get::<Option<String>, _>("cover_image_url").unwrap_or_default(),
        "rating": r.try_get::<Option<f64>, _>("rating").unwrap_or_default(),
        "review_count": r.try_get::<i32, _>("review_count").unwrap_or(0),
        "is_featured": r.try_get::<bool, _>("is_featured").unwrap_or(false),
        "is_claimed": r.try_get::<bool, _>("is_claimed").unwrap_or(false),
        "is_editors_pick": r.try_get::<bool, _>("is_editors_pick").unwrap_or(false),
        "editors_pick_note": r.try_get::<Option<String>, _>("editors_pick_note").unwrap_or_default(),
        "deal_text": r.try_get::<Option<String>, _>("deal_text").unwrap_or_default(),
        "deal_url": r.try_get::<Option<String>, _>("deal_url").unwrap_or_default(),
        "coordinates_lat": r.try_get::<Option<f64>, _>("coordinates_lat").unwrap_or_default(),
        "coordinates_lng": r.try_get::<Option<f64>, _>("coordinates_lng").unwrap_or_default(),
        "display_order": r.try_get::<i32, _>("display_order").unwrap_or(0),
        "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").unwrap_or_default(),
    })
}

/// One public search result. The field names are what the search screen renders (`name`, `rating`,
/// `review_count`, `latitude`, `longitude`, `image_url`, `directory_slug`); the `business_name` /
/// `coordinates_lat` / `coordinates_lng` spellings are kept so older consumers of `/zaarhub/search`
/// keep working.
fn search_result_json(r: &sqlx::postgres::PgRow) -> Value {
    let id = r
        .try_get::<Uuid, _>("id")
        .map(|v| v.to_string())
        .unwrap_or_default();
    let name = r.try_get::<String, _>("name").unwrap_or_default();
    let lat = r.try_get::<Option<f64>, _>("latitude").unwrap_or_default();
    let lng = r.try_get::<Option<f64>, _>("longitude").unwrap_or_default();
    json!({
        "id": id,
        "business_id": id,
        "name": name,
        "business_name": name,
        "slug": r.try_get::<Option<String>, _>("slug").unwrap_or_default(),
        "description": r.try_get::<Option<String>, _>("description").unwrap_or_default(),
        "address": r.try_get::<Option<String>, _>("address").unwrap_or_default(),
        "city": r.try_get::<Option<String>, _>("city").unwrap_or_default(),
        "state": r.try_get::<Option<String>, _>("state").unwrap_or_default(),
        "phone": r.try_get::<Option<String>, _>("phone").unwrap_or_default(),
        "rating": r.try_get::<Option<f64>, _>("rating").unwrap_or_default(),
        "review_count": r.try_get::<Option<i32>, _>("review_count").unwrap_or_default(),
        "latitude": lat,
        "longitude": lng,
        "coordinates_lat": lat,
        "coordinates_lng": lng,
        "image_url": r.try_get::<Option<String>, _>("image_url").unwrap_or_default(),
        "is_claimed": r.try_get::<Option<bool>, _>("claimed").unwrap_or_default().unwrap_or(false),
        "category": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "category_name": r.try_get::<Option<String>, _>("category").unwrap_or_default(),
        "category_slug": r.try_get::<Option<String>, _>("category_slug").unwrap_or_default(),
        "directory_slug": r.try_get::<Option<String>, _>("directory_slug").unwrap_or_default(),
    })
}

/// List all active city pages
pub async fn list_cities(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT cp.*, (SELECT COUNT(*) FROM business_listings bl WHERE bl.city_page_id = cp.id) AS listing_count \
         FROM city_pages cp \
         WHERE cp.is_active = true \
         ORDER BY cp.display_order ASC, cp.city_name ASC"
    )
    .fetch_all(&state.db)
    .await?;

    let cities: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "tenant_id": r.try_get::<Uuid, _>("tenant_id").map(|v| v.to_string()).unwrap_or_default(),
        "city_slug": r.try_get::<String, _>("city_slug").unwrap_or_default(),
        "city_name": r.try_get::<String, _>("city_name").unwrap_or_default(),
        "state": r.try_get::<Option<String>, _>("state").unwrap_or_default(),
        "description": r.try_get::<Option<String>, _>("description").unwrap_or_default(),
        "hero_image_url": r.try_get::<Option<String>, _>("hero_image_url").unwrap_or_default(),
        "meta_title": r.try_get::<Option<String>, _>("meta_title").unwrap_or_default(),
        "meta_description": r.try_get::<Option<String>, _>("meta_description").unwrap_or_default(),
        "is_active": r.try_get::<bool, _>("is_active").unwrap_or(false),
        "display_order": r.try_get::<i32, _>("display_order").unwrap_or(0),
        // The city's own centre (migration 170). The Populate panel reads these to bias the
        // Google Places search to the operator's Radius; without them the Radius box is inert.
        "latitude": r.try_get::<Option<f64>, _>("latitude").unwrap_or_default(),
        "longitude": r.try_get::<Option<f64>, _>("longitude").unwrap_or_default(),
        "listing_count": r.try_get::<i64, _>("listing_count").unwrap_or(0),
        "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").unwrap_or_default(),
    })).collect();

    Ok(Json(json!({"cities": cities, "total": cities.len()})))
}

/// List businesses for a city with pagination, filtering, and sorting
pub async fn list_city_listings(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<ListingQuery>,
) -> ApiResult<Json<Value>> {
    let city_row =
        sqlx::query("SELECT id FROM city_pages WHERE city_slug = $1 AND is_active = true")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("City page not found: {}", slug)))?;

    let city_page_id: Uuid = city_row.get("id");
    let page = default_page(params.page);
    let per_page = default_per_page(params.per_page);

    let has_cat = params.category.is_some();
    let has_search = params.search.is_some();

    let (rows, total): (Vec<sqlx::postgres::PgRow>, i64) = match (has_cat, has_search) {
        (false, false) => {
            let count = sqlx::query(
                "SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1",
            )
            .bind(city_page_id)
            .fetch_one(&state.db)
            .await?;
            // Complete compile-time statements: `sort` is a closed whitelist (the match arms below
            // are the only sources of the ORDER BY text), so nothing is built at run time
            // (class-14 paydown, kanban t_3d273da1).
            let sql: &str = match params.sort.as_deref() {
                Some("rating") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 ORDER BY bl.rating DESC NULLS LAST, bl.review_count DESC LIMIT $2 OFFSET $3",
                Some("name") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 ORDER BY bl.business_name ASC LIMIT $2 OFFSET $3",
                Some("featured") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST LIMIT $2 OFFSET $3",
                _ => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC LIMIT $2 OFFSET $3",
            };
            let rows = sqlx::query(sql)
                .bind(city_page_id)
                .bind(per_page as i64)
                .bind(offset(page, per_page) as i64)
                .fetch_all(&state.db)
                .await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
        (true, false) => {
            let cat = params.category.as_ref().unwrap();
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1 AND category = $2")
                .bind(city_page_id).bind(cat)
                .fetch_one(&state.db).await?;
            // Complete compile-time statements: `sort` is a closed whitelist (the match arms below
            // are the only sources of the ORDER BY text), so nothing is built at run time
            // (class-14 paydown, kanban t_3d273da1).
            let sql: &str = match params.sort.as_deref() {
                Some("rating") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 ORDER BY bl.rating DESC NULLS LAST, bl.review_count DESC LIMIT $3 OFFSET $4",
                Some("name") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 ORDER BY bl.business_name ASC LIMIT $3 OFFSET $4",
                Some("featured") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST LIMIT $3 OFFSET $4",
                _ => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC LIMIT $3 OFFSET $4",
            };
            let rows = sqlx::query(sql)
                .bind(city_page_id)
                .bind(cat)
                .bind(per_page as i64)
                .bind(offset(page, per_page) as i64)
                .fetch_all(&state.db)
                .await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
        (false, true) => {
            let search = params.search.as_ref().unwrap();
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1 AND (business_name ILIKE '%' || $2 || '%' OR description ILIKE '%' || $2 || '%')")
                .bind(city_page_id).bind(search)
                .fetch_one(&state.db).await?;
            // Complete compile-time statements: `sort` is a closed whitelist (the match arms below
            // are the only sources of the ORDER BY text), so nothing is built at run time
            // (class-14 paydown, kanban t_3d273da1).
            let sql: &str = match params.sort.as_deref() {
                Some("rating") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%') ORDER BY bl.rating DESC NULLS LAST, bl.review_count DESC LIMIT $3 OFFSET $4",
                Some("name") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%') ORDER BY bl.business_name ASC LIMIT $3 OFFSET $4",
                Some("featured") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%') ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST LIMIT $3 OFFSET $4",
                _ => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND (bl.business_name ILIKE '%' || $2 || '%' OR bl.description ILIKE '%' || $2 || '%') ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC LIMIT $3 OFFSET $4",
            };
            let rows = sqlx::query(sql)
                .bind(city_page_id)
                .bind(search)
                .bind(per_page as i64)
                .bind(offset(page, per_page) as i64)
                .fetch_all(&state.db)
                .await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
        (true, true) => {
            let cat = params.category.as_ref().unwrap();
            let search = params.search.as_ref().unwrap();
            let count = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE city_page_id = $1 AND category = $2 AND (business_name ILIKE '%' || $3 || '%' OR description ILIKE '%' || $3 || '%')")
                .bind(city_page_id).bind(cat).bind(search)
                .fetch_one(&state.db).await?;
            // Complete compile-time statements: `sort` is a closed whitelist (the match arms below
            // are the only sources of the ORDER BY text), so nothing is built at run time
            // (class-14 paydown, kanban t_3d273da1).
            let sql: &str = match params.sort.as_deref() {
                Some("rating") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 AND (bl.business_name ILIKE '%' || $3 || '%' OR bl.description ILIKE '%' || $3 || '%') ORDER BY bl.rating DESC NULLS LAST, bl.review_count DESC LIMIT $4 OFFSET $5",
                Some("name") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 AND (bl.business_name ILIKE '%' || $3 || '%' OR bl.description ILIKE '%' || $3 || '%') ORDER BY bl.business_name ASC LIMIT $4 OFFSET $5",
                Some("featured") => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 AND (bl.business_name ILIKE '%' || $3 || '%' OR bl.description ILIKE '%' || $3 || '%') ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST LIMIT $4 OFFSET $5",
                _ => "SELECT bl.* FROM business_listings bl WHERE bl.city_page_id = $1 AND bl.category = $2 AND (bl.business_name ILIKE '%' || $3 || '%' OR bl.description ILIKE '%' || $3 || '%') ORDER BY bl.is_featured DESC, bl.rating DESC NULLS LAST, bl.business_name ASC LIMIT $4 OFFSET $5",
            };
            let rows = sqlx::query(sql)
                .bind(city_page_id)
                .bind(cat)
                .bind(search)
                .bind(per_page as i64)
                .bind(offset(page, per_page) as i64)
                .fetch_all(&state.db)
                .await?;
            let total: i64 = count.get("cnt");
            (rows, total)
        }
    };

    let listings: Vec<Value> = rows.iter().map(listing_json).collect();
    let total_pages = if total == 0 {
        0
    } else {
        ((total as f64) / (per_page as f64)).ceil() as i32
    };

    Ok(Json(json!({
        "listings": listings,
        "pagination": {
            "page": page,
            "per_page": per_page,
            "total": total,
            "total_pages": total_pages
        }
    })))
}

/// Get single listing detail by id
pub async fn get_listing(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let listing_uuid =
        Uuid::parse_str(&id).map_err(|_| AppError::BadRequest("Invalid listing ID".into()))?;

    let row = sqlx::query("SELECT bl.* FROM business_listings bl WHERE bl.id = $1")
        .bind(listing_uuid)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Listing not found: {}", id)))?;

    Ok(Json(json!({"listing": listing_json(&row)})))
}

/// List all unique categories across all city listings
pub async fn list_categories(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT category, cnt FROM ( \
           SELECT category, COUNT(*) AS cnt \
           FROM business_listings \
           WHERE category IS NOT NULL \
           GROUP BY category \
         ) t \
         ORDER BY \
           CASE t.category \
             WHEN 'Restaurant' THEN 1 WHEN 'Fitness Studio' THEN 2 WHEN 'Day Spa' THEN 3 \
             WHEN 'Dentist' THEN 4 WHEN 'Real Estate Agent' THEN 5 WHEN 'Hair Salon' THEN 6 \
             WHEN 'Auto Repair' THEN 7 WHEN 'Plumber' THEN 8 \
             ELSE 100 \
           END ASC,\
           t.cnt DESC",
    )
    .fetch_all(&state.db)
    .await?;

    let categories: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "name": r.try_get::<String, _>("category").unwrap_or_default(),
                "count": r.try_get::<i64, _>("cnt").unwrap_or(0),
            })
        })
        .collect();

    Ok(Json(
        json!({"categories": categories, "total": categories.len()}),
    ))
}

/// Search businesses across all directories.
///
/// The public search screen (`frontend/index.html`, `frontend/zaarhub.html` and
/// `frontend/claim.html`) renders its cards from `results` / `deals` / `total`, reading `name`,
/// `rating`, `review_count`, `latitude`, `longitude`, `image_url` and `directory_slug`, and links
/// each card to `#/city/<directory_slug>/biz/<id>` — the public business-detail route, which
/// resolves the row in `businesses`. This handler therefore searches `businesses` (the same entity
/// the detail page, the city grid and the favourites list use) and answers THAT shape.
///
/// It used to search `business_listings` (ids that mostly do not resolve on the detail page) and
/// answer `{listings: [...]}`, so every search rendered "No results found" while the API answered
/// 200 — and the `category` / `lat` / `lng` / `radius` parameters the screen sends were ignored
/// entirely (measured 2026-10-15).
pub async fn search_listings(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let page = default_page(params.page);
    let per_page = default_per_page(params.per_page);
    let search_term = params.q.unwrap_or_default().trim().to_string();
    let category = params.category.unwrap_or_default().trim().to_string();
    let city = params.city.unwrap_or_default().trim().to_string();
    let (lat, lng, radius) = (params.lat, params.lng, params.radius);

    // A blank box is not a filter: with nothing to search on, answer empty rather than list everything.
    if search_term.is_empty() && category.is_empty() {
        return Ok(Json(json!({
            "results": [],
            "deals": [],
            "listings": [],
            "total": 0,
            "pagination": { "page": 1, "per_page": per_page, "total": 0, "total_pages": 0 }
        })));
    }

    // ONE filter shared by the count / page / deals queries so the three can never drift. Every
    // optional filter is BOUND (never interpolated) and short-circuits on '' / NULL, so the SQL
    // string is static and the bind order is identical in all three. It is spliced in at COMPILE
    // time (concat! + macro), never assembled at run time.
    // `radius` is metres, exactly as the page's radius selector sends it; the distance term is the
    // haversine great-circle distance in metres.
    macro_rules! search_filter {
        () => {
            "b.is_active = true \
             AND (d.status = 'active' OR d.status IS NULL) \
             AND ($1 = '' OR b.name ILIKE '%' || $1 || '%' \
                  OR COALESCE(b.description, '') ILIKE '%' || $1 || '%' \
                  OR COALESCE(dc.name, '') ILIKE '%' || $1 || '%') \
             AND ($2 = '' OR lower(COALESCE(dc.name, '')) = lower($2) \
                  OR lower(COALESCE(dc.slug, '')) = lower($2)) \
             AND ($3 = '' OR d.slug = $3) \
             AND ($4::double precision IS NULL OR $5::double precision IS NULL OR $6::integer IS NULL \
                  OR (COALESCE(b.latitude, b.lat) IS NOT NULL AND COALESCE(b.longitude, b.lng) IS NOT NULL \
                      AND 6371000 * 2 * asin(sqrt( \
                            power(sin(radians((COALESCE(b.latitude, b.lat) - $4) / 2)), 2) \
                          + cos(radians($4)) * cos(radians(COALESCE(b.latitude, b.lat))) \
                            * power(sin(radians((COALESCE(b.longitude, b.lng) - $5) / 2)), 2) \
                      )) <= $6))"
        };
    }

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT COUNT(*) FROM businesses b \
         JOIN directories d ON d.id = b.directory_id \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE ",
        search_filter!()
    ))
    .bind(&search_term)
    .bind(&category)
    .bind(&city)
    .bind(lat)
    .bind(lng)
    .bind(radius)
    .fetch_one(&state.db)
    .await?;

    let rows = sqlx::query(concat!(
        "SELECT b.id, b.name, b.slug, b.description, b.address, b.city, b.state, b.phone, \
                b.rating, b.review_count, b.claimed, \
                COALESCE(b.latitude, b.lat)  AS latitude, \
                COALESCE(b.longitude, b.lng) AS longitude, \
                CASE WHEN COALESCE(b.logo_url, b.cover_url) LIKE 'http%' \
                       OR COALESCE(b.logo_url, b.cover_url) LIKE '/%' \
                     THEN COALESCE(b.logo_url, b.cover_url) END AS image_url, \
                dc.name AS category, dc.slug AS category_slug, \
                d.slug AS directory_slug \
         FROM businesses b \
         JOIN directories d ON d.id = b.directory_id \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE ",
        search_filter!(),
        " \
         ORDER BY b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST, b.name ASC \
         LIMIT $7 OFFSET $8"
    ))
    .bind(&search_term)
    .bind(&category)
    .bind(&city)
    .bind(lat)
    .bind(lng)
    .bind(radius)
    .bind(per_page as i64)
    .bind(offset(page, per_page) as i64)
    .fetch_all(&state.db)
    .await?;

    let results: Vec<Value> = rows.iter().map(search_result_json).collect();

    // Deals attached to matching businesses — the screen renders a Deals strip under the businesses.
    let deal_rows = sqlx::query(concat!(
        "SELECT de.id, de.title, de.deal_price, b.id AS business_id, \
                b.name AS business_name, d.slug AS directory_slug \
         FROM deals de \
         JOIN businesses b ON b.id = de.business_id \
         JOIN directories d ON d.id = b.directory_id \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE de.is_active = true AND (",
        search_filter!(),
        ") \
         ORDER BY de.featured DESC NULLS LAST, de.created_at DESC \
         LIMIT 20"
    ))
    .bind(&search_term)
    .bind(&category)
    .bind(&city)
    .bind(lat)
    .bind(lng)
    .bind(radius)
    .fetch_all(&state.db)
    .await?;

    let deals: Vec<Value> = deal_rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
                "title": r.try_get::<String, _>("title").unwrap_or_default(),
                "deal_price": r.try_get::<Option<String>, _>("deal_price").unwrap_or_default(),
                "business_id": r.try_get::<Uuid, _>("business_id").map(|v| v.to_string()).unwrap_or_default(),
                "business_name": r.try_get::<String, _>("business_name").unwrap_or_default(),
                "directory_slug": r.try_get::<String, _>("directory_slug").unwrap_or_default(),
            })
        })
        .collect();

    let total_pages = if total == 0 {
        0
    } else {
        ((total as f64) / (per_page as f64)).ceil() as i32
    };

    let results_json = Value::Array(results);

    Ok(Json(json!({
        // `results` is the key the screen reads; `listings` is kept as the legacy alias.
        "results": results_json.clone(),
        "listings": results_json,
        "deals": deals,
        "total": total,
        "pagination": {
            "page": page,
            "per_page": per_page,
            "total": total,
            "total_pages": total_pages
        }
    })))
}

// ── Phase 5: Claim/Redemption handlers ────────────────────────

#[derive(Deserialize)]
pub struct ClaimOfferBody {
    pub visitor_id: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
}

/// Get active offers for a specific listing
pub async fn listing_offers(
    State(state): State<AppState>,
    Path(listing_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let listing_uuid = Uuid::parse_str(&listing_id)
        .map_err(|_| AppError::BadRequest("Invalid listing ID".into()))?;

    let rows = sqlx::query(
        "SELECT co.* \
         FROM claim_offers co \
         JOIN business_listings bl ON co.listing_id = bl.id \
         WHERE co.listing_id = $1 AND co.is_active = true \
         AND (co.max_claims IS NULL OR co.current_claims < co.max_claims) \
         AND (co.expires_at IS NULL OR co.expires_at > now()) \
         ORDER BY co.created_at DESC",
    )
    .bind(listing_uuid)
    .fetch_all(&state.db)
    .await?;

    let offers: Vec<Value> = rows.iter().map(offer_json).collect();
    Ok(Json(json!({ "offers": offers, "total": offers.len() })))
}

/// Get a single offer by ID
pub async fn get_offer(
    State(state): State<AppState>,
    Path(offer_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let offer_uuid =
        Uuid::parse_str(&offer_id).map_err(|_| AppError::BadRequest("Invalid offer ID".into()))?;

    let row = sqlx::query("SELECT co.* FROM claim_offers co WHERE co.id = $1")
        .bind(offer_uuid)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Offer not found: {}", offer_id)))?;

    Ok(Json(json!({ "offer": offer_json(&row) })))
}

/// Claim an offer — increments claim count and returns the promo code/redemption
pub async fn claim_offer(
    State(state): State<AppState>,
    Path(offer_id): Path<String>,
    Json(body): Json<ClaimOfferBody>,
) -> ApiResult<Json<Value>> {
    let offer_uuid =
        Uuid::parse_str(&offer_id).map_err(|_| AppError::BadRequest("Invalid offer ID".into()))?;

    let visitor_id = body
        .visitor_id
        .as_deref()
        .unwrap_or("anonymous")
        .to_string();

    // 🔒 Auth gate — reject anonymous claims
    if visitor_id == "anonymous" || visitor_id.is_empty() || visitor_id.starts_with("v_") {
        return Err(AppError::Unauthorized);
    }

    // Fetch the offer and lock it for update to avoid race conditions on max_claims
    let offer_row = sqlx::query(
        "SELECT co.* FROM claim_offers co \
         WHERE co.id = $1 AND co.is_active = true \
         AND (co.max_claims IS NULL OR co.current_claims < co.max_claims) \
         AND (co.expires_at IS NULL OR co.expires_at > now()) \
         FOR UPDATE OF co",
    )
    .bind(offer_uuid)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Offer not found, expired, or fully claimed".into()))?;

    // Increment current_claims
    sqlx::query("UPDATE claim_offers SET current_claims = current_claims + 1, updated_at = now() WHERE id = $1")
        .bind(offer_uuid)
        .execute(&state.db)
        .await?;

    let promo_revealed: String = offer_row
        .try_get::<String, _>("promo_code")
        .unwrap_or_default();
    let redemption_url: Option<String> = offer_row.try_get("redemption_url").unwrap_or_default();

    // Insert claim record
    let _ = sqlx::query(
        "INSERT INTO offer_claims (offer_id, visitor_id, email, phone, promo_code_revealed, claimed_at) \
         VALUES ($1, $2, $3, $4, $5, now())"
    )
    .bind(offer_uuid)
    .bind(&visitor_id)
    .bind(&body.email)
    .bind(&body.phone)
    .bind(&promo_revealed)
    .execute(&state.db)
    .await;

    Ok(Json(json!({
        "success": true,
        "offer": offer_json(&offer_row),
        "promo_code": promo_revealed,
        "redemption_url": redemption_url,
    })))
}

/// Get a visitor's claim history
pub async fn visitor_claims(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(visitor_id): Path<String>,
) -> ApiResult<Json<Value>> {
    // A claim carries the email and phone the visitor left at the till, plus the promo code
    // they were given, so the list is readable only by that visitor (or the platform
    // operator). A guessed visitor id is not enough (kanban t_14927725 — the visitor portal
    // now shows "my claims").
    let claims =
        crate::handlers::tenant_scope::claims_from_headers(&headers, &state.config.jwt_secret)?;
    if !crate::handlers::tenant_scope::is_platform_operator(&claims) {
        let own_email: Option<String> =
            sqlx::query_scalar("SELECT email FROM visitor_accounts WHERE id::text = $1")
                .bind(&claims.sub)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);
        let is_self = claims.sub == visitor_id
            || own_email
                .as_deref()
                .map(|e| e.eq_ignore_ascii_case(&visitor_id))
                .unwrap_or(false);
        if !is_self {
            return Err(AppError::NotFound("No claims found".into()));
        }
    }

    let rows = sqlx::query(
        "SELECT oc.*, co.offer_title, co.listing_id, bl.business_name \
         FROM offer_claims oc \
         JOIN claim_offers co ON oc.offer_id = co.id \
         JOIN business_listings bl ON co.listing_id = bl.id \
         WHERE oc.visitor_id = $1 \
         ORDER BY oc.claimed_at DESC \
         LIMIT 50",
    )
    .bind(&visitor_id)
    .fetch_all(&state.db)
    .await?;

    let claims: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "offer_id": r.try_get::<Uuid, _>("offer_id").map(|v| v.to_string()).unwrap_or_default(),
        "visitor_id": r.try_get::<String, _>("visitor_id").unwrap_or_default(),
        "email": r.try_get::<Option<String>, _>("email").unwrap_or_default(),
        "phone": r.try_get::<Option<String>, _>("phone").unwrap_or_default(),
        "promo_code_revealed": r.try_get::<String, _>("promo_code_revealed").unwrap_or_default(),
        "claimed_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("claimed_at").unwrap_or_default(),
        "redeemed": r.try_get::<bool, _>("redeemed").unwrap_or(false),
        "redeemed_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("redeemed_at").unwrap_or_default(),
        "offer_title": r.try_get::<String, _>("offer_title").unwrap_or_default(),
        "listing_id": r.try_get::<Uuid, _>("listing_id").map(|v| v.to_string()).unwrap_or_default(),
        "business_name": r.try_get::<String, _>("business_name").unwrap_or_default(),
    })).collect();

    Ok(Json(json!({ "claims": claims, "total": claims.len() })))
}

/// Build offer JSON from a sqlx::Row
fn offer_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.try_get::<Uuid, _>("id").map(|v| v.to_string()).unwrap_or_default(),
        "listing_id": r.try_get::<Uuid, _>("listing_id").map(|v| v.to_string()).unwrap_or_default(),
        "offer_type": r.try_get::<String, _>("offer_type").unwrap_or_default(),
        "offer_title": r.try_get::<String, _>("offer_title").unwrap_or_default(),
        "offer_description": r.try_get::<Option<String>, _>("offer_description").unwrap_or_default(),
        "promo_code": r.try_get::<Option<String>, _>("promo_code").unwrap_or_default(),
        "coupon_image_url": r.try_get::<Option<String>, _>("coupon_image_url").unwrap_or_default(),
        "redemption_url": r.try_get::<Option<String>, _>("redemption_url").unwrap_or_default(),
        "redemption_phone": r.try_get::<Option<String>, _>("redemption_phone").unwrap_or_default(),
        "discount_value": r.try_get::<Option<String>, _>("discount_value").unwrap_or_default(),
        "expires_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("expires_at").unwrap_or_default(),
        "terms_conditions": r.try_get::<Option<String>, _>("terms_conditions").unwrap_or_default(),
        "is_active": r.try_get::<bool, _>("is_active").unwrap_or(false),
        "max_claims": r.try_get::<Option<i32>, _>("max_claims").unwrap_or_default(),
        "current_claims": r.try_get::<i32, _>("current_claims").unwrap_or(0),
        "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").unwrap_or_default(),
        "updated_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").unwrap_or_default(),
    })
}

/// Get featured listings across all cities
pub async fn featured_listings(
    State(state): State<AppState>,
    Query(q): Query<FeaturedQuery>,
) -> ApiResult<Json<Value>> {
    let page = default_page(q.page);
    let per_page = default_per_page(q.per_page);

    let count_row =
        sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings WHERE is_featured = true")
            .fetch_one(&state.db)
            .await?;
    let total: i64 = count_row.get("cnt");

    let rows = sqlx::query(
        "SELECT bl.* FROM business_listings bl \
         WHERE bl.is_featured = true \
         ORDER BY bl.rating DESC NULLS LAST, bl.business_name ASC \
         LIMIT $1 OFFSET $2",
    )
    .bind(per_page as i64)
    .bind(offset(page, per_page) as i64)
    .fetch_all(&state.db)
    .await?;

    let listings: Vec<Value> = rows.iter().map(listing_json).collect();
    let total_pages = if total == 0 {
        0
    } else {
        ((total as f64) / (per_page as f64)).ceil() as i32
    };

    Ok(Json(json!({
        "listings": listings,
        "pagination": {
            "page": page,
            "per_page": per_page,
            "total": total,
            "total_pages": total_pages
        }
    })))
}

// ── Editor's Picks API ─────────────────────────────────────────

#[derive(Deserialize)]
pub struct EditorsPickQuery {
    pub city_slug: Option<String>,
}

#[derive(Deserialize)]
pub struct EditorsPickPatch {
    pub is_editors_pick: bool,
    pub editors_pick_note: Option<String>,
}

/// GET /api/v1/zaarhub/editors-picks — list all editor's picks, optional city filter
pub async fn list_editors_picks(
    State(state): State<AppState>,
    Query(params): Query<EditorsPickQuery>,
) -> ApiResult<Json<Value>> {
    let (rows, _total) = if let Some(ref city_slug) = params.city_slug {
        let rows = sqlx::query(
            "SELECT bl.*, cp.city_name, cp.city_slug \
             FROM business_listings bl \
             JOIN city_pages cp ON bl.city_page_id = cp.id \
             WHERE bl.is_editors_pick = true AND cp.city_slug = $1 \
             ORDER BY bl.rating DESC NULLS LAST, bl.business_name ASC",
        )
        .bind(city_slug)
        .fetch_all(&state.db)
        .await?;
        let cnt = rows.len() as i64;
        (rows, cnt)
    } else {
        let rows = sqlx::query(
            "SELECT bl.*, cp.city_name, cp.city_slug \
             FROM business_listings bl \
             JOIN city_pages cp ON bl.city_page_id = cp.id \
             WHERE bl.is_editors_pick = true \
             ORDER BY bl.rating DESC NULLS LAST, bl.business_name ASC",
        )
        .fetch_all(&state.db)
        .await?;
        let cnt = rows.len() as i64;
        (rows, cnt)
    };

    let picks: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut j = listing_json(r);
            let city_name: String = r.try_get("city_name").unwrap_or_default();
            let city_slug: String = r.try_get("city_slug").unwrap_or_default();
            if let Some(obj) = j.as_object_mut() {
                obj.insert("city_name".into(), json!(city_name));
                obj.insert("city_slug".into(), json!(city_slug));
            }
            j
        })
        .collect();

    Ok(Json(json!({
        "editor_picks": picks,
        "total": _total
    })))
}

/// PATCH /api/v1/zaarhub/admin/listings/:id/editors-pick — toggle editors_pick on a listing
pub async fn toggle_editors_pick(
    State(state): State<AppState>,
    Path(listing_id): Path<String>,
    Json(payload): Json<EditorsPickPatch>,
) -> ApiResult<Json<Value>> {
    let listing_uuid = Uuid::parse_str(&listing_id)
        .map_err(|_| AppError::BadRequest("Invalid listing ID".into()))?;

    sqlx::query(
        "UPDATE business_listings SET is_editors_pick = $1, editors_pick_note = $2, updated_at = now() WHERE id = $3"
    )
    .bind(payload.is_editors_pick)
    .bind(&payload.editors_pick_note)
    .bind(listing_uuid)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "success": true,
        "listing_id": listing_id,
        "is_editors_pick": payload.is_editors_pick,
        "editors_pick_note": payload.editors_pick_note
    })))
}
