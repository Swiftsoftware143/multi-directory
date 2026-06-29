//! Blog post CRUD handlers for Multi-Directory API.

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
}

#[derive(Debug, Deserialize)]
pub struct UpdateBlogPostRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: Option<String>,
    pub published: Option<bool>,
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
