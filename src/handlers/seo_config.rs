//! Handlers: SEO Fallback Templates, Schema Config, Google Maps Config, Directory SEO Settings

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::tenant_scope::assert_directory_admin;
use crate::AppState;

// ── SEO Fallback Templates ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SeoFallbackTemplate {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub page_type: String,
    pub title_template: Option<String>,
    pub description_template: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSeoFallbackReq {
    pub title_template: Option<String>,
    pub description_template: Option<String>,
}

pub async fn list_seo_fallbacks(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, dir_id).await?;
    Ok(Json(
        sqlx::query_as::<_, SeoFallbackTemplate>(
            "SELECT * FROM seo_fallback_templates WHERE directory_id=$1 ORDER BY page_type",
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?,
    ))
}

pub async fn upsert_seo_fallback(
    State(s): State<AppState>,
    Path((dir_id, pt)): Path<(Uuid, String)>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<UpsertSeoFallbackReq>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, dir_id).await?;
    // B119: the same guard the legal-page editor has — an unknown merge field is refused on
    // save with a plain-English message, so a raw `{brace}` can never reach a visitor.
    let unknown = crate::merge_fields::unknown_fields(&[
        req.title_template.as_deref().unwrap_or(""),
        req.description_template.as_deref().unwrap_or(""),
    ]);
    if !unknown.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Unknown merge field(s): {}. A title or description pattern may only use fields \
             the platform can fill: {}. Remove the extra braces — an unknown field would show \
             blank to visitors.",
            unknown.join(", "),
            crate::merge_fields::names().join(", ")
        )));
    }
    let t = sqlx::query_as::<_, SeoFallbackTemplate>(
        "INSERT INTO seo_fallback_templates (directory_id,page_type,title_template,description_template) VALUES($1,$2,$3,$4) ON CONFLICT (directory_id,page_type) DO UPDATE SET title_template=COALESCE($3,seo_fallback_templates.title_template),description_template=COALESCE($4,seo_fallback_templates.description_template),updated_at=NOW() RETURNING *"
    ).bind(dir_id).bind(&pt).bind(&req.title_template).bind(&req.description_template)
    .fetch_one(&s.db).await?;
    Ok(Json(t))
}

// ── Schema Config ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SchemaConfig {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub schema_type: String,
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSchemaConfigReq {
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

pub async fn list_schema_configs(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, dir_id).await?;
    Ok(Json(
        sqlx::query_as::<_, SchemaConfig>(
            "SELECT * FROM schema_config WHERE directory_id=$1 ORDER BY schema_type",
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?,
    ))
}

pub async fn upsert_schema_config(
    State(s): State<AppState>,
    Path((dir_id, st)): Path<(Uuid, String)>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<UpsertSchemaConfigReq>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, dir_id).await?;
    let cfg = sqlx::query_as::<_, SchemaConfig>(
        "INSERT INTO schema_config (directory_id,schema_type,enabled,config) VALUES($1,$2,$3,$4::jsonb) ON CONFLICT (directory_id,schema_type) DO UPDATE SET enabled=COALESCE($3,schema_config.enabled),config=CASE WHEN $4::jsonb='{}'::jsonb THEN schema_config.config ELSE COALESCE($4::jsonb,schema_config.config) END,updated_at=NOW() RETURNING *"
    ).bind(dir_id).bind(&st).bind(req.enabled).bind(&req.config)
    .fetch_one(&s.db).await?;
    Ok(Json(cfg))
}

// ── Directory SEO Settings ──

#[derive(Debug, Serialize, Deserialize)]
pub struct DirSeoSettings {
    pub page_slug_pattern: Option<String>,
    pub google_maps_api_key: Option<String>,
    pub internal_linking_enabled: Option<bool>,
    pub internal_linking_logic: Option<String>,
    pub ai_provider: Option<String>,
    pub ai_model: Option<String>,
    pub ai_word_count_min: Option<i32>,
    pub ai_word_count_max: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDirSeoSettingsReq {
    pub page_slug_pattern: Option<String>,
    pub google_maps_api_key: Option<String>,
    pub internal_linking_enabled: Option<bool>,
    pub internal_linking_logic: Option<String>,
    pub ai_provider: Option<String>,
    pub ai_model: Option<String>,
    pub ai_word_count_min: Option<i32>,
    pub ai_word_count_max: Option<i32>,
}

pub async fn get_dir_seo_settings(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>, Option<bool>, Option<String>, Option<String>, Option<String>, Option<i32>, Option<i32>)>(
        "SELECT page_slug_pattern, google_maps_api_key, internal_linking_enabled, internal_linking_logic, ai_provider, ai_model, ai_word_count_min, ai_word_count_max FROM directories WHERE id=$1"
    ).bind(dir_id).fetch_optional(&s.db).await?.ok_or(AppError::NotFound("Directory".into()))?;
    Ok(Json(DirSeoSettings {
        page_slug_pattern: row.0,
        google_maps_api_key: row.1,
        internal_linking_enabled: row.2,
        internal_linking_logic: row.3,
        ai_provider: row.4,
        ai_model: row.5,
        ai_word_count_min: row.6,
        ai_word_count_max: row.7,
    }))
}

pub async fn update_dir_seo_settings(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Json(req): Json<UpdateDirSeoSettingsReq>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE directories SET page_slug_pattern=COALESCE($1,page_slug_pattern), google_maps_api_key=COALESCE($2,google_maps_api_key), internal_linking_enabled=COALESCE($3,internal_linking_enabled), internal_linking_logic=COALESCE($4,internal_linking_logic), ai_provider=COALESCE($5,ai_provider), ai_model=COALESCE($6,ai_model), ai_word_count_min=COALESCE($7,ai_word_count_min), ai_word_count_max=COALESCE($8,ai_word_count_max) WHERE id=$9"
    ).bind(&req.page_slug_pattern).bind(&req.google_maps_api_key).bind(req.internal_linking_enabled)
    .bind(&req.internal_linking_logic).bind(&req.ai_provider).bind(&req.ai_model)
    .bind(req.ai_word_count_min).bind(req.ai_word_count_max).bind(dir_id)
    .execute(&s.db).await?;
    Ok(Json(json!({"ok":true})))
}

// ── Generate Sitemap Index ──

/// True when `host` is a per-directory subdomain of `base_domain`
/// (`<slug>.<base_domain>`). Those hosts have no DNS record, so they must never
/// be named as the platform origin in a sitemap (kanban t_543d51d8).
pub(crate) fn is_directory_subdomain(host: &str, base_domain: &str) -> bool {
    let base = base_domain.trim().trim_matches('.');
    if base.is_empty() {
        return false;
    }
    let h = host.trim().split(':').next().unwrap_or(host);
    let h = h.trim_start_matches("www.");
    match h.strip_suffix(&format!(".{}", base)) {
        Some(label) => !label.is_empty() && !label.contains('.'),
        None => false,
    }
}

pub async fn generate_sitemap(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (String, Option<String>, String)>(
        "SELECT name, page_slug_pattern, slug FROM directories WHERE id=$1",
    )
    .bind(dir_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Directory".into()))?;
    let site_name = dir.0;
    let dir_slug = dir.2;

    // Get the directory domain
    let domains: Vec<String> = sqlx::query_scalar(
        "SELECT domain FROM domain_mappings WHERE directory_id=$1 AND status='active'",
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;
    // t_543d51d8: the no-mapped-domain fallback used to be `<name>.<base_domain>`,
    // a host with no DNS record — every URL in this sitemap was unreachable and the
    // programmatic pages it advertises had no route. Directories are served as
    // subfolders on the platform host (`<origin>/<dir-slug>/...`), the same scheme
    // the public /sitemap.xml uses, so name that. A directory that owns a mapped
    // domain still owns that domain's root.
    let base_url = match domains.first() {
        Some(d) => format!("https://{}", d),
        None => {
            let (host, proto) = crate::handlers::subfolder::host_proto(&headers);
            let host = host.filter(|h| !is_directory_subdomain(h, &s.config.base_domain));
            let host = host.or_else(|| {
                let b = s.config.base_domain.trim().to_string();
                if b.is_empty() {
                    None
                } else {
                    Some(b)
                }
            });
            format!(
                "{}/{}",
                crate::handlers::subfolder::origin(host.as_deref(), &proto, &s.config.base_domain),
                dir_slug
            )
        }
    };

    let mut urls = Vec::new();
    urls.push(format!("{}", base_url));
    urls.push(format!("{}/blog", base_url));

    // Add blog posts
    let posts: Vec<String> =
        // t_959ee844: slug is NULLABLE and T is String -> one NULL slug 500d the whole sitemap.
        // A post with no slug has no URL, so it is skipped (no migration, no backfill).
        sqlx::query_scalar(
            "SELECT slug FROM blog_posts WHERE directory_id=$1 AND published=true AND slug IS NOT NULL",
        )
            .bind(dir_id)
            .fetch_all(&s.db)
            .await?;
    for slug in &posts {
        urls.push(format!("{}/blog/{}", base_url, slug));
    }

    // Add programmatic pages
    let pp: Vec<String> = sqlx::query_scalar(
        "SELECT slug FROM programmatic_pages WHERE directory_id=$1 AND status='published'",
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;
    for slug in &pp {
        urls.push(format!("{}/{}", base_url, slug));
    }

    // Add categories
    let cats: Vec<String> =
        sqlx::query_scalar("SELECT slug FROM directory_categories WHERE directory_id=$1")
            .bind(dir_id)
            .fetch_all(&s.db)
            .await?;
    for slug in &cats {
        urls.push(format!("{}/category/{}", base_url, slug));
    }

    Ok(Json(json!({
        "base_url": base_url,
        "urls": urls,
        "count": urls.len()
    })))
}
