//! PublicPage CRUD handlers for Multi-Directory API.

use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::auth::models::Claims;
use crate::auth::middleware::verify_token;
use crate::AppState;
use crate::error::{AppError, ApiResult};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PublicPage {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub original_price: Option<String>,
    pub public_page_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    pub terms: Option<String>,
    pub redemption_limit: Option<i32>,
    pub redemption_count: Option<i32>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Uuid,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub public_page_type: Option<String>,
    pub coupon_code: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePublicPageRequest {
    pub title: String,
    pub description: Option<String>,
    pub original_price: Option<String>,
    pub public_page_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    pub terms: Option<String>,
    pub redemption_limit: Option<i32>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Uuid,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub public_page_type: Option<String>,
    pub coupon_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePublicPageRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub original_price: Option<String>,
    pub public_page_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    pub terms: Option<String>,
    pub redemption_limit: Option<i32>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub public_page_type: Option<String>,
    pub coupon_code: Option<String>,
}

/// GET /api/v1/public_pages — list all public_pages
pub async fn list_public_pages(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let public_pages = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(public_pages))
}

/// GET /api/v1/public_pages/featured — featured public_pages across all directories
pub async fn list_featured_public_pages(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let public_pages = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages WHERE featured = true AND status = 'active' ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(public_pages))
}

/// POST /api/v1/public_pages — create a public_page
pub async fn create_public_page(
    State(s): State<AppState>,
    Json(req): Json<CreatePublicPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let public_page = sqlx::query_as::<_, PublicPage>(
        "INSERT INTO public_pages (title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413, \x2414, \x2415, \x2416, \x2417) RETURNING id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at "
    )
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.original_price)
    .bind(&req.public_page_price)
    .bind(req.discount_percent)
    .bind(req.currency.as_deref().unwrap_or("USD"))
    .bind(&req.image_url)
    .bind(&req.terms)
    .bind(req.redemption_limit)
    .bind(req.status.as_deref().unwrap_or("active"))
    .bind(req.directory_id)
    .bind(req.business_id)
    .bind(req.start_date)
    .bind(req.end_date)
    .bind(req.featured.unwrap_or(false))
    .bind(req.public_page_type.as_deref().unwrap_or("coupon"))
    .bind(&req.coupon_code)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(public_page)))
}

/// GET /api/v1/public_pages/:id — get single public_page
pub async fn get_public_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let public_page = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("PublicPage not found".into()))?;

    Ok(Json(public_page))
}

/// PUT /api/v1/public_pages/:id — update public_page
pub async fn update_public_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePublicPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("PublicPage not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let description = req.description.or(existing.description);
    let original_price = req.original_price.or(existing.original_price);
    let public_page_price = req.public_page_price.or(existing.public_page_price);
    let discount_percent = req.discount_percent.or(existing.discount_percent);
    let currency = req.currency.or(existing.currency);
    let image_url = req.image_url.or(existing.image_url);
    let terms = req.terms.or(existing.terms);
    let redemption_limit = req.redemption_limit.or(existing.redemption_limit);
    let status = req.status.or(existing.status);
    let directory_id = req.directory_id.or(existing.directory_id);
    let business_id = req.business_id.unwrap_or(existing.business_id);
    let start_date = req.start_date.or(existing.start_date);
    let end_date = req.end_date.or(existing.end_date);
    let featured = req.featured.or(existing.featured);
    let public_page_type = req.public_page_type.or(existing.public_page_type);
    let coupon_code = req.coupon_code.or(existing.coupon_code);

    let public_page = sqlx::query_as::<_, PublicPage>(
        "UPDATE public_pages SET title = \x241, description = \x242, original_price = \x243, public_page_price = \x244, discount_percent = \x245, currency = \x246, image_url = \x247, terms = \x248, redemption_limit = \x249, status = \x2410, directory_id = \x2411, business_id = \x2412, start_date = \x2413, end_date = \x2414, featured = \x2415, public_page_type = \x2416, coupon_code = \x2417, updated_at = NOW() WHERE id = \x2418 RETURNING id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at "
    )
    .bind(&title)
    .bind(&description)
    .bind(&original_price)
    .bind(&public_page_price)
    .bind(discount_percent)
    .bind(&currency)
    .bind(&image_url)
    .bind(&terms)
    .bind(redemption_limit)
    .bind(&status)
    .bind(directory_id)
    .bind(business_id)
    .bind(start_date)
    .bind(end_date)
    .bind(featured)
    .bind(&public_page_type)
    .bind(&coupon_code)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(public_page))
}

/// DELETE /api/v1/public_pages/:id — delete public_page
pub async fn delete_public_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM public_pages WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("PublicPage not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/public_pages/:id/claim — increment redemption_count
pub async fn claim_public_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let public_page = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("PublicPage not found".into()))?;

    if let Some(limit) = public_page.redemption_limit {
        let count = public_page.redemption_count.unwrap_or(0);
        if count >= limit {
            return Err(AppError::BadRequest("Redemption limit reached for this public_page".into()));
        }
    }

    let updated = sqlx::query_as::<_, PublicPage>(
        "UPDATE public_pages SET redemption_count = COALESCE(redemption_count, 0) + 1, updated_at = NOW() WHERE id = \x241 RETURNING id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at "
    )
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(updated))
}

/// GET /api/v1/directories/:slug/public_pages — public_pages for a directory
pub async fn list_directory_public_pages(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let public_pages = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages WHERE directory_id = \x241 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(public_pages))
}

/// GET /api/v1/directories/:slug/businesses/:business_id/public_pages — public_pages for a business
pub async fn list_business_public_pages(
    State(s): State<AppState>,
    Path((slug, business_id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let public_pages = sqlx::query_as::<_, PublicPage>(
        "SELECT id, title, description, original_price, public_page_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, public_page_type, coupon_code, created_at, updated_at FROM public_pages WHERE directory_id = \x241 AND business_id = \x242 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(public_pages))
}


// ── Landing Pages (re-exported from public_pages) ────────────────
pub use crate::handlers::public_pages::{
    list_landing_pages,
    create_landing_page,
    get_landing_page,
    update_landing_page,
    delete_landing_page,
    toggle_publish,
    list_public_themes,
    create_public_theme,
    get_public_theme,
    update_public_theme,
    delete_public_theme,
};

// ── Homepage / Directory / Business data endpoints ──────────────

pub async fn homepage_data(
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"status": "ok", "message": "Homepage data endpoint"})))
}

pub async fn directory_data(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"slug": slug, "message": "Directory data endpoint"})))
}

pub async fn business_data(
    State(s): State<AppState>,
    Path((slug, business_id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    // 1. Look up directory by slug
    let directory = sqlx::query_as::<_, crate::models::Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    // 2. Look up business by ID (try UUID first, then slug)
    let business = if let Ok(bid) = Uuid::parse_str(&business_id) {
        sqlx::query_as::<_, crate::models::Business>(
            "SELECT * FROM businesses WHERE id = \x241 AND directory_id = \x242 "
        )
        .bind(bid)
        .bind(directory.id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, crate::models::Business>(
            "SELECT * FROM businesses WHERE slug = \x241 AND directory_id = \x242 "
        )
        .bind(&business_id)
        .bind(directory.id)
        .fetch_optional(&s.db)
        .await?
    };
    let business = business.ok_or(AppError::NotFound("Business not found".to_string()))?;

    // 3. Look up business meta (AI-optimized fields from business_meta table)
    let meta = sqlx::query_as::<_, crate::models::BusinessMeta>(
        "SELECT * FROM business_meta WHERE business_id = \x241 AND template = \x242 "
    )
    .bind(business.id)
    .bind(crate::template_engine::TEMPLATE_BUSINESS_DETAIL)
    .fetch_optional(&s.db)
    .await?
    .map(|m| m.meta_data)
    .unwrap_or_else(|| json!({}));

    // 4. Resolve color scheme from the directory's color_scheme column
    let colors = crate::template_engine::normalize_color_scheme(directory.color_scheme.clone());

    // 5. Build template context — individual business, not a list
    //    We provide a "business" object, "directory" info, and "colors".
    //    AI-optimized fields come from meta if populated, otherwise empty.
    //    The template uses {{#if}} guards for optional sections.

    // Merge business fields with meta fields
    let mut business_obj = serde_json::to_value(&business).unwrap_or_else(|_| json!({}));

    // Convert business fields to snake_case object for template
    if let Some(obj) = business_obj.as_object_mut() {
        // Add category name if business has a category_id
        if let Some(cat_id) = business.category_id {
            let cat = sqlx::query_scalar::<_, Option<String>>(
                "SELECT name FROM directory_categories WHERE id = \x241 "
            )
            .bind(cat_id)
            .fetch_optional(&s.db)
            .await?;
            if let Some(Some(cat_name)) = cat {
                obj.insert("category".to_string(), json!(cat_name));
            }
        }

        // Merge meta fields into business object so template can access
        // {{business.specialty}}, {{business.metric}}, etc.
        if let Some(meta_obj) = meta.as_object() {
            for (k, v) in meta_obj {
                obj.insert(k.clone(), v.clone());
            }
        }

        // Provide sensible defaults where template expects data
        obj.entry("schema_type").or_insert_with(|| json!("LocalBusiness"));
        obj.entry("description_plain").or_insert_with(|| {
            business.description.clone().map(|d| {
                // Strip HTML tags for plain-text description
                d.replace("<p>", "").replace("</p>", " ")
                 .replace("<br>", " ")
                 .replace("<br/>", " ")
                 .replace('<', "")
                 .replace('>', "")
                 .trim().to_string()
            }).unwrap_or_default().into()
        });
        obj.entry("image_url").or_insert_with(|| json!(""));
        obj.entry("price_range").or_insert_with(|| json!("$$"));
        obj.entry("excerpt").or_insert_with(|| json!(""));
        obj.entry("years_in_business").or_insert_with(|| json!(""));
        obj.entry("response_time").or_insert_with(|| json!(""));
        obj.entry("specialty").or_insert_with(|| json!(""));
        obj.entry("metric").or_insert_with(|| json!(""));
        obj.entry("service_area").or_insert_with(|| json!(""));
        obj.entry("review_highlight").or_insert_with(|| json!(""));
        obj.entry("response_detail").or_insert_with(|| json!(""));
        obj.entry("pain_point").or_insert_with(|| json!(""));
        obj.entry("differentiator").or_insert_with(|| json!(""));
        obj.entry("approach").or_insert_with(|| json!(""));
        obj.entry("competitors").or_insert_with(|| json!([]));
        obj.entry("metric_name_1").or_insert_with(|| json!(""));
        obj.entry("metric_name_2").or_insert_with(|| json!(""));
        obj.entry("metric_name_3").or_insert_with(|| json!(""));
        obj.entry("price_min").or_insert_with(|| json!(""));
        obj.entry("price_max").or_insert_with(|| json!(""));
        obj.entry("price_variable").or_insert_with(|| json!(""));
        obj.entry("flat_rate").or_insert_with(|| json!(""));
        obj.entry("booking_method").or_insert_with(|| json!(""));
        obj.entry("typical_wait").or_insert_with(|| json!(""));
        obj.entry("license_info").or_insert_with(|| json!("licensed"));
        obj.entry("insurance_info").or_insert_with(|| json!("general liability insurance"));
        obj.entry("certifications").or_insert_with(|| json!(""));
    }

    // Build directory view model with extra template fields
    let mut dir_obj = serde_json::to_value(&directory).unwrap_or_else(|_| json!({}));
    if let Some(obj) = dir_obj.as_object_mut() {
        // Derive region from directory city and business state
        let dir_state = business.state.clone().unwrap_or_default();
        obj.entry("region").or_insert_with(|| {
            let city = directory.city.as_deref().unwrap_or("");
            if city.is_empty() && dir_state.is_empty() {
                json!("")
            } else if city.is_empty() {
                json!(dir_state)
            } else if dir_state.is_empty() {
                json!(city)
            } else {
                json!(format!("{}, {}", city, dir_state))
            }
        });
        // Add state from business (directory table has no state column)
        obj.entry("state").or_insert_with(|| json!(dir_state));
        obj.entry("neighborhood_1").or_insert_with(|| json!(""));
        obj.entry("neighborhood_2").or_insert_with(|| json!(""));
        obj.entry("nearby_areas").or_insert_with(|| json!([]));
        obj.entry("wikipedia_slug").or_insert_with(|| json!(""));
    }

    let context = serde_json::json!({
        "business": business_obj,
        "directory": dir_obj,
        "colors": colors,
    });

    // 6. Render template
    let engine = s.template_engine.lock().unwrap();
    let html = engine.render_directory_page(
        crate::template_engine::TEMPLATE_BUSINESS_DETAIL,
        &context,
    )
    .map_err(|e| AppError::Internal(e))?;

    Ok(axum::response::Html(html))
}

/// GET /api/v1/saved-places — render saved places page for a visitor
/// Requires visitor JWT auth. Shows bookmarked businesses.
/// GET /api/v1/saved-places — render saved places page for a visitor
/// Requires visitor JWT auth. Shows bookmarked businesses.
pub async fn saved_places_page(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    // Manually verify visitor JWT from Authorization header
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok());
    
    let visitor_id = match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = header.strip_prefix("Bearer ").unwrap_or("");
            match verify_token(token, &s.config.jwt_secret) {
                Ok(claims) => {
                    match Uuid::parse_str(&claims.sub) {
                        Ok(id) => id,
                        Err(_) => {
                            // Invalid sub, redirect to visitor portal
                            return Ok(axum::response::Redirect::to("/visitor").into_response());
                        }
                    }
                }
                Err(_) => {
                    // Invalid token, redirect to visitor portal
                    return Ok(axum::response::Redirect::to("/visitor").into_response());
                }
            }
        }
        _ => {
            // No auth, redirect to visitor portal
            return Ok(axum::response::Redirect::to("/visitor").into_response());
        }
    };

    // Fetch visitor's favorites
    let favorites = sqlx::query_as::<_, super::visitors::FavoriteBusinessRow>(
        r#"SELECT
            vf.id,
            vf.created_at as saved_at,
            b.id as business_id,
            b.name as business_name,
            b.slug as business_slug,
            b.city,
            b.state,
            dc.name as category_name,
            b.images,
            b.rating,
            b.review_count,
            b.phone,
            d.slug as directory_slug
        FROM visitor_favorites vf
        JOIN businesses b ON b.id = vf.business_id
        LEFT JOIN directory_categories dc ON dc.id = b.category_id
        JOIN directories d ON d.id = vf.directory_id
        WHERE vf.visitor_account_id = $1
        ORDER BY vf.created_at DESC"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    // Enhance with image_url from images JSON array
    let saved_places: Vec<serde_json::Value> = favorites.into_iter().map(|f| {
        let image_url = f.images.as_ref()
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let description = None::<String>; // Could fetch from business.description if needed
        json!({
            "id": f.id,
            "saved_at": f.saved_at,
            "business_id": f.business_id,
            "business_name": f.business_name,
            "business_slug": f.business_slug,
            "city": f.city,
            "state": f.state,
            "category_name": f.category_name,
            "image_url": image_url,
            "rating": f.rating,
            "review_count": f.review_count,
            "phone": f.phone,
            "directory_slug": f.directory_slug,
            "description": description,
        })
    }).collect();

    // Build context — use a default directory for nav
    let dir_val = json!({
        "name": "Multi-Directory",
        "slug": "",
        "description": ""
    });
    let colors = crate::template_engine::normalize_color_scheme(None);

    let context = json!({
        "directory": dir_val,
        "colors": colors,
        "saved_places": saved_places,
    });

    let engine = s.template_engine.lock().unwrap();
    let html = engine.render_directory_page(
        crate::template_engine::TEMPLATE_SAVED_PLACES,
        &context,
    )
    .map_err(|e| AppError::Internal(e))?;

    Ok(axum::response::Html(html).into_response())
}

/// GET /api/v1/my-bookings — server-rendered My Bookings page (Stage 5)
pub async fn my_bookings_page(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    // Manually verify visitor JWT from Authorization header
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok());
    
    let visitor_id = match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = header.strip_prefix("Bearer ").unwrap_or("");
            match verify_token(token, &s.config.jwt_secret) {
                Ok(claims) => {
                    match Uuid::parse_str(&claims.sub) {
                        Ok(id) => id,
                        Err(_) => {
                            return Ok(axum::response::Redirect::to("/visitor").into_response());
                        }
                    }
                }
                Err(_) => {
                    return Ok(axum::response::Redirect::to("/visitor").into_response());
                }
            }
        }
        _ => {
            return Ok(axum::response::Redirect::to("/visitor").into_response());
        }
    };

    // Fetch visitor's bookings
    let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Option<String>, Option<String>,
        Option<chrono::DateTime<chrono::Utc>>, Option<String>,
        String, Option<String>, chrono::DateTime<chrono::Utc>, String)>(
        r#"SELECT sb.id, sb.directory_id, sb.business_id, sb.service_name, sb.description,
                  sb.preferred_date, sb.preferred_time, sb.status, sb.notes, sb.created_at,
                  b.name as business_name
           FROM service_bookings sb
           JOIN businesses b ON b.id = sb.business_id
           WHERE sb.visitor_account_id = $1
           ORDER BY sb.created_at DESC"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    let bookings: Vec<Value> = rows.into_iter().map(|(id, dir_id, biz_id, svc_name, desc,
        pref_date, pref_time, status, notes, created_at, biz_name)| {
        json!({
            "id": id,
            "business_name": biz_name,
            "service_name": svc_name,
            "description": desc,
            "preferred_date": pref_date,
            "preferred_time": pref_time,
            "status": status,
            "notes": notes,
            "created_at": created_at,
        })
    }).collect();

    // Build context
    let dir_val = json!({
        "name": "Multi-Directory",
        "slug": "",
        "description": ""
    });
    let colors = crate::template_engine::normalize_color_scheme(None);

    let context = json!({
        "directory": dir_val,
        "colors": colors,
        "bookings": bookings,
    });

    let engine = s.template_engine.lock().unwrap();
    let html = engine.render_directory_page(
        crate::template_engine::TEMPLATE_MY_BOOKINGS,
        &context,
    )
    .map_err(|e| AppError::Internal(e))?;

    Ok(axum::response::Html(html).into_response())
}

