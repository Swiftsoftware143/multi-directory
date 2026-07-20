//! Public blog page rendering — serves HTML blog listing and individual posts
//! using the directory's template.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::models::directory::Directory;
use crate::models::directory::{BlogPost, AuthorProfile};
use crate::template_engine;
use crate::tracking_script;

/// GET /api/v1/directories/:slug/blog — public blog listing page
pub async fn render_blog_list(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let posts = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at,
                focus_keyword, meta_title, meta_description, canonical_url, robots_meta,
                featured_image_url, featured_image_alt, schema_type, author_id,
                service_id, location_id, scheduled_at, template_id, template_data,
                is_master, master_post_id, blog_category, tags,
                feature_image, feature_video, media_json
         FROM blog_posts
         WHERE directory_id = $1 AND (published = true OR (scheduled_at IS NOT NULL AND scheduled_at <= NOW()))
         ORDER BY COALESCE(scheduled_at, created_at) DESC
         LIMIT 50"
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    let posts_html: Vec<String> = posts.iter().map(|post| {
        let title = &post.title;
        let ps = post.slug.as_deref().unwrap_or("post");
        let excerpt = post.excerpt.as_deref().unwrap_or("");
        let date = post.scheduled_at
            .map(|d| d.format("%B %d, %Y").to_string())
            .unwrap_or_else(|| post.created_at.map(|d| d.format("%B %d, %Y").to_string()).unwrap_or_default());
        format!(
            "<article class=\"blog-card\"><a href=\"/api/v1/d/{slug}/blog/{ps}\"><h2>{title}</h2></a><p class=\"post-meta\">{date}</p><p>{excerpt}</p></article>",
            slug = slug,
            ps = ps,
            title = esc(title),
            date = esc(&date),
            excerpt = esc(excerpt),
        )
    }).collect();

    let template_id = directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let ctx = template_engine::build_template_context(
        &dir_val,
        &serde_json::Value::Null,
        &serde_json::Value::Null,
        None,
        None,
    );

    let blog_section = format!(
        "<div class=\"blog-list\"><h1>Blog - {name}</h1>{posts}</div>",
        name = esc(&directory.name),
        posts = posts_html.join("\n"),
    );

    let full_html = engine.render_blog_page(template_id, &ctx, &blog_section)
        .map_err(|e| AppError::Internal(e))?;

    let mut output = crate::tracking_script::inject_tracking_script(&full_html);
    // Apply custom head/body/footer injections
    let dir = &directory;
    if let Some(ref hi) = dir.head_injection {
        if !hi.trim().is_empty() {
            output = output.replace("</head>", &format!("\n{}\n</head>", crate::template_engine::sanitize_html(hi)));
        }
    }
    if let Some(ref bi) = dir.body_injection {
        if !bi.trim().is_empty() {
            output = output.replace("<body", &format!("\n{}\n<body", crate::template_engine::sanitize_html(bi)));
        }
    }
    if let Some(ref fi) = dir.footer_injection {
        if !fi.trim().is_empty() {
            output = output.replace("</body>", &format!("\n{}\n</body>", crate::template_engine::sanitize_html(fi)));
        }
    }
    // Inject survey widget if onboarding_survey is enabled
    if let Some(ref fc) = dir.feature_config {
        if fc.get("onboarding_survey").and_then(|v| v.as_bool()).unwrap_or(false) {
            let survey_tag = "<script src=\"/survey-widget.js\"></script>";
            output = output.replace("</head>", &format!("\n{}\n</head>", survey_tag));
        }
    }
    Ok(axum::response::Html(output))
}

/// GET /api/v1/directories/:slug/blog/:post_slug — public single blog post
pub async fn render_blog_post(
    State(s): State<AppState>,
    Path((slug, post_slug)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let post = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at,
                focus_keyword, meta_title, meta_description, canonical_url, robots_meta,
                featured_image_url, featured_image_alt, schema_type, author_id,
                service_id, location_id, scheduled_at, template_id, template_data,
                is_master, master_post_id, blog_category, tags,
                feature_image, feature_video, media_json
         FROM blog_posts
         WHERE directory_id = $1 AND slug = $2 AND (published = true OR (scheduled_at IS NOT NULL AND scheduled_at <= NOW()))"
    )
    .bind(directory.id)
    .bind(&post_slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;

    let mut byline = String::from("Admin");
    if let Some(aid) = post.author_id {
        if let Ok(author) = sqlx::query_as::<_, AuthorProfile>(
            "SELECT id, directory_id, user_id, name, slug, bio, avatar_url, twitter_url, linkedin_url, website_url, role, is_active, created_at, updated_at FROM author_profiles WHERE id = $1"
        )
        .bind(aid)
        .fetch_optional(&s.db)
        .await
        {
            if let Some(a) = author {
                byline = a.name;
            }
        }
    }

    let template_id = directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let ctx = template_engine::build_template_context(
        &dir_val,
        &serde_json::Value::Null,
        &serde_json::Value::Null,
        None,
        None,
    );

    let content = strip_blockquote(&post.content);

    let date_str = post.scheduled_at
        .map(|d| d.format("%B %d, %Y").to_string())
        .unwrap_or_else(|| post.created_at.map(|d| d.format("%B %d, %Y").to_string()).unwrap_or_default());

    let back_link = format!(
        "<p><a href=\"/api/v1/d/{slug}/blog\">&larr; Back to {name} Blog</a> | <a href=\"/api/v1/directories/{slug}/render\">Back to {name}</a></p>",
        slug = slug,
        name = esc(&directory.name)
    );

    let article_html = format!(
        "<article class=\"blog-post\"><h1>{title}</h1><p class=\"post-meta\">{date} by {byline}</p>{back}{content}</article>",
        title = esc(&post.title),
        date = esc(&date_str),
        byline = esc(&byline),
        back = back_link,
        content = content,
    );

    let full_html = engine.render_blog_page(template_id, &ctx, &article_html)
        .map_err(|e| AppError::Internal(e))?;

    let mut output = crate::tracking_script::inject_tracking_script(&full_html);
    // Apply custom head/body/footer injections
    if let Some(ref hi) = directory.head_injection {
        if !hi.trim().is_empty() {
            output = output.replace("</head>", &format!("\n{}\n</head>", crate::template_engine::sanitize_html(hi)));
        }
    }
    if let Some(ref bi) = directory.body_injection {
        if !bi.trim().is_empty() {
            output = output.replace("<body", &format!("\n{}\n<body", crate::template_engine::sanitize_html(bi)));
        }
    }
    if let Some(ref fi) = directory.footer_injection {
        if !fi.trim().is_empty() {
            output = output.replace("</body>", &format!("\n{}\n</body>", crate::template_engine::sanitize_html(fi)));
        }
    }
    // Inject survey widget if onboarding_survey is enabled in feature_config
    if let Some(ref fc) = directory.feature_config {
        if fc.get("onboarding_survey").and_then(|v| v.as_bool()).unwrap_or(false) {
            let survey_tag = "<script src=\"/survey-widget.js\"></script>";
            output = output.replace("</head>", &format!("\n{}\n</head>", survey_tag));
        }
    }
    Ok(axum::response::Html(output))
}

fn strip_blockquote(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with("<blockquote>") && trimmed.ends_with("</blockquote>") {
        let inner = &trimmed[12..trimmed.len()-13];
        inner.trim().to_string()
    } else {
        content.to_string()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
