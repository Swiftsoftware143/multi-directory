//! Handlers: Business SEO Articles
//!
//! Manages SEO-optimized articles for businesses in a directory.
//! Supports paid recurring articles and free directory-owner articles.
//! Each article targets a specific keyword and links back to the business/directory.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc, Duration};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Models ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BusinessArticle {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub business_id: Option<Uuid>,
    pub title: String,
    pub slug: String,
    pub keyword: String,
    pub meta_description: Option<String>,
    pub content: Option<String>,
    pub status: Option<String>,
    pub impressions: Option<i32>,
    pub clicks: Option<i32>,
    pub is_owner_article: Option<bool>,
    pub subscription_active: Option<bool>,
    pub subscription_expires_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateArticleReq {
    pub business_id: Option<Uuid>,
    pub keyword: String,
    pub city: String,
    pub service: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateArticleReq {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub meta_description: Option<String>,
    pub content: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListArticlesParams {
    pub status: Option<String>,
    pub business_id: Option<Uuid>,
    pub is_owner_article: Option<bool>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Helpers ──

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != ' ', "-")
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

fn htmlesc(s: &str) -> String {
    s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&#39;")
}

fn generate_article_content(
    keyword: &str,
    service: &str,
    city: &str,
    title: &str,
    dir_name: &str,
    dir_slug: &str,
    business_name: Option<&str>,
    business_slug: Option<&str>,
) -> String {
    let business_link = business_name.zip(business_slug)
        .map(|(name, slug)| format!(
            r#"<a href="/{}/{}">{}</a>"#,
            htmlesc(dir_slug),
            htmlesc(slug),
            htmlesc(name)
        ))
        .unwrap_or_else(|| format!("{} in {}", htmlesc(service), htmlesc(city)));

    let dir_link = format!(
        r#"<a href="/{}">{}</a>"#,
        htmlesc(dir_slug),
        htmlesc(dir_name)
    );

    format!(
        r#"<p>When it comes to {keyword} in {city}, finding a reliable provider can make all the difference. Whether you are a homeowner or a business owner, knowing what to look for and where to start can save you time, money, and frustration.</p>

<h2>Why {service_title} Matter in {city}</h2>
<p>{city} is a growing community with diverse needs when it comes to {service} services. From routine maintenance to emergency repairs, having access to trusted professionals is essential. The demand for quality {service} in {city} continues to rise, and finding the right partner is key to ensuring your projects are completed on time and within budget.</p>

<h2>What to Look for in {service_title}</h2>
<p>When searching for {keyword} in {city}, consider factors such as experience, licensing, insurance coverage, and customer reviews. A reputable provider should have a proven track record and be willing to provide references. Additionally, look for transparent pricing and clear communication from the start.</p>

<h2>Local Expertise Matters</h2>
<p>Choosing a {service} provider who knows {city} and its specific requirements can make a significant difference. Local professionals understand the regional regulations, climate considerations, and community needs that can affect your project. This local knowledge often translates into better service and more efficient solutions.</p>

<h2>Find Top {service_title} in {city}</h2>
<p>If you are looking for {keyword} in {city}, start your search with our directory. We connect you with vetted {service} providers who serve the {city} area. Browse listings, compare services, and read reviews from real customers to make an informed decision.</p>

<p>Visit {business_link} in our comprehensive {dir_link} to learn more about available {service} services in {city}. Check business hours, read customer feedback, and request a quote today.</p>"#,
        keyword = htmlesc(keyword),
        city = htmlesc(city),
        service = htmlesc(service),
        service_title = title,
        business_link = business_link,
        dir_link = dir_link,
    )
}

fn generate_meta_description(keyword: &str, service: &str, city: &str) -> String {
    format!(
        "Looking for {} in {}? Learn what to look for, how to choose the right {}, and browse top-rated providers in the {} area.",
        keyword, city, service, city
    )
}

// ── Endpoints ──

/// POST /directories/:id/business-articles/generate
///
/// Generates an SEO-optimized article for a keyword.
/// If business_id is provided, the article links to that business.
/// If business_id is None, it is a directory-owner (free SEO) article.
pub async fn generate_article(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Json(req): Json<GenerateArticleReq>,
) -> ApiResult<impl IntoResponse> {
    // Validate directory
    let dir_info = sqlx::query_as::<_, (String, String)>(
        "SELECT name, slug FROM directories WHERE id=$1"
    )
    .bind(dir_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Directory not found".into()))?;

    let (dir_name, dir_slug) = dir_info;

    // Get business info if provided
    let (business_name, business_slug) = if let Some(biz_id) = req.business_id {
        let biz = sqlx::query_as::<_, (String, String)>(
            "SELECT name, slug FROM businesses WHERE id=$1 AND directory_id=$2"
        )
        .bind(biz_id)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound("Business not found in this directory".into()))?;
        (Some(biz.0), Some(biz.1))
    } else {
        (None, None)
    };

    // Generate article data
    let title = format!(
        "{} in {} – A Complete Guide",
        capitalize_first(&req.keyword),
        &req.city
    );

    let base_slug = slugify(&format!("{}-in-{}", &req.keyword, &req.city));
    let slug = format!("{}-guide", base_slug);

    let meta_description = generate_meta_description(&req.keyword, &req.service, &req.city);
    let content = generate_article_content(
        &req.keyword,
        &req.service,
        &req.city,
        &capitalize_first(&req.service),
        &dir_name,
        &dir_slug,
        business_name.as_deref(),
        business_slug.as_deref(),
    );

    // Check for duplicate slug
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM business_articles WHERE directory_id=$1 AND slug=$2"
    )
    .bind(dir_id)
    .bind(&slug)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let final_slug = if existing > 0 {
        let ts = Utc::now().timestamp();
        format!("{}-{}", &slug, ts)
    } else {
        slug
    };

    let article = sqlx::query_as::<_, BusinessArticle>(
        "INSERT INTO business_articles \
         (directory_id, business_id, title, slug, keyword, meta_description, content, status, \
          is_owner_article, subscription_active) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'draft', $8, $9) \
         RETURNING *"
    )
    .bind(dir_id)
    .bind(req.business_id)
    .bind(&title)
    .bind(&final_slug)
    .bind(&req.keyword)
    .bind(&meta_description)
    .bind(&content)
    .bind(req.business_id.is_none())  // is_owner_article = true if no business_id
    .bind(req.business_id.is_some())  // subscription_active = true if paid (has business)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(article))))
}

/// GET /directories/:id/business-articles
///
/// List articles for a directory with optional filters and pagination.
pub async fn list_articles(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Query(params): Query<ListArticlesParams>,
) -> ApiResult<impl IntoResponse> {
    let limit = params.limit.unwrap_or(50).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    use sqlx::Row;

    let mut conditions = vec!["ba.directory_id=$1".to_string()];
    let mut idx = 2;

    if let Some(ref status) = params.status {
        conditions.push(format!("ba.status=${}", idx));
        idx += 1;
    }
    if let Some(ref biz_id) = params.business_id {
        conditions.push(format!("ba.business_id=${}", idx));
        idx += 1;
    }
    if let Some(ref is_owner) = params.is_owner_article {
        conditions.push(format!("ba.is_owner_article=${}", idx));
        idx += 1;
    }
    if let Some(ref search) = params.search {
        conditions.push(format!(
            "(ba.title ILIKE ${} OR ba.keyword ILIKE ${} OR ba.slug ILIKE ${})",
            idx, idx + 1, idx + 2
        ));
        idx += 3;
    }

    let where_clause = conditions.join(" AND ");

    let sql = format!(
        "SELECT ba.*, b.name as business_name \
         FROM business_articles ba \
         LEFT JOIN businesses b ON b.id = ba.business_id \
         WHERE {} \
         ORDER BY ba.updated_at DESC \
         LIMIT ${} OFFSET ${}",
        where_clause, idx, idx + 1
    );

    let status_filter = params.status.clone();
    let biz_id_filter = params.business_id;
    let is_owner_filter = params.is_owner_article;
    let search_filter = params.search.clone();

    let mut query = sqlx::query(&sql).bind(dir_id);

    if let Some(ref status) = status_filter {
        query = query.bind(status.clone());
    }
    if let Some(ref biz_id) = biz_id_filter {
        query = query.bind(biz_id);
    }
    if let Some(ref is_owner) = is_owner_filter {
        query = query.bind(is_owner);
    }
    if let Some(ref search) = search_filter {
        let s = format!("%{}%", search);
        query = query.bind(s.clone()).bind(s.clone()).bind(s);
    }

    query = query.bind(limit).bind(offset);

    let rows = query.fetch_all(&s.db).await?;

    let mut results: Vec<serde_json::Value> = Vec::new();
    for row in &rows {
        results.push(json!({
            "id": row.try_get::<Uuid,_>("id").unwrap_or_default(),
            "directory_id": row.try_get::<Uuid,_>("directory_id").unwrap_or_default(),
            "business_id": row.try_get::<Option<Uuid>,_>("business_id").ok().flatten(),
            "business_name": row.try_get::<Option<String>,_>("business_name").ok().flatten(),
            "title": row.try_get::<String,_>("title").unwrap_or_default(),
            "slug": row.try_get::<String,_>("slug").unwrap_or_default(),
            "keyword": row.try_get::<String,_>("keyword").unwrap_or_default(),
            "meta_description": row.try_get::<Option<String>,_>("meta_description").ok().flatten(),
            "content": row.try_get::<Option<String>,_>("content").ok().flatten(),
            "status": row.try_get::<Option<String>,_>("status").ok().flatten(),
            "impressions": row.try_get::<Option<i32>,_>("impressions").ok().flatten().unwrap_or(0),
            "clicks": row.try_get::<Option<i32>,_>("clicks").ok().flatten().unwrap_or(0),
            "is_owner_article": row.try_get::<Option<bool>,_>("is_owner_article").ok().flatten().unwrap_or(false),
            "subscription_active": row.try_get::<Option<bool>,_>("subscription_active").ok().flatten().unwrap_or(false),
            "subscription_expires_at": row.try_get::<Option<DateTime<Utc>>,_>("subscription_expires_at").ok().flatten(),
            "created_at": row.try_get::<Option<DateTime<Utc>>,_>("created_at").ok().flatten(),
            "updated_at": row.try_get::<Option<DateTime<Utc>>,_>("updated_at").ok().flatten(),
        }));
    }

    // Count total (without pagination)
    let count_sql = format!(
        "SELECT COUNT(*) FROM business_articles ba WHERE {}",
        conditions.join(" AND ")
    );

    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql).bind(dir_id);

    if let Some(ref status) = status_filter {
        count_query = count_query.bind(status.clone());
    }
    if let Some(ref biz_id) = biz_id_filter {
        count_query = count_query.bind(biz_id);
    }
    if let Some(ref is_owner) = is_owner_filter {
        count_query = count_query.bind(is_owner);
    }
    if let Some(ref search) = search_filter {
        let s = format!("%{}%", search);
        count_query = count_query.bind(s.clone()).bind(s.clone()).bind(s);
    }

    let total = count_query.fetch_one(&s.db).await.unwrap_or(0);

    Ok(Json(json!({
        "data": results,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

/// PUT /business-articles/:id
///
/// Update an article's content, status, title, slug, or meta_description.
pub async fn update_article(
    State(s): State<AppState>,
    Path(article_id): Path<Uuid>,
    Json(req): Json<UpdateArticleReq>,
) -> ApiResult<impl IntoResponse> {
    // Verify article exists
    let existing = sqlx::query_as::<_, BusinessArticle>(
        "SELECT * FROM business_articles WHERE id=$1"
    )
    .bind(article_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Article not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let slug = req.slug.unwrap_or(existing.slug);
    let meta_description = req.meta_description.or(existing.meta_description);
    let content = req.content.or(existing.content);
    let status = req.status.or(Some(existing.status.clone().unwrap_or("draft".to_string())));

    let article = sqlx::query_as::<_, BusinessArticle>(
        "UPDATE business_articles \
         SET title=$1, slug=$2, meta_description=$3, content=$4, status=$5, updated_at=NOW() \
         WHERE id=$6 \
         RETURNING *"
    )
    .bind(&title)
    .bind(&slug)
    .bind(&meta_description)
    .bind(&content)
    .bind(&status)
    .bind(article_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(article)))
}

/// DELETE /business-articles/:id
///
/// Soft delete — sets status to 'archived'.
pub async fn delete_article(
    State(s): State<AppState>,
    Path(article_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query(
        "UPDATE business_articles SET status='archived', updated_at=NOW() WHERE id=$1 AND status != 'archived'"
    )
    .bind(article_id)
    .execute(&s.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Article not found or already archived".into()));
    }

    Ok(Json(json!({"deleted": true, "id": article_id})))
}

/// POST /business-articles/:id/generate-weekly
///
/// Regenerates article content for a subscription renewal.
/// Updates slug to keep URLs fresh. Extends subscription_expires_at by 1 week.
pub async fn generate_weekly(
    State(s): State<AppState>,
    Path(article_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let article = sqlx::query_as::<_, BusinessArticle>(
        "SELECT * FROM business_articles WHERE id=$1"
    )
    .bind(article_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Article not found".into()))?;

    // Verify it has a business (paid article)
    if article.business_id.is_none() {
        return Err(AppError::BadRequest("Cannot regenerate owner articles via weekly subscription".into()));
    }

    // Get directory info
    let dir_info = sqlx::query_as::<_, (String, String)>(
        "SELECT name, slug FROM directories WHERE id=$1"
    )
    .bind(article.directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Directory not found".into()))?;

    let (dir_name, dir_slug) = dir_info;

    // Get business info
    let biz_id = article.business_id.ok_or_else(|| AppError::BadRequest("Business ID required".into()))?;
    let biz_info = sqlx::query_as::<_, (String, String)>(
        "SELECT name, slug FROM businesses WHERE id=$1"
    )
    .bind(biz_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Business not found".into()))?;

    let (biz_name, biz_slug) = biz_info;

    // Parse city and service from keyword or fallback
    let service = capitalize_first(&article.keyword.split_whitespace()
        .last().unwrap_or(&article.keyword));
    let city = "your area"; // fallback

    // Generate fresh slug
    let ts = Utc::now().timestamp();
    let fresh_slug = format!("{}-{}", slugify(&article.keyword), ts);

    // Generate fresh content
    let title = format!(
        "{} – Updated Guide",
        &article.title
    );

    let fresh_content = generate_article_content(
        &article.keyword,
        &service,
        city,
        &service,
        &dir_name,
        &dir_slug,
        Some(&biz_name),
        Some(&biz_slug),
    );

    // Update article with fresh content, new slug, and extend subscription
    let updated = sqlx::query_as::<_, BusinessArticle>(
        "UPDATE business_articles \
         SET title=$1, slug=$2, content=$3, updated_at=NOW(), \
             subscription_expires_at = COALESCE(subscription_expires_at, NOW()) + INTERVAL '7 days', \
             subscription_active = true \
         WHERE id=$4 \
         RETURNING *"
    )
    .bind(&title)
    .bind(&fresh_slug)
    .bind(&fresh_content)
    .bind(article_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "message": "Weekly article regenerated",
        "article": updated
    })))
}

/// GET /articles/{slug}
///
/// Public serving route for business articles.
/// Returns full HTML page with SEO meta tags, author byline, and directory link.
/// Tracks impression asynchronously.
pub async fn serve_article(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    use sqlx::Row;

    let row = sqlx::query(
        "SELECT ba.*, d.name as directory_name, d.slug as directory_slug, \
         d.template_config, \
         b.name as business_name, b.slug as business_slug \
         FROM business_articles ba \
         JOIN directories d ON d.id = ba.directory_id \
         LEFT JOIN businesses b ON b.id = ba.business_id \
         WHERE ba.slug = $1 AND ba.status IN ('published', 'draft') \
         LIMIT 1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Article not found".into()))?;

    let article_id: Uuid = row.try_get("id").unwrap_or_default();
    let dir_id: Uuid = row.try_get("directory_id").unwrap_or_default();
    let dir_name: String = row.try_get::<String,_>("directory_name").unwrap_or_default();
    let dir_slug: String = row.try_get::<String,_>("directory_slug").unwrap_or_default();
    let biz_name: Option<String> = row.try_get::<Option<String>,_>("business_name").ok().flatten();
    let biz_slug: Option<String> = row.try_get::<Option<String>,_>("business_slug").ok().flatten();
    let title: String = row.try_get::<String,_>("title").unwrap_or_default();
    let meta_description: String = row.try_get::<Option<String>,_>("meta_description").ok().flatten().unwrap_or_default();
    let content: String = row.try_get::<Option<String>,_>("content").ok().flatten().unwrap_or_default();
    let keyword: String = row.try_get::<String,_>("keyword").unwrap_or_default();
    let status: String = row.try_get::<Option<String>,_>("status").ok().flatten().unwrap_or_default();
    let is_owner: bool = row.try_get::<Option<bool>,_>("is_owner_article").ok().flatten().unwrap_or(false);
    let created_at: Option<DateTime<Utc>> = row.try_get::<Option<DateTime<Utc>>,_>("created_at").ok().flatten();

    // Track impression asynchronously
    let db = s.db.clone();
    let aid = article_id;
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE business_articles SET impressions = impressions + 1 WHERE id=$1")
            .bind(aid)
            .execute(&db)
            .await;
    });

    let date_str = created_at.map(|dt| dt.format("%B %d, %Y").to_string()).unwrap_or_default();
    let author_label = if is_owner {
        format!("Published by {}", htmlesc(&dir_name))
    } else if let Some(ref bn) = biz_name {
        format!("Published by {}", htmlesc(bn))
    } else {
        format!("Published by {}", htmlesc(&dir_name))
    };

    let sponsored_label: String = row
        .try_get::<Option<serde_json::Value>, _>("template_config")
        .ok()
        .flatten()
        .and_then(|cfg| {
            cfg.get("sponsored_label")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "Sponsored".to_string());

    let escaped_label = htmlesc(&sponsored_label);
    let sponsored_badge = if !is_owner && biz_name.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        format!(
            r#"<div style="margin-bottom:16px"><span class="sponsored-badge">{}</span></div>"#,
            escaped_label
        )
    } else {
        String::new()
    };

    let biz_link = biz_name.zip(biz_slug.as_ref())
        .map(|(name, slug)| format!(
            r#"<p style="margin-bottom:16px">This article is brought to you by <a href="/{}/{}" class="business-link" style="color:#0d9488;font-weight:600">{}</a> — a trusted provider in the {} directory.</p>"#,
            htmlesc(&dir_slug),
            htmlesc(slug.as_str()),
            htmlesc(name.as_str()),
            htmlesc(&dir_name),
        ))
        .unwrap_or_default();


    let dir_link = format!(
        r#"<p style="margin-bottom:16px">Browse the full <a href="/{}" style="color:#0d9488;font-weight:600">{}</a> directory for more {}-related services.</p>"#,
        htmlesc(&dir_slug),
        htmlesc(&dir_name),
        htmlesc(&keyword),
    );

    let mt = htmlesc(&title);
    let md = htmlesc(&meta_description);
    let sl = htmlesc(&slug);
    let hn = htmlesc(&dir_name);
    let year_str = Utc::now().format("%Y").to_string();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{mt}</title>
    <meta name="description" content="{md}">
    <meta property="og:title" content="{mt}">
    <meta property="og:description" content="{md}">
    <meta name="robots" content="index,follow">
    <link rel="canonical" href="/articles/{sl}">
    <style>
        *{{margin:0;padding:0;box-sizing:border-box}}
        body{{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;background:#f8fafc;color:#1e293b;line-height:1.6}}
        .container{{max-width:800px;margin:0 auto;padding:40px 24px}}
        h1{{font-size:2rem;margin-bottom:12px;color:#0d9488}}
        .article-meta{{font-size:.85rem;color:#64748b;margin-bottom:8px}}
        .article-date{{font-size:.8rem;color:#94a3b8;margin-bottom:24px}}
        .content{{font-size:1rem;line-height:1.8}}
        .content p{{margin-bottom:16px}}
        .content h2{{font-size:1.3rem;margin-top:24px;margin-bottom:12px;color:#0f766e}}
        .content ul,.content ol{{margin:0 0 16px 24px}}
        .content a{{color:#0d9488;text-decoration:underline}}
        .content a:hover{{color:#0f766e}}
        footer{{margin-top:48px;padding-top:24px;border-top:1px solid #e2e8f0;font-size:.8rem;color:#64748b;text-align:center}}
        .status-badge{{display:inline-block;padding:4px 12px;background:#14532d;color:#bbf7d0;border-radius:4px;font-size:.8rem;font-weight:600}}
        .sponsored-badge{{display:inline-block;padding:4px 12px;background:#92400e;color:#fde68a;border-radius:4px;font-size:.75rem;font-weight:600;text-transform:uppercase;letter-spacing:.5px}}
    </style>
</head>
<body>
    <div class="container">
        {status_badge}
        {sponsored_badge}
        <h1>{mt}</h1>
        <div class="article-meta">{author_label}</div>
        <div class="article-date">{date_str}</div>
        <div class="content">{biz_link}{dir_link}{content_block}</div>
        <footer>
            <p>&copy; {year} | Powered by {hn}</p>
        </footer>
    </div>
    <script>
        fetch('/api/v1/business-articles/{article_id_str}/track', {{
            method: 'POST',
            headers: {{'Content-Type':'application/json'}},
            body: JSON.stringify({{event:'impression'}})
        }}).catch(function(){{}});
        // Click tracking on business links
        document.querySelectorAll('.business-link').forEach(function(el){{
            el.addEventListener('click',function(){{
                fetch('/api/v1/business-articles/{article_id_str}/track', {{
                    method: 'POST',
                    headers: {{'Content-Type':'application/json'}},
                    body: JSON.stringify({{event:'click'}})
                }}).catch(function(){{}});
            }});
        }});
    </script>
</body>
</html>"#,
        mt = mt,
        md = md,
        sl = sl,
        sponsored_badge = sponsored_badge,
        status_badge = if status == "published" {
            r#"<div style="margin-bottom:16px"><span class="status-badge">Published</span></div>"#.to_string()
        } else {
            String::new()
        },
        author_label = author_label,
        date_str = date_str,
        biz_link = biz_link,
        dir_link = dir_link,
        content_block = content,
        year = year_str,
        hn = hn,
        article_id_str = article_id.to_string(),
    );

    Ok((
        [("Content-Type", "text/html; charset=utf-8")],
        html,
    ))
}

/// POST /business-articles/:id/track
///
/// Tracks impression or click events on business articles.
pub async fn track_article_event(
    State(s): State<AppState>,
    Path(article_id): Path<Uuid>,
    Json(req): Json<super::trap_doors::TrackPageEventReq>,
) -> ApiResult<impl IntoResponse> {
    match req.event.to_lowercase().as_str() {
        "impression" => {
            sqlx::query("UPDATE business_articles SET impressions = impressions + 1 WHERE id=$1")
                .bind(article_id)
                .execute(&s.db)
                .await?;
        }
        "click" => {
            sqlx::query("UPDATE business_articles SET clicks = clicks + 1 WHERE id=$1")
                .bind(article_id)
                .execute(&s.db)
                .await?;
        }
        _ => return Err(AppError::Validation(format!(
            "Unknown event type '{}'. Valid: impression, click", req.event
        ))),
    }

    Ok(Json(json!({"tracked": req.event, "article_id": article_id})))
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
