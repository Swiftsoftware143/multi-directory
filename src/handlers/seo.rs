//! SEO handlers for Multi-Directory API.
//! Manages per-page meta tags, sitemap config, and robots.txt.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Models ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SeoMeta {
    pub id: Uuid,
    pub page_type: String,
    pub page_id: Option<Uuid>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<String>,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub schema_type: Option<String>,
    pub custom_schema: Option<Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSeoMetaRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<String>,
    pub og_image: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub schema_type: Option<String>,
    pub custom_schema: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SitemapConfig {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub auto_generate: Option<bool>,
    pub priority: Option<f64>,   // DECIMAL(2,1) maps to f64 via sqlx
    pub change_freq: Option<String>,
    pub last_generated: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSitemapConfigRequest {
    pub auto_generate: Option<bool>,
    pub priority: Option<f64>,
    pub change_freq: Option<String>,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// GET /api/v1/seo/:page_type/:page_id — get SEO meta for a page
pub async fn get_seo_meta(
    State(s): State<AppState>,
    Path((page_type, page_id)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let page_id = if page_id == "homepage" || page_id == "null" || page_id == "none" {
        None
    } else {
        Some(Uuid::parse_str(&page_id).map_err(|_| AppError::BadRequest("Invalid page_id UUID".to_string()))?)
    };

    let meta = sqlx::query_as::<_, SeoMeta>(
        "SELECT id, page_type, page_id, title, description, keywords, og_image, og_title, \
         og_description, schema_type, custom_schema, created_at, updated_at \
         FROM seo_meta WHERE page_type = \x241 AND (page_id = \x242 OR (page_id IS NULL AND \x242 IS NULL))",
    )
    .bind(&page_type)
    .bind(page_id)
    .fetch_optional(&s.db)
    .await?;

    match meta {
        Some(m) => Ok(Json(serde_json::json!(m))),
        None => {
            Ok(Json(serde_json::json!({
                "page_type": page_type,
                "page_id": page_id,
                "title": null,
                "description": null,
                "keywords": null,
                "og_image": null,
                "og_title": null,
                "og_description": null,
                "schema_type": null,
                "custom_schema": null
            })))
        }
    }
}

/// PUT /api/v1/seo/:page_type/:page_id — upsert SEO meta
pub async fn update_seo_meta(
    State(s): State<AppState>,
    Path((page_type, page_id)): Path<(String, String)>,
    Json(body): Json<UpdateSeoMetaRequest>,
) -> ApiResult<impl IntoResponse> {
    let page_id = if page_id == "homepage" || page_id == "null" || page_id == "none" {
        None
    } else {
        Some(Uuid::parse_str(&page_id).map_err(|_| AppError::BadRequest("Invalid page_id UUID".to_string()))?)
    };

    let result = sqlx::query_as::<_, SeoMeta>(
        "INSERT INTO seo_meta (page_type, page_id, title, description, keywords, og_image, \
         og_title, og_description, schema_type, custom_schema, updated_at) \
         VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, NOW()) \
         ON CONFLICT (page_type, page_id) DO UPDATE SET \
         title = COALESCE(\x243, seo_meta.title), \
         description = COALESCE(\x244, seo_meta.description), \
         keywords = COALESCE(\x245, seo_meta.keywords), \
         og_image = COALESCE(\x246, seo_meta.og_image), \
         og_title = COALESCE(\x247, seo_meta.og_title), \
         og_description = COALESCE(\x248, seo_meta.og_description), \
         schema_type = COALESCE(\x249, seo_meta.schema_type), \
         custom_schema = COALESCE(\x2410, seo_meta.custom_schema), \
         updated_at = NOW() \
         RETURNING id, page_type, page_id, title, description, keywords, og_image, og_title, \
         og_description, schema_type, custom_schema, created_at, updated_at",
    )
    .bind(&page_type)
    .bind(page_id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(&body.keywords)
    .bind(&body.og_image)
    .bind(&body.og_title)
    .bind(&body.og_description)
    .bind(&body.schema_type)
    .bind(&body.custom_schema)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::OK, Json(serde_json::json!(result))))
}

/// GET /api/v1/seo/sitemap-config/:directory_id — get sitemap config for directory
pub async fn get_sitemap_config(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let config = sqlx::query_as::<_, SitemapConfig>(
        "SELECT id, directory_id, auto_generate, \
         CAST(priority AS DOUBLE PRECISION) as priority, \
         change_freq, last_generated, created_at \
         FROM sitemap_config WHERE directory_id = \x241",
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    match config {
        Some(c) => Ok(Json(serde_json::json!(c))),
        None => Ok(Json(serde_json::json!({
            "directory_id": directory_id,
            "auto_generate": true,
            "priority": 0.5,
            "change_freq": "weekly",
            "last_generated": null
        }))),
    }
}

/// PUT /api/v1/seo/sitemap-config/:directory_id — upsert sitemap config
pub async fn update_sitemap_config(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(body): Json<UpdateSitemapConfigRequest>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query_as::<_, SitemapConfig>(
        "INSERT INTO sitemap_config (directory_id, auto_generate, priority, change_freq) \
         VALUES (\x241, \x242, \x243::DECIMAL(2,1), \x244) \
         ON CONFLICT (directory_id) DO UPDATE SET \
         auto_generate = COALESCE(\x242, sitemap_config.auto_generate), \
         priority = COALESCE(\x243::DECIMAL(2,1), sitemap_config.priority), \
         change_freq = COALESCE(\x244, sitemap_config.change_freq) \
         RETURNING id, directory_id, auto_generate, \
         CAST(priority AS DOUBLE PRECISION) as priority, \
         change_freq, last_generated, created_at",
    )
    .bind(directory_id)
    .bind(body.auto_generate)
    .bind(body.priority)
    .bind(&body.change_freq)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(serde_json::json!(result)))
}

/// POST /api/v1/seo/regenerate-sitemap — force regenerate sitemap
pub async fn regenerate_sitemap(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE sitemap_config SET last_generated = NOW() WHERE auto_generate = true",
    )
    .execute(&s.db)
    .await?;

    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sitemap_config WHERE auto_generate = true",
    )
    .fetch_one(&s.db)
    .await?;

    Ok(Json(serde_json::json!({
        "message": "Sitemap regeneration triggered",
        "directories_updated": count,
        "timestamp": Utc::now()
    })))
}

/// GET /api/v1/sitemap.xml — generate and return XML sitemap
pub async fn generate_sitemap(
    State(s): State<AppState>,
) -> impl IntoResponse {
    let base_url = "https://directory.swiftsoftware.net";

    // Collect all directories
    let directories = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, slug FROM directories ORDER BY name",
    )
    .fetch_all(&s.db)
    .await;

    let dirs = match directories {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to fetch directories for sitemap: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                "Error generating sitemap".to_string(),
            );
        }
    };

    // Collect blog posts
    let blog_posts = sqlx::query_as::<_, (String, String)>(
        "SELECT bp.slug, d.slug FROM blog_posts bp \
         JOIN directories d ON d.id = bp.directory_id \
         WHERE (bp.published = true OR bp.published IS NULL) AND bp.slug IS NOT NULL",
    )
    .fetch_all(&s.db)
    .await;

    let posts = match blog_posts {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to fetch blog posts for sitemap: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                "Error generating sitemap".to_string(),
            );
        }
    };

    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    xml.push_str(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#);

    // Homepage
    xml.push_str(&format!(
        "<url><loc>{base}/</loc><priority>1.0</priority></url>",
        base = base_url
    ));

    // Directories
    for (dir_id, slug) in &dirs {
        let cfg = sqlx::query_as::<_, (Option<f64>, Option<String>)>(
            "SELECT CAST(priority AS DOUBLE PRECISION), change_freq FROM sitemap_config WHERE directory_id = \x241",
        )
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await;

        let (priority, change_freq) = match cfg {
            Ok(Some((p, cf))) => (
                p.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "0.9".to_string()),
                cf.unwrap_or_else(|| "weekly".to_string()),
            ),
            _ => ("0.9".to_string(), "weekly".to_string()),
        };

        xml.push_str(&format!(
            "<url><loc>{base}/directories/{slug}</loc><priority>{p}</priority><changefreq>{cf}</changefreq></url>",
            base = base_url,
            slug = slug,
            p = priority,
            cf = change_freq,
        ));
    }

    // Blog posts
    for (post_slug, dir_slug) in &posts {
        xml.push_str(&format!(
            "<url><loc>{base}/directories/{ds}/blog/{ps}</loc><priority>0.6</priority><changefreq>monthly</changefreq></url>",
            base = base_url,
            ds = dir_slug,
            ps = post_slug,
        ));
    }

    xml.push_str("</urlset>");

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/xml")],
        xml,
    )
}

/// GET /api/v1/robots.txt — serve dynamic robots.txt
pub async fn get_robots_txt() -> impl IntoResponse {
    let robots = "User-agent: *\nAllow: /\n\nSitemap: https://directory.swiftsoftware.net/sitemap.xml\n";
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        robots.to_string(),
    )
}
