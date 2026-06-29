//! Landing Page and Public Theme CRUD handlers for Multi-Directory API.

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

// ===== Landing Page =====

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct LandingPage {
    pub id: Uuid,
    pub title: String,
    pub slug: String,
    pub directory_id: Option<Uuid>,
    pub hero_title: Option<String>,
    pub hero_subtitle: Option<String>,
    pub hero_cta_text: Option<String>,
    pub hero_cta_url: Option<String>,
    pub features: serde_json::Value,
    pub testimonials: serde_json::Value,
    pub faq: serde_json::Value,
    pub seo_title: Option<String>,
    pub seo_description: Option<String>,
    pub published: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLandingPageRequest {
    pub title: String,
    pub slug: String,
    pub directory_id: Option<Uuid>,
    pub hero_title: Option<String>,
    pub hero_subtitle: Option<String>,
    pub hero_cta_text: Option<String>,
    pub hero_cta_url: Option<String>,
    pub features: Option<serde_json::Value>,
    pub testimonials: Option<serde_json::Value>,
    pub faq: Option<serde_json::Value>,
    pub seo_title: Option<String>,
    pub seo_description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLandingPageRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub directory_id: Option<Uuid>,
    pub hero_title: Option<String>,
    pub hero_subtitle: Option<String>,
    pub hero_cta_text: Option<String>,
    pub hero_cta_url: Option<String>,
    pub features: Option<serde_json::Value>,
    pub testimonials: Option<serde_json::Value>,
    pub faq: Option<serde_json::Value>,
    pub seo_title: Option<String>,
    pub seo_description: Option<String>,
    pub published: Option<bool>,
}

/// GET /api/v1/landing-pages — list all landing pages
pub async fn list_landing_pages(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let pages = sqlx::query_as::<_, LandingPage>(
        "SELECT id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at FROM landing_pages ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(pages))
}

/// POST /api/v1/landing-pages — create a landing page
pub async fn create_landing_page(
    State(s): State<AppState>,
    Json(req): Json<CreateLandingPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let page = sqlx::query_as::<_, LandingPage>(
        "INSERT INTO landing_pages (title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412) RETURNING id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at "
    )
    .bind(&req.title)
    .bind(&req.slug)
    .bind(req.directory_id)
    .bind(&req.hero_title)
    .bind(&req.hero_subtitle)
    .bind(&req.hero_cta_text)
    .bind(&req.hero_cta_url)
    .bind(req.features.unwrap_or(serde_json::Value::Array(vec![])))
    .bind(req.testimonials.unwrap_or(serde_json::Value::Array(vec![])))
    .bind(req.faq.unwrap_or(serde_json::Value::Array(vec![])))
    .bind(&req.seo_title)
    .bind(&req.seo_description)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(page)))
}

/// GET /api/v1/landing-pages/:id — get single landing page
pub async fn get_landing_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let page = sqlx::query_as::<_, LandingPage>(
        "SELECT id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at FROM landing_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Landing page not found".into()))?;

    Ok(Json(page))
}

/// PUT /api/v1/landing-pages/:id — update landing page
pub async fn update_landing_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLandingPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, LandingPage>(
        "SELECT id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at FROM landing_pages WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Landing page not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let slug = req.slug.unwrap_or(existing.slug);
    let directory_id = req.directory_id.or(existing.directory_id);
    let hero_title = req.hero_title.or(existing.hero_title);
    let hero_subtitle = req.hero_subtitle.or(existing.hero_subtitle);
    let hero_cta_text = req.hero_cta_text.or(existing.hero_cta_text);
    let hero_cta_url = req.hero_cta_url.or(existing.hero_cta_url);
    let features = req.features.unwrap_or(existing.features);
    let testimonials = req.testimonials.unwrap_or(existing.testimonials);
    let faq = req.faq.unwrap_or(existing.faq);
    let seo_title = req.seo_title.or(existing.seo_title);
    let seo_description = req.seo_description.or(existing.seo_description);
    let published = req.published.unwrap_or(existing.published);

    let page = sqlx::query_as::<_, LandingPage>(
        "UPDATE landing_pages SET title = \x241, slug = \x242, directory_id = \x243, hero_title = \x244, hero_subtitle = \x245, hero_cta_text = \x246, hero_cta_url = \x247, features = \x248, testimonials = \x249, faq = \x2410, seo_title = \x2411, seo_description = \x2412, published = \x2413, updated_at = NOW() WHERE id = \x2414 RETURNING id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at "
    )
    .bind(&title)
    .bind(&slug)
    .bind(directory_id)
    .bind(&hero_title)
    .bind(&hero_subtitle)
    .bind(&hero_cta_text)
    .bind(&hero_cta_url)
    .bind(&features)
    .bind(&testimonials)
    .bind(&faq)
    .bind(&seo_title)
    .bind(&seo_description)
    .bind(published)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(page))
}

/// DELETE /api/v1/landing-pages/:id — delete landing page
pub async fn delete_landing_page(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM landing_pages WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Landing page not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/landing-pages/:slug/publish — toggle publish
pub async fn toggle_publish(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let page = sqlx::query_as::<_, LandingPage>(
        "UPDATE landing_pages SET published = NOT published, updated_at = NOW() WHERE slug = \x241 RETURNING id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Landing page not found".into()))?;

    Ok(Json(page))
}

/// GET /api/v1/directories/:slug/landing-pages — landing pages for a directory
pub async fn directory_landing_pages(
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

    let pages = sqlx::query_as::<_, LandingPage>(
        "SELECT id, title, slug, directory_id, hero_title, hero_subtitle, hero_cta_text, hero_cta_url, features, testimonials, faq, seo_title, seo_description, published, created_at, updated_at FROM landing_pages WHERE directory_id = \x241 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(pages))
}

// ===== Public Theme =====

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PublicTheme {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub directory_id: Option<Uuid>,
    pub primary_color: String,
    pub secondary_color: String,
    pub header_style: String,
    pub layout: String,
    pub show_search: bool,
    pub show_categories: bool,
    pub show_featured: bool,
    pub items_per_page: i32,
    pub custom_css: Option<String>,
    pub custom_js: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateThemeRequest {
    pub name: String,
    pub slug: String,
    pub directory_id: Option<Uuid>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub header_style: Option<String>,
    pub layout: Option<String>,
    pub show_search: Option<bool>,
    pub show_categories: Option<bool>,
    pub show_featured: Option<bool>,
    pub items_per_page: Option<i32>,
    pub custom_css: Option<String>,
    pub custom_js: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateThemeRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub directory_id: Option<Uuid>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub header_style: Option<String>,
    pub layout: Option<String>,
    pub show_search: Option<bool>,
    pub show_categories: Option<bool>,
    pub show_featured: Option<bool>,
    pub items_per_page: Option<i32>,
    pub custom_css: Option<String>,
    pub custom_js: Option<String>,
}

/// GET /api/v1/public-themes — list all themes
pub async fn list_public_themes(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let themes = sqlx::query_as::<_, PublicTheme>(
        "SELECT id, name, slug, directory_id, primary_color, secondary_color, header_style, layout, show_search, show_categories, show_featured, items_per_page, custom_css, custom_js, created_at FROM public_themes ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(themes))
}

/// POST /api/v1/public-themes — create a theme
pub async fn create_public_theme(
    State(s): State<AppState>,
    Json(req): Json<CreateThemeRequest>,
) -> ApiResult<impl IntoResponse> {
    let theme = sqlx::query_as::<_, PublicTheme>(
        "INSERT INTO public_themes (name, slug, directory_id, primary_color, secondary_color, header_style, layout, show_search, show_categories, show_featured, items_per_page, custom_css, custom_js) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413) RETURNING id, name, slug, directory_id, primary_color, secondary_color, header_style, layout, show_search, show_categories, show_featured, items_per_page, custom_css, custom_js, created_at "
    )
    .bind(&req.name)
    .bind(&req.slug)
    .bind(req.directory_id)
    .bind(req.primary_color.as_deref().unwrap_or("#2563eb"))
    .bind(req.secondary_color.as_deref().unwrap_or("#1e40af"))
    .bind(req.header_style.as_deref().unwrap_or("gradient"))
    .bind(req.layout.as_deref().unwrap_or("grid"))
    .bind(req.show_search.unwrap_or(true))
    .bind(req.show_categories.unwrap_or(true))
    .bind(req.show_featured.unwrap_or(true))
    .bind(req.items_per_page.unwrap_or(12))
    .bind(&req.custom_css)
    .bind(&req.custom_js)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(theme)))
}

/// GET /api/v1/public-themes/:id — get single theme
pub async fn get_public_theme(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let theme = sqlx::query_as::<_, PublicTheme>(
        "SELECT id, name, slug, directory_id, primary_color, secondary_color, header_style, layout, show_search, show_categories, show_featured, items_per_page, custom_css, custom_js, created_at FROM public_themes WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Theme not found".into()))?;

    Ok(Json(theme))
}

/// PUT /api/v1/public-themes/:id — update theme
pub async fn update_public_theme(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateThemeRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, PublicTheme>(
        "SELECT id, name, slug, directory_id, primary_color, secondary_color, header_style, layout, show_search, show_categories, show_featured, items_per_page, custom_css, custom_js, created_at FROM public_themes WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Theme not found".into()))?;

    let name = req.name.unwrap_or(existing.name);
    let slug = req.slug.unwrap_or(existing.slug);
    let directory_id = req.directory_id.or(existing.directory_id);
    let primary_color = req.primary_color.unwrap_or(existing.primary_color);
    let secondary_color = req.secondary_color.unwrap_or(existing.secondary_color);
    let header_style = req.header_style.unwrap_or(existing.header_style);
    let layout = req.layout.unwrap_or(existing.layout);
    let show_search = req.show_search.unwrap_or(existing.show_search);
    let show_categories = req.show_categories.unwrap_or(existing.show_categories);
    let show_featured = req.show_featured.unwrap_or(existing.show_featured);
    let items_per_page = req.items_per_page.unwrap_or(existing.items_per_page);
    let custom_css = req.custom_css.or(existing.custom_css);
    let custom_js = req.custom_js.or(existing.custom_js);

    let theme = sqlx::query_as::<_, PublicTheme>(
        "UPDATE public_themes SET name = \x241, slug = \x242, directory_id = \x243, primary_color = \x244, secondary_color = \x245, header_style = \x246, layout = \x247, show_search = \x248, show_categories = \x249, show_featured = \x2410, items_per_page = \x2411, custom_css = \x2412, custom_js = \x2413 WHERE id = \x2414 RETURNING id, name, slug, directory_id, primary_color, secondary_color, header_style, layout, show_search, show_categories, show_featured, items_per_page, custom_css, custom_js, created_at "
    )
    .bind(&name)
    .bind(&slug)
    .bind(directory_id)
    .bind(&primary_color)
    .bind(&secondary_color)
    .bind(&header_style)
    .bind(&layout)
    .bind(show_search)
    .bind(show_categories)
    .bind(show_featured)
    .bind(items_per_page)
    .bind(&custom_css)
    .bind(&custom_js)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(theme))
}

/// DELETE /api/v1/public-themes/:id — delete theme
pub async fn delete_public_theme(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM public_themes WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Theme not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}
