//! Handlers: SEO Fallback Templates, Schema Config, Google Maps Config, Directory SEO Settings

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── SEO Fallback Templates ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SeoFallbackTemplate {
    pub id: Uuid, pub directory_id: Uuid, pub page_type: String,
    pub title_template: Option<String>, pub description_template: Option<String>,
    pub created_at: Option<DateTime<Utc>>, pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSeoFallbackReq { pub title_template: Option<String>, pub description_template: Option<String> }

pub async fn list_seo_fallbacks(State(s): State<AppState>, Path(dir_id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    Ok(Json(sqlx::query_as::<_, SeoFallbackTemplate>(
        "SELECT * FROM seo_fallback_templates WHERE directory_id=$1 ORDER BY page_type"
    ).bind(dir_id).fetch_all(&s.db).await?))
}

pub async fn upsert_seo_fallback(State(s): State<AppState>, Path((dir_id, pt)): Path<(Uuid, String)>, Json(req): Json<UpsertSeoFallbackReq>) -> ApiResult<impl IntoResponse> {
    let t = sqlx::query_as::<_, SeoFallbackTemplate>(
        "INSERT INTO seo_fallback_templates (directory_id,page_type,title_template,description_template) VALUES($1,$2,$3,$4) ON CONFLICT (directory_id,page_type) DO UPDATE SET title_template=COALESCE($3,seo_fallback_templates.title_template),description_template=COALESCE($4,seo_fallback_templates.description_template),updated_at=NOW() RETURNING *"
    ).bind(dir_id).bind(&pt).bind(&req.title_template).bind(&req.description_template)
    .fetch_one(&s.db).await?;
    Ok(Json(t))
}

// ── Schema Config ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SchemaConfig {
    pub id: Uuid, pub directory_id: Uuid, pub schema_type: String,
    pub enabled: Option<bool>, pub config: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>, pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSchemaConfigReq { pub enabled: Option<bool>, pub config: Option<serde_json::Value> }

pub async fn list_schema_configs(State(s): State<AppState>, Path(dir_id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    Ok(Json(sqlx::query_as::<_, SchemaConfig>(
        "SELECT * FROM schema_config WHERE directory_id=$1 ORDER BY schema_type"
    ).bind(dir_id).fetch_all(&s.db).await?))
}

pub async fn upsert_schema_config(State(s): State<AppState>, Path((dir_id, st)): Path<(Uuid, String)>, Json(req): Json<UpsertSchemaConfigReq>) -> ApiResult<impl IntoResponse> {
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

pub async fn get_dir_seo_settings(State(s): State<AppState>, Path(dir_id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>, Option<bool>, Option<String>, Option<String>, Option<String>, Option<i32>, Option<i32>)>(
        "SELECT page_slug_pattern, google_maps_api_key, internal_linking_enabled, internal_linking_logic, ai_provider, ai_model, ai_word_count_min, ai_word_count_max FROM directories WHERE id=$1"
    ).bind(dir_id).fetch_optional(&s.db).await?.ok_or(AppError::NotFound("Directory".into()))?;
    Ok(Json(DirSeoSettings {
        page_slug_pattern: row.0, google_maps_api_key: row.1,
        internal_linking_enabled: row.2, internal_linking_logic: row.3,
        ai_provider: row.4, ai_model: row.5,
        ai_word_count_min: row.6, ai_word_count_max: row.7,
    }))
}

pub async fn update_dir_seo_settings(State(s): State<AppState>, Path(dir_id): Path<Uuid>, Json(req): Json<UpdateDirSeoSettingsReq>) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE directories SET page_slug_pattern=COALESCE($1,page_slug_pattern), google_maps_api_key=COALESCE($2,google_maps_api_key), internal_linking_enabled=COALESCE($3,internal_linking_enabled), internal_linking_logic=COALESCE($4,internal_linking_logic), ai_provider=COALESCE($5,ai_provider), ai_model=COALESCE($6,ai_model), ai_word_count_min=COALESCE($7,ai_word_count_min), ai_word_count_max=COALESCE($8,ai_word_count_max) WHERE id=$9"
    ).bind(&req.page_slug_pattern).bind(&req.google_maps_api_key).bind(req.internal_linking_enabled)
    .bind(&req.internal_linking_logic).bind(&req.ai_provider).bind(&req.ai_model)
    .bind(req.ai_word_count_min).bind(req.ai_word_count_max).bind(dir_id)
    .execute(&s.db).await?;
    Ok(Json(json!({"ok":true})))
}

// ── Generate Sitemap Index ──

pub async fn generate_sitemap(State(s): State<AppState>, Path(dir_id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT name, page_slug_pattern FROM directories WHERE id=$1"
    ).bind(dir_id).fetch_optional(&s.db).await?.ok_or(AppError::NotFound("Directory".into()))?;
    let site_name = dir.0;

    // Get the directory domain
    let domains: Vec<String> = sqlx::query_scalar(
        "SELECT domain FROM domains WHERE directory_id=$1 AND verified=true"
    ).bind(dir_id).fetch_all(&s.db).await?;
    let base_url = domains.first().map(|d| format!("https://{}", d)).unwrap_or_else(|| format!("https://{}.{}", site_name.to_lowercase().replace(' ', "-"), s.config.base_domain));

    let mut urls = Vec::new();
    urls.push(format!("{}", base_url));
    urls.push(format!("{}/blog", base_url));

    // Add blog posts
    let posts: Vec<String> = sqlx::query_scalar("SELECT slug FROM blog_posts WHERE directory_id=$1 AND published=true")
        .bind(dir_id).fetch_all(&s.db).await?;
    for slug in &posts {
        urls.push(format!("{}/blog/{}", base_url, slug));
    }

    // Add programmatic pages
    let pp: Vec<String> = sqlx::query_scalar("SELECT slug FROM programmatic_pages WHERE directory_id=$1 AND status='published'")
        .bind(dir_id).fetch_all(&s.db).await?;
    for slug in &pp {
        urls.push(format!("{}/{}", base_url, slug));
    }

    // Add categories
    let cats: Vec<String> = sqlx::query_scalar("SELECT slug FROM directory_categories WHERE directory_id=$1")
        .bind(dir_id).fetch_all(&s.db).await?;
    for slug in &cats {
        urls.push(format!("{}/category/{}", base_url, slug));
    }

    Ok(Json(json!({
        "base_url": base_url,
        "urls": urls,
        "count": urls.len()
    })))
}
