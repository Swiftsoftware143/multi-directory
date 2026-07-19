//! Blog post CRUD handlers for Multi-Directory API.

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

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BlogPost {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: String,
    pub directory_id: Uuid,
    pub published: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBlogPostRequest {
    pub title: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: String,
    pub directory_id: Uuid,
    pub published: Option<bool>,
    pub author_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBlogPostRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: Option<String>,
    pub published: Option<bool>,
    pub status: Option<String>,
}

/// GET /api/v1/blog-posts — list all blog posts
pub async fn list_blog_posts(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let posts = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at FROM blog_posts ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(posts))
}

/// GET /api/v1/directories/:slug/blog-posts — list posts for a directory
pub async fn list_directory_blog_posts(
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

    let posts = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at FROM blog_posts WHERE directory_id = \x241 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(posts))
}

/// POST /api/v1/blog-posts — create a blog post
pub async fn create_blog_post(
    State(s): State<AppState>,
    Json(req): Json<CreateBlogPostRequest>,
) -> ApiResult<impl IntoResponse> {
    let slug = req.slug.unwrap_or_else(|| slugify(&req.title));

    let post = sqlx::query_as::<_, BlogPost>(
        "INSERT INTO blog_posts (title, slug, excerpt, content, directory_id, published) VALUES (\x241, \x242, \x243, \x244, \x245, \x246) RETURNING id, title, slug, excerpt, content, directory_id, published, created_at, updated_at "
    )
    .bind(&req.title)
    .bind(&slug)
    .bind(&req.excerpt)
    .bind(&req.content)
    .bind(req.directory_id)
    .bind(req.published.unwrap_or(true))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(post)))
}

/// GET /api/v1/blog-posts/:id — get single blog post
pub async fn get_blog_post(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let post = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at FROM blog_posts WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;

    Ok(Json(post))
}

/// PUT /api/v1/blog-posts/:id — update blog post
pub async fn update_blog_post(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateBlogPostRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at FROM blog_posts WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let slug = req.slug.unwrap_or_else(|| existing.slug.unwrap_or_else(|| slugify(&title)));
    let excerpt = req.excerpt.or(existing.excerpt);
    let content = req.content.unwrap_or(existing.content);
    let published = req.published.unwrap_or(existing.published.unwrap_or(true));

    let post = sqlx::query_as::<_, BlogPost>(
        "UPDATE blog_posts SET title = \x241, slug = \x242, excerpt = \x243, content = \x244, published = \x245, updated_at = NOW() WHERE id = \x246 RETURNING id, title, slug, excerpt, content, directory_id, published, created_at, updated_at "
    )
    .bind(&title)
    .bind(&slug)
    .bind(&excerpt)
    .bind(&content)
    .bind(published)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(post))
}

/// DELETE /api/v1/blog-posts/:id — delete blog post
pub async fn delete_blog_post(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM blog_posts WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Blog post not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}


// ═══════════════════════════════════════════════════════════════════════════
// Blog Automation — Templates, Scheduling, Multi-City Distribution
// ═══════════════════════════════════════════════════════════════════════════

use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BlogTemplate {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category: String,
    pub content_template: String,
    pub merge_fields: Value,
    pub is_global: Option<bool>,
    pub directory_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category: String,
    pub content_template: String,
    pub merge_fields: Option<Value>,
    pub is_global: Option<bool>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub content_template: Option<String>,
    pub merge_fields: Option<Value>,
    pub is_global: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BlogPostExt {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: String,
    pub directory_id: Uuid,
    pub published: Option<bool>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub template_id: Option<Uuid>,
    pub template_data: Option<Value>,
    pub is_master: Option<bool>,
    pub master_post_id: Option<Uuid>,
    pub blog_category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta_description: Option<String>,
    pub feature_image: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBlogPostExtRequest {
    pub title: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: String,
    pub directory_id: Uuid,
    pub published: Option<bool>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub template_id: Option<Uuid>,
    pub template_data: Option<Value>,
    pub blog_category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta_description: Option<String>,
    pub feature_image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBlogPostExtRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: Option<String>,
    pub published: Option<bool>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub template_id: Option<Uuid>,
    pub template_data: Option<Value>,
    pub blog_category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta_description: Option<String>,
    pub feature_image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DistributeRequest {
    pub title: String,
    pub template_id: Uuid,
    pub template_data: Value,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub meta_description: Option<String>,
    pub published: Option<bool>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub target_directory_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize)]
pub struct DistributeResult {
    pub total: usize,
    pub created: usize,
    pub skipped: Vec<String>,
}

/// GET /api/v1/blog-templates — list all templates
pub async fn list_templates(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let templates = sqlx::query_as::<_, BlogTemplate>(
        "SELECT id, name, slug, description, category, content_template, merge_fields, is_global, directory_id, created_at, updated_at FROM blog_templates ORDER BY category, name"
    ).fetch_all(&s.db).await?;
    Ok(Json(templates))
}

/// GET /api/v1/blog-templates/:id
pub async fn get_template(State(s): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, BlogTemplate>(
        "SELECT id, name, slug, description, category, content_template, merge_fields, is_global, directory_id, created_at, updated_at FROM blog_templates WHERE id = $1"
    ).bind(id).fetch_optional(&s.db).await?
    .ok_or_else(|| AppError::NotFound("Template not found".into()))?;
    Ok(Json(template))
}

/// POST /api/v1/blog-templates
pub async fn create_template(State(s): State<AppState>, Json(req): Json<CreateTemplateRequest>) -> ApiResult<impl IntoResponse> {
    let mf = req.merge_fields.unwrap_or_else(|| serde_json::json!([]));
    let template = sqlx::query_as::<_, BlogTemplate>(
        "INSERT INTO blog_templates (name, slug, description, category, content_template, merge_fields, is_global, directory_id) VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8) RETURNING id, name, slug, description, category, content_template, merge_fields, is_global, directory_id, created_at, updated_at"
    ).bind(&req.name).bind(&req.slug).bind(&req.description)
    .bind(&req.category).bind(&req.content_template).bind(&mf)
    .bind(req.is_global.unwrap_or(true)).bind(req.directory_id)
    .fetch_one(&s.db).await?;
    Ok((StatusCode::CREATED, Json(template)))
}

/// PUT /api/v1/blog-templates/:id
pub async fn update_template(State(s): State<AppState>, Path(id): Path<Uuid>, Json(req): Json<UpdateTemplateRequest>) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, BlogTemplate>(
        "SELECT id, name, slug, description, category, content_template, merge_fields, is_global, directory_id, created_at, updated_at FROM blog_templates WHERE id = $1"
    ).bind(id).fetch_optional(&s.db).await?
    .ok_or_else(|| AppError::NotFound("Template not found".into()))?;
    let template = sqlx::query_as::<_, BlogTemplate>(
        "UPDATE blog_templates SET name=$1, slug=$2, description=$3, category=$4, content_template=$5, merge_fields=$6::jsonb, is_global=$7, updated_at=NOW() WHERE id=$8 RETURNING id, name, slug, description, category, content_template, merge_fields, is_global, directory_id, created_at, updated_at"
    ).bind(req.name.unwrap_or(existing.name)).bind(req.slug.unwrap_or(existing.slug))
    .bind(req.description.or(existing.description)).bind(req.category.unwrap_or(existing.category))
    .bind(req.content_template.unwrap_or(existing.content_template))
    .bind(req.merge_fields.unwrap_or(existing.merge_fields))
    .bind(req.is_global.unwrap_or(existing.is_global.unwrap_or(true))).bind(id)
    .fetch_one(&s.db).await?;
    Ok(Json(template))
}

/// DELETE /api/v1/blog-templates/:id
pub async fn delete_template(State(s): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    let r = sqlx::query("DELETE FROM blog_templates WHERE id = $1").bind(id).execute(&s.db).await?;
    if r.rows_affected() == 0 { return Err(AppError::NotFound("Template not found".into())); }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/blog-posts/ext — create blog post with extended fields
pub async fn create_blog_post_ext(State(s): State<AppState>, Json(req): Json<CreateBlogPostExtRequest>) -> ApiResult<impl IntoResponse> {
    let slug = req.slug.unwrap_or_else(|| slugify(&req.title));
    let tags = req.tags.unwrap_or_default();
    let post = sqlx::query_as::<_, BlogPostExt>(
        "INSERT INTO blog_posts (title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, blog_category, tags, meta_description, feature_image) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9::jsonb,$10,$11::text[],$12,$13) RETURNING id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at"
    ).bind(&req.title).bind(&slug).bind(&req.excerpt).bind(&req.content)
    .bind(req.directory_id).bind(req.published.unwrap_or(true)).bind(req.scheduled_at)
    .bind(req.template_id).bind(req.template_data.unwrap_or_else(|| serde_json::json!({})))
    .bind(req.blog_category.unwrap_or_else(|| "general".to_string())).bind(&tags)
    .bind(&req.meta_description).bind(&req.feature_image).fetch_one(&s.db).await?;
    Ok((StatusCode::CREATED, Json(post)))
}

/// PUT /api/v1/blog-posts/:id/ext — update blog post with extended fields
pub async fn update_blog_post_ext(State(s): State<AppState>, Path(id): Path<Uuid>, Json(req): Json<UpdateBlogPostExtRequest>) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, BlogPostExt>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at FROM blog_posts WHERE id = $1"
    ).bind(id).fetch_optional(&s.db).await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;
    let tags = req.tags.unwrap_or_else(|| existing.tags.unwrap_or_default());
    let post = sqlx::query_as::<_, BlogPostExt>(
        "UPDATE blog_posts SET title=$1, slug=$2, excerpt=$3, content=$4, published=$5, scheduled_at=$6, template_id=$7, template_data=$8::jsonb, blog_category=$9, tags=$10::text[], meta_description=$11, feature_image=$12, updated_at=NOW() WHERE id=$13 RETURNING id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at"
    ).bind(req.title.clone().unwrap_or(existing.title.clone()))
    .bind(req.slug.unwrap_or_else(|| existing.slug.clone().unwrap_or_else(|| slugify(&existing.title))))
    .bind(req.excerpt.or(existing.excerpt)).bind(req.content.unwrap_or(existing.content))
    .bind(req.published.unwrap_or(existing.published.unwrap_or(true)))
    .bind(req.scheduled_at.or(existing.scheduled_at)).bind(req.template_id.or(existing.template_id))
    .bind(req.template_data.clone().or(existing.template_data.clone()))
    .bind(req.blog_category.unwrap_or_else(|| existing.blog_category.unwrap_or_else(|| "general".to_string())))
    .bind(&tags).bind(&req.meta_description.or(existing.meta_description))
    .bind(&req.feature_image.or(existing.feature_image)).bind(id).fetch_one(&s.db).await?;
    Ok(Json(post))
}

/// GET /api/v1/blog-posts/ext — list all blog posts with extended fields
pub async fn list_blog_posts_ext(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let posts = sqlx::query_as::<_, BlogPostExt>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at FROM blog_posts ORDER BY created_at DESC"
    ).fetch_all(&s.db).await?;
    Ok(Json(posts))
}

/// GET /api/v1/directories/:slug/blog-posts/ext — list posts for directory (extended)
pub async fn list_directory_blog_posts_ext(State(s): State<AppState>, Path(slug): Path<String>) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1").bind(&slug)
        .fetch_optional(&s.db).await?.ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let posts = sqlx::query_as::<_, BlogPostExt>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at FROM blog_posts WHERE directory_id = $1 ORDER BY created_at DESC"
    ).bind(dir.0).fetch_all(&s.db).await?;
    Ok(Json(posts))
}

/// GET /api/v1/blog-posts/scheduled — list scheduled posts
pub async fn list_scheduled_posts(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let posts = sqlx::query_as::<_, BlogPostExt>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at FROM blog_posts WHERE scheduled_at IS NOT NULL AND scheduled_at > NOW() AND published = false ORDER BY scheduled_at ASC"
    ).fetch_all(&s.db).await?;
    Ok(Json(posts))
}

/// POST /api/v1/blog-posts/:id/publish — publish/unpublish
pub async fn publish_blog_post_handler(State(s): State<AppState>, Path(id): Path<Uuid>, Json(req): Json<PublishRequest>) -> ApiResult<impl IntoResponse> {
    let post = if req.publish_now.unwrap_or(true) {
        sqlx::query_as::<_, BlogPostExt>(
            "UPDATE blog_posts SET published = true, scheduled_at = NULL, updated_at = NOW() WHERE id = $1 RETURNING id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at"
        ).bind(id).fetch_optional(&s.db).await?
    } else {
        sqlx::query_as::<_, BlogPostExt>(
            "UPDATE blog_posts SET published = false, updated_at = NOW() WHERE id = $1 RETURNING id, title, slug, excerpt, content, directory_id, published, scheduled_at, template_id, template_data, is_master, master_post_id, blog_category, tags, meta_description, feature_image, created_at, updated_at"
        ).bind(id).fetch_optional(&s.db).await?
    };
    match post {
        Some(p) => Ok(Json(p)),
        None => Err(AppError::NotFound("Blog post not found".into())),
    }
}

#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    pub publish_now: Option<bool>,
}

/// POST /api/v1/blog/distribute — multi-city distribution
pub async fn distribute_blog_post(State(s): State<AppState>, Json(req): Json<DistributeRequest>) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, BlogTemplate>(
        "SELECT id, name, slug, description, category, content_template, merge_fields, is_global, directory_id, created_at, updated_at FROM blog_templates WHERE id = $1"
    ).bind(req.template_id).fetch_optional(&s.db).await?
    .ok_or_else(|| AppError::NotFound("Template not found".into()))?;

    let directories = if let Some(ref ids) = req.target_directory_ids {
        sqlx::query_as::<_, (Uuid, String, String)>("SELECT id, name, COALESCE(location, '') FROM directories WHERE id = ANY($1) ORDER BY name")
            .bind(ids).fetch_all(&s.db).await?
    } else {
        sqlx::query_as::<_, (Uuid, String, String)>("SELECT id, name, COALESCE(location, '') FROM directories ORDER BY name")
            .fetch_all(&s.db).await?
    };

    if directories.is_empty() {
        return Err(AppError::BadRequest("No target directories found".into()));
    }

    let mut result = DistributeResult { total: directories.len(), created: 0, skipped: Vec::new() };
    let category = req.category.unwrap_or_else(|| "general".to_string());
    let tags = req.tags.unwrap_or_default();
    let published = req.published.unwrap_or(false);
    let meta_desc = &req.meta_description;

    for (dir_id, dir_name, dir_location) in &directories {
        let mut data = req.template_data.clone();
        if let Some(obj) = data.as_object_mut() {
            if !obj.contains_key("city") || obj["city"].as_str().map_or(true, |s| s.is_empty()) {
                obj.insert("city".to_string(), Value::String(dir_location.clone()));
            }
            if !obj.contains_key("directory_name") || obj["directory_name"].as_str().map_or(true, |s| s.is_empty()) {
                obj.insert("directory_name".to_string(), Value::String(dir_name.clone()));
            }
        }
        let content = render_template_str(&template.content_template, &data);
        let title = render_template_str(&req.title, &data);
        let slug = slugify(&title);

        let existing = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM blog_posts WHERE directory_id = $1 AND slug = $2")
            .bind(dir_id).bind(&slug).fetch_optional(&s.db).await?;
        if existing.is_some() {
            result.skipped.push(format!("{} (duplicate slug)", dir_name));
            continue;
        }

        sqlx::query(
            "INSERT INTO blog_posts (title, slug, content, directory_id, published, scheduled_at, template_id, template_data, blog_category, tags, meta_description) VALUES ($1,$2,$3,$4,$5,$6,$7,$8::jsonb,$9,$10::text[],$11)"
        ).bind(&title).bind(&slug).bind(&content).bind(dir_id)
        .bind(published).bind(req.scheduled_at).bind(req.template_id)
        .bind(&data).bind(&category).bind(&tags).bind(meta_desc)
        .execute(&s.db).await?;
        result.created += 1;
    }
    Ok((StatusCode::CREATED, Json(result)))
}

/// POST /api/v1/blog/process-scheduled — publish due scheduled posts
pub async fn process_scheduled_posts_handler(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let r = sqlx::query("UPDATE blog_posts SET published = true, scheduled_at = NULL, updated_at = NOW() WHERE scheduled_at IS NOT NULL AND scheduled_at <= NOW() AND published = false")
        .execute(&s.db).await?;
    Ok(Json(serde_json::json!({"published": r.rows_affected()})))
}

fn render_template_str(template: &str, data: &Value) -> String {
    let mut result = template.to_string();
    if let Some(obj) = data.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{{}}}", key);
            result = result.replace(&placeholder, value.as_str().unwrap_or(""));
        }
    }
    result
}

/// GET /api/v1/community/posts — list community posts
pub async fn list_community_posts(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let posts = sqlx::query_as::<_, (Uuid, String, String, Option<String>, String, Option<chrono::DateTime<chrono::Utc>>)>(
        r#"SELECT id, title, slug, excerpt, post_type, created_at
           FROM blog_posts WHERE post_type = 'community' AND status = 'published'
           ORDER BY created_at DESC LIMIT 50"#
    )
    .fetch_all(&s.db)
    .await?;

    let result: Vec<serde_json::Value> = posts.into_iter().map(|p| json!({
        "id": p.0, "title": p.1, "slug": p.2, "excerpt": p.3, "type": p.4, "created_at": p.5
    })).collect();

    Ok(Json(json!({"posts": result})))
}

/// POST /api/v1/community/posts — create a community post (business owner or visitor)
pub async fn create_community_post(
    State(s): State<AppState>,
    Json(req): Json<CreateBlogPostRequest>,
) -> ApiResult<impl IntoResponse> {
    let id = Uuid::new_v4();
    let slug = req.title.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != ' ', "-")
        .chars().take(100).collect::<String>();

    sqlx::query(
        "INSERT INTO blog_posts (id, title, slug, content, excerpt, directory_id, post_type, status, author_name, published)
         VALUES ($1, $2, $3, $4, $5, $6, 'community', 'pending_review', $7, false)"
    )
    .bind(id)
    .bind(&req.title)
    .bind(&slug)
    .bind(&req.content)
    .bind(&req.excerpt)
    .bind(req.directory_id)
    .bind(&req.author_name)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"id": id, "slug": slug, "status": "pending_review"})))
}

/// PUT /api/v1/community/posts/:id — update a community post
pub async fn update_community_post(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateBlogPostRequest>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE blog_posts SET title=COALESCE($1,title), content=COALESCE($2,content), excerpt=COALESCE($3,excerpt),
         status=COALESCE($4,status), updated_at=NOW() WHERE id=$5 AND post_type='community'"
    )
    .bind(&req.title)
    .bind(&req.content)
    .bind(&req.excerpt)
    .bind(&req.status)
    .bind(id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "updated"})))
}
