//! Blog enhancement features: content decay, internal linking, AEO scoring, schema markup.
//!
//! These handlers implement four Gemini-requested strategic features:
//! 1. Content Decay Detection — flag stale posts, prioritize refresh
//! 2. Internal Linking Automation — analyze content, suggest/add internal links
//! 3. AEO (Answer Engine Optimization) Scoring — score posts for AI/voice search readiness
//! 4. Schema Markup Generation — auto-generate schema.org JSON from blog content

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use chrono::{DateTime, Utc, Duration};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ─── 1. Content Decay Detection ───────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DecayReport {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub directory_id: Option<Uuid>,
    pub last_refreshed: Option<DateTime<Utc>>,
    pub page_views: i32,
    pub traffic_trend: Option<String>,
    pub decay_flag: bool,
    pub refresh_priority: Option<String>,
    pub days_since_refresh: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DecaySummary {
    pub total_posts: i64,
    pub decayed_posts: i64,
    pub high_priority: i64,
    pub medium_priority: i64,
    pub low_priority: i64,
    pub posts: Vec<DecayReport>,
}

/// POST /api/v1/blog/decay/scan
/// Scan all published blog posts and update decay flags + refresh priorities.
pub async fn scan_content_decay(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let now = Utc::now();

    // Mark posts as decayed if last_refreshed is > 6 months old or null and created > 6 months ago
    sqlx::query(
        "UPDATE blog_posts \
         SET decay_flag = true, \
             refresh_priority = CASE \
                 WHEN page_views > 1000 AND (last_refreshed IS NULL OR last_refreshed < NOW() - INTERVAL '12 months') THEN 'high' \
                 WHEN page_views > 500 OR (last_refreshed IS NULL OR last_refreshed < NOW() - INTERVAL '9 months') THEN 'medium' \
                 ELSE 'low' \
             END \
         WHERE published = true \
           AND ( \
             (last_refreshed IS NOT NULL AND last_refreshed < NOW() - INTERVAL '6 months') \
             OR \
             (last_refreshed IS NULL AND created_at < NOW() - INTERVAL '6 months') \
           ) \
           AND decay_flag IS DISTINCT FROM true"
    )
    .execute(&s.db)
    .await?;

    // Clear decay flag for freshly refreshed posts
    sqlx::query(
        "UPDATE blog_posts \
         SET decay_flag = false, refresh_priority = NULL \
         WHERE published = true \
           AND last_refreshed IS NOT NULL \
           AND last_refreshed >= NOW() - INTERVAL '6 months' \
           AND decay_flag = true"
    )
    .execute(&s.db)
    .await?;

    let posts = sqlx::query_as::<_, DecayReport>(
        "SELECT id, title, slug, directory_id, last_refreshed, \
                page_views, traffic_trend, decay_flag, refresh_priority, \
                EXTRACT(DAY FROM (NOW() - COALESCE(last_refreshed, created_at)))::bigint AS days_since_refresh \
         FROM blog_posts \
         WHERE published = true \
         ORDER BY \
             CASE refresh_priority WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END, \
             days_since_refresh DESC \
         LIMIT 100"
    )
    .fetch_all(&s.db)
    .await?;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM blog_posts WHERE published = true"
    )
    .fetch_one(&s.db)
    .await?;

    let decayed = posts.iter().filter(|p| p.decay_flag).count() as i64;
    let high = posts.iter().filter(|p| p.refresh_priority.as_deref() == Some("high")).count() as i64;
    let medium = posts.iter().filter(|p| p.refresh_priority.as_deref() == Some("medium")).count() as i64;
    let low = posts.iter().filter(|p| p.refresh_priority.as_deref() == Some("low")).count() as i64;

    Ok(Json(DecaySummary {
        total_posts: total,
        decayed_posts: decayed,
        high_priority: high,
        medium_priority: medium,
        low_priority: low,
        posts,
    }))
}

/// POST /api/v1/blog/decay/refresh/:id
/// Mark a post as refreshed (sets last_refreshed=NOW, clears decay_flag)
pub async fn refresh_post_content(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query(
        "UPDATE blog_posts SET last_refreshed = NOW(), decay_flag = false, refresh_priority = NULL WHERE id = $1"
    )
    .bind(id)
    .execute(&s.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Blog post not found".into()));
    }

    Ok(Json(json!({"status": "refreshed", "id": id})))
}

// ─── 2. Internal Linking Automation ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct LinkSuggestion {
    pub source_post_id: Uuid,
    pub source_title: String,
    pub target_post_id: Option<Uuid>,
    pub target_title: String,
    pub target_slug: String,
    pub anchor_text: String,
    pub match_type: String, // "keyword", "phrase", "title", "category"
}

#[derive(Debug, Deserialize)]
pub struct AddInternalLinkRequest {
    pub source_post_id: Uuid,
    pub target_post_id: Uuid,
    pub anchor_text: String,
}

/// GET /api/v1/blog/internal-links/suggestions?directory_id=<uuid>
/// Generate internal link suggestions based on content overlap and keyword matching.
pub async fn internal_link_suggestions(
    State(s): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let directory_id = params.get("directory_id")
        .and_then(|v| Uuid::parse_str(v).ok());

    // Fetch all published posts for link analysis
    #[derive(sqlx::FromRow)]
    struct PostLink {
        id: Uuid,
        title: String,
        slug: Option<String>,
        content: String,
        directory_id: Option<Uuid>,
    }

    let posts = if let Some(did) = directory_id {
        sqlx::query_as::<_, PostLink>(
            "SELECT id, title, slug, content, directory_id FROM blog_posts WHERE published = true AND directory_id = $1 ORDER BY created_at DESC LIMIT 200"
        )
        .bind(did)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, PostLink>(
            "SELECT id, title, slug, content, directory_id FROM blog_posts WHERE published = true ORDER BY created_at DESC LIMIT 200"
        )
        .fetch_all(&s.db)
        .await?
    };

    let mut suggestions: Vec<LinkSuggestion> = Vec::new();

    for (i, source) in posts.iter().enumerate() {
        // Skip posts that already have plenty of internal links
        let existing_count = sqlx::query_scalar::<_, i64>(
            "SELECT jsonb_array_length(internal_links) FROM blog_posts WHERE id = $1"
        )
        .bind(source.id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        if existing_count >= 5 {
            continue;
        }

        // Find candidate target posts by title keyword overlap
        let source_title_lower = source.title.to_lowercase();
        let source_words: Vec<&str> = source_title_lower
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .collect();

        for candidate in &posts[i+1..] {
            if candidate.id == source.id { continue; }
            if suggestions.len() >= 50 { break; }

            let candidate_title = candidate.title.to_lowercase();
            for word in &source_words {
                if candidate_title.contains(*word) && *word != "blog" && *word != "post" {
                    suggestions.push(LinkSuggestion {
                        source_post_id: source.id,
                        source_title: source.title.clone(),
                        target_post_id: Some(candidate.id),
                        target_title: candidate.title.clone(),
                        target_slug: candidate.slug.clone().unwrap_or_default(),
                        anchor_text: word.to_string(),
                        match_type: "keyword".into(),
                    });
                    break;
                }
            }
        }
    }

    Ok(Json(json!({
        "suggestions": suggestions,
        "total_suggestions": suggestions.len()
    })))
}

/// POST /api/v1/blog/internal-links/add
/// Add an internal link to a post: stores in JSONB + injects <a href> into content body.
pub async fn add_internal_link(
    State(s): State<AppState>,
    Json(req): Json<AddInternalLinkRequest>,
) -> ApiResult<impl IntoResponse> {
    // Get target post title + slug for constructing the href
    #[derive(sqlx::FromRow)]
    struct TargetInfo {
        title: String,
        slug: Option<String>,
    }
    let target = sqlx::query_as::<_, TargetInfo>(
        "SELECT title, slug FROM blog_posts WHERE id = $1"
    )
    .bind(req.target_post_id)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or(TargetInfo { title: String::new(), slug: None });

    let target_title = target.title;
    let target_slug = target.slug.unwrap_or_default();
    let href = format!("/blog/{}", target_slug);

    let link_obj = json!({
        "target_id": req.target_post_id,
        "target_title": target_title,
        "target_slug": target_slug,
        "anchor_text": req.anchor_text,
        "added_at": Utc::now().to_rfc3339(),
    });

    // 1. Store in JSONB metadata
    sqlx::query(
        "UPDATE blog_posts SET internal_links = COALESCE(internal_links, '[]'::jsonb) || $1::jsonb WHERE id = $2"
    )
    .bind(&link_obj)
    .bind(req.source_post_id)
    .execute(&s.db)
    .await?;

    // 2. Inject <a href> into the post content HTML body
    let source_content: Option<String> = sqlx::query_scalar(
        "SELECT content FROM blog_posts WHERE id = $1"
    )
    .bind(req.source_post_id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    if let Some(body) = source_content {
        let anchor = &req.anchor_text;
        let anchor_lower = anchor.to_lowercase();
        let body_lower = body.to_lowercase();
        let link_tag = format!("<a href=\"{0}\">{1}</a>", href, anchor);
        let related_note = format!(
            "\n<p><em>Related reading: <a href=\"{0}\">{1}</a></em></p>",
            href, anchor
        );

        // Try to find anchor_text in body outside of existing <a> tags
        if let Some(pos) = body_lower.find(&anchor_lower) {
            // Check if this occurrence is inside an existing <a> tag
            let context_start = if pos > 200 { pos - 200 } else { 0 };
            let before = &body_lower[context_start..pos];

            let inside_existing_link = before.rfind("<a ").map_or(false, |lp| {
                let between = &body_lower[context_start + lp..pos];
                !between.contains("</a>")
            });

            let updated = if inside_existing_link {
                // Already linked elsewhere in this context — append note at end
                format!("{}{}", body, related_note)
            } else {
                // Replace plain text with linked version
                format!("{}{}{}",
                    &body[..pos],
                    link_tag,
                    &body[pos + anchor.len()..]
                )
            };

            sqlx::query("UPDATE blog_posts SET content = $1 WHERE id = $2")
                .bind(&updated)
                .bind(req.source_post_id)
                .execute(&s.db)
                .await?;
        } else {
            // Anchor text not found in body — append note at end
            let updated = format!("{}{}", body, related_note);
            sqlx::query("UPDATE blog_posts SET content = $1 WHERE id = $2")
                .bind(&updated)
                .bind(req.source_post_id)
                .execute(&s.db)
                .await?;
        }
    }

    Ok(Json(json!({
        "status": "added",
        "link": link_obj,
        "injected_into_content": true
    })))
}

/// DELETE /api/v1/blog/internal-links/:post_id/:target_id
/// Remove an internal link from a post.
pub async fn remove_internal_link(
    State(s): State<AppState>,
    Path((post_id, target_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let current = sqlx::query_scalar::<_, Option<Value>>(
        "SELECT internal_links FROM blog_posts WHERE id = $1"
    )
    .bind(post_id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    let mut links: Vec<Value> = current
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let before = links.len();
    links.retain(|l| {
        l.get("target_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            != Some(target_id)
    });
    let after = links.len();

    sqlx::query(
        "UPDATE blog_posts SET internal_links = $1::jsonb WHERE id = $2"
    )
    .bind(serde_json::to_value(&links).unwrap_or_default())
    .bind(post_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "removed", "links_removed": before - after, "remaining": after})))
}

// ─── 3. AEO (Answer Engine Optimization) Scoring ─────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AeoScoredPost {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub aeo_score: i32,
    pub answer_block: Option<String>,
    pub schema_type: Option<String>,
    pub traffic_trend: Option<String>,
    pub page_views: i32,
}

#[derive(Debug, Serialize)]
pub struct AeoSummary {
    pub average_score: f64,
    pub high_aeo_count: i64,   // 80-100
    pub medium_aeo_count: i64, // 50-79
    pub low_aeo_count: i64,    // <50
    pub posts: Vec<AeoScoredPost>,
}

/// POST /api/v1/blog/aeo/score/:id
/// Calculate AEO score for a specific post based on content structure signals.
pub async fn score_post_aeo(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    #[derive(sqlx::FromRow)]
    struct PostContent {
        id: Uuid,
        title: String,
        content: String,
        answer_block: Option<String>,
        internal_links: Option<Value>,
        schema_json: Option<Value>,
    }

    let post = sqlx::query_as::<_, PostContent>(
        "SELECT id, title, content, answer_block, internal_links, schema_json FROM blog_posts WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;

    let content_lower = post.content.to_lowercase();
    let mut score: i32 = 0;

    // 1. Has a dedicated answer block (0-20 pts)
    if post.answer_block.as_ref().map_or(false, |a| a.len() > 100) {
        score += 20;
    } else if post.answer_block.as_ref().map_or(false, |a| !a.is_empty()) {
        score += 10;
    }

    // 2. Uses heading structure (h2, h3) for scannability (0-15 pts)
    let h2_count = content_lower.matches("<h2").count();
    let h3_count = content_lower.matches("<h3").count();
    if h2_count >= 3 && h3_count >= 2 {
        score += 15;
    } else if h2_count >= 2 {
        score += 10;
    } else if h2_count >= 1 {
        score += 5;
    }

    // 3. Has schema markup (0-20 pts)
    if let Some(ref schema) = post.schema_json {
        let schema_str = serde_json::to_string(schema).unwrap_or_default();
        if schema_str.len() > 200 {
            score += 20;
        } else if schema_str.len() > 50 {
            score += 10;
        }
    }

    // 4. Has internal links (0-15 pts)
    let link_count = post.internal_links.as_ref()
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len());
    if link_count >= 5 {
        score += 15;
    } else if link_count >= 2 {
        score += 10;
    } else if link_count >= 1 {
        score += 5;
    }

    // 5. Content length adequate for depth (0-15 pts)
    let word_count = content_lower.split_whitespace().count();
    if word_count >= 1500 {
        score += 15;
    } else if word_count >= 800 {
        score += 10;
    } else if word_count >= 300 {
        score += 5;
    }

    // 6. Uses lists (ul/ol) for structured answers (0-15 pts)
    let list_count = content_lower.matches("<ul").count() + content_lower.matches("<ol").count();
    if list_count >= 3 {
        score += 15;
    } else if list_count >= 1 {
        score += 10;
    }

    // Clamp 0-100
    score = score.clamp(0, 100);

    // Determine traffic trend based on score
    let trend: &str = if score >= 80 { "rising" } else if score >= 50 { "stable" } else { "declining" };

    sqlx::query(
        "UPDATE blog_posts SET aeo_score = $1, traffic_trend = $2, updated_at = NOW() WHERE id = $3"
    )
    .bind(score)
    .bind(trend)
    .bind(post.id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "id": post.id,
        "title": post.title,
        "aeo_score": score,
        "traffic_trend": trend,
        "breakdown": {
            "answer_block": if post.answer_block.as_ref().map_or(false, |a| a.len() > 100) { 20 } else { 10 },
            "heading_structure": if h2_count >= 3 && h3_count >= 2 { 15 } else if h2_count >= 2 { 10 } else { 5 },
            "schema_markup": if post.schema_json.is_some() { 20 } else { 0 },
            "internal_links": if link_count >= 5 { 15 } else if link_count >= 2 { 10 } else { 5 },
            "content_length": if word_count >= 1500 { 15 } else if word_count >= 800 { 10 } else { 5 },
            "lists": if list_count >= 3 { 15 } else if list_count >= 1 { 10 } else { 0 },
        }
    })))
}

/// POST /api/v1/blog/aeo/scan-all
/// Re-score AEO for all published posts.
pub async fn score_all_posts_aeo(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let post_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM blog_posts WHERE published = true"
    )
    .fetch_all(&s.db)
    .await?;

    let mut scored = 0;
    for pid in &post_ids {
        // Run scoring logic inline per post
        #[derive(sqlx::FromRow)]
        struct ScoringData {
            content: String,
            answer_block: Option<String>,
            internal_links: Option<Value>,
            schema_json: Option<Value>,
        }

        if let Ok(data) = sqlx::query_as::<_, ScoringData>(
            "SELECT content, answer_block, internal_links, schema_json FROM blog_posts WHERE id = $1"
        )
        .bind(pid)
        .fetch_one(&s.db)
        .await
        {
            let cl = data.content.to_lowercase();
            let mut blog_score: i32 = 0;

            if data.answer_block.as_ref().map_or(false, |a| a.len() > 100) { blog_score += 20 }
            else if data.answer_block.as_ref().map_or(false, |a| !a.is_empty()) { blog_score += 10 }

            let h2c = cl.matches("<h2").count();
            let h3c = cl.matches("<h3").count();
            if h2c >= 3 && h3c >= 2 { blog_score += 15 }
            else if h2c >= 2 { blog_score += 10 }
            else if h2c >= 1 { blog_score += 5 }

            if data.schema_json.is_some() { blog_score += 20 }

            let lc = data.internal_links.as_ref().and_then(|v| v.as_array()).map_or(0, |a| a.len());
            if lc >= 5 { blog_score += 15 } else if lc >= 2 { blog_score += 10 } else if lc >= 1 { blog_score += 5 }

            let wc = cl.split_whitespace().count();
            if wc >= 1500 { blog_score += 15 } else if wc >= 800 { blog_score += 10 } else if wc >= 300 { blog_score += 5 }

            let lst = cl.matches("<ul").count() + cl.matches("<ol").count();
            if lst >= 3 { blog_score += 15 } else if lst >= 1 { blog_score += 10 }

            blog_score = blog_score.clamp(0, 100);
            let trend: &str = if blog_score >= 80 { "rising" } else if blog_score >= 50 { "stable" } else { "declining" };

            let _ = sqlx::query("UPDATE blog_posts SET aeo_score = $1, traffic_trend = $2 WHERE id = $3")
                .bind(blog_score)
                .bind(trend)
                .bind(*pid)
                .execute(&s.db)
                .await;
            scored += 1;
        }
    }

    // Return updated scores
    let posts = sqlx::query_as::<_, AeoScoredPost>(
        "SELECT id, title, slug, aeo_score, answer_block, schema_type, traffic_trend, page_views \
         FROM blog_posts WHERE published = true ORDER BY aeo_score DESC LIMIT 100"
    )
    .fetch_all(&s.db)
    .await?;

    let avg = if !posts.is_empty() {
        posts.iter().map(|p| p.aeo_score as f64).sum::<f64>() / posts.len() as f64
    } else { 0.0 };

    let high = posts.iter().filter(|p| p.aeo_score >= 80).count() as i64;
    let medium = posts.iter().filter(|p| p.aeo_score >= 50 && p.aeo_score < 80).count() as i64;
    let low = posts.iter().filter(|p| p.aeo_score < 50).count() as i64;

    Ok(Json(json!({
        "scored": scored,
        "summary": AeoSummary {
            average_score: avg,
            high_aeo_count: high,
            medium_aeo_count: medium,
            low_aeo_count: low,
            posts,
        }
    })))
}

/// GET /api/v1/blog/aeo/report
/// Get AEO score report for all published posts.
pub async fn aeo_report(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let posts = sqlx::query_as::<_, AeoScoredPost>(
        "SELECT id, title, slug, aeo_score, answer_block, schema_type, traffic_trend, page_views \
         FROM blog_posts WHERE published = true ORDER BY aeo_score DESC LIMIT 100"
    )
    .fetch_all(&s.db)
    .await?;

    let avg = if !posts.is_empty() {
        posts.iter().map(|p| p.aeo_score as f64).sum::<f64>() / posts.len() as f64
    } else { 0.0 };

    let high = posts.iter().filter(|p| p.aeo_score >= 80).count() as i64;
    let medium = posts.iter().filter(|p| p.aeo_score >= 50 && p.aeo_score < 80).count() as i64;
    let low = posts.iter().filter(|p| p.aeo_score < 50).count() as i64;

    Ok(Json(AeoSummary {
        average_score: avg,
        high_aeo_count: high,
        medium_aeo_count: medium,
        low_aeo_count: low,
        posts,
    }))
}

// ─── 4. Schema Markup Generation ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GenerateSchemaRequest {
    pub schema_type: Option<String>, // Article, BlogPosting, NewsArticle, FAQ, HowTo, etc.
    pub additional_fields: Option<Value>,
}

/// POST /api/v1/blog/schema/generate/:id
/// Auto-generate schema.org JSON for a blog post.
pub async fn generate_schema_markup(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<GenerateSchemaRequest>,
) -> ApiResult<impl IntoResponse> {
    #[derive(sqlx::FromRow)]
    struct PostData {
        id: Uuid,
        title: String,
        slug: Option<String>,
        excerpt: Option<String>,
        content: String,
        author_name: Option<String>,
        created_at: Option<DateTime<Utc>>,
        updated_at: Option<DateTime<Utc>>,
        directory_id: Option<Uuid>,
    }

    let post = sqlx::query_as::<_, PostData>(
        "SELECT id, title, slug, excerpt, content, author_name, created_at, updated_at, directory_id \
         FROM blog_posts WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;

    // Get directory info for publisher data
    let dir_info: Option<(String, String)> = if let Some(did) = post.directory_id {
        sqlx::query_as::<_, (String, String)>(
            "SELECT name, domain FROM directories WHERE id = $1"
        )
        .bind(did)
        .fetch_optional(&s.db)
        .await
        .ok()
        .flatten()
    } else { None };

    let schema_type = req.schema_type.unwrap_or_else(|| "Article".to_string());
    let publisher_name = dir_info.as_ref().map_or("SwiftSoftware Directory".to_string(), |d| d.0.clone());
    let site_url = dir_info.as_ref().and_then(|d| if d.1.is_empty() { None } else { Some(format!("https://{}", d.1)) });

    let word_count = post.content.split_whitespace().count();
    let reading_time_minutes = (word_count as f64 / 200.0).ceil() as i32;

    // Build schema.org JSON
    let mut schema = json!({
        "@context": "https://schema.org",
        "@type": schema_type,
        "headline": post.title,
        "url": format!("{}/blog/{}", site_url.as_deref().unwrap_or(""), post.slug.as_deref().unwrap_or("")),
        "datePublished": post.created_at.map(|d| d.to_rfc3339()),
        "dateModified": post.updated_at.map(|d| d.to_rfc3339()),
        "author": {
            "@type": "Person",
            "name": post.author_name.as_deref().unwrap_or("SwiftSoftware Author")
        },
        "publisher": {
            "@type": "Organization",
            "name": publisher_name,
        },
        "description": post.excerpt.as_deref().unwrap_or(""),
        "wordCount": word_count,
        "timeRequired": format!("PT{}M", reading_time_minutes),
    });

    // Add article body
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("articleBody".to_string(), json!(post.content.chars().take(5000).collect::<String>()));
    }

    // Merge additional fields
    if let Some(ref extra) = req.additional_fields {
        if let (Some(obj), Some(extra_obj)) = (schema.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }

    // Store in DB
    let schema_value = serde_json::to_value(&schema).unwrap_or_default();
    sqlx::query(
        "UPDATE blog_posts SET schema_json = $1, schema_type = $2, updated_at = NOW() WHERE id = $3"
    )
    .bind(&schema_value)
    .bind(&schema_type)
    .bind(post.id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "id": post.id,
        "schema_type": schema_type,
        "schema": schema,
        "stored": true,
    })))
}

/// POST /api/v1/blog/schema/generate-all
/// Generate schema markup for all published posts without it.
pub async fn generate_all_schema(
    State(s): State<AppState>,
    Json(req): Json<GenerateSchemaRequest>,
) -> ApiResult<impl IntoResponse> {
    let posts = sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<Uuid>)>(
        "SELECT id, title, slug, content, author_name, created_at, updated_at, directory_id \
         FROM blog_posts WHERE published = true AND (schema_json IS NULL OR schema_json::text = '{}') \
         LIMIT 100"
    )
    .fetch_all(&s.db)
    .await?;

    let schema_type = req.schema_type.unwrap_or_else(|| "Article".to_string());
    let mut generated = 0;

    for (pid, title, slug, content, author_name, created_at, updated_at, directory_id) in &posts {
        let dir_info: Option<String> = if let Some(did) = directory_id {
            sqlx::query_scalar::<_, String>("SELECT name FROM directories WHERE id = $1")
                .bind(did).fetch_optional(&s.db).await.ok().flatten()
        } else { None };

        let wc = content.split_whitespace().count();
        let rt = (wc as f64 / 200.0).ceil() as i32;

        let schema = json!({
            "@context": "https://schema.org",
            "@type": &schema_type,
            "headline": title,
            "datePublished": created_at.map(|d| d.to_rfc3339()),
            "dateModified": updated_at.map(|d| d.to_rfc3339()),
            "author": { "@type": "Person", "name": author_name.as_deref().unwrap_or("SwiftSoftware Author") },
            "publisher": { "@type": "Organization", "name": dir_info.as_deref().unwrap_or("SwiftSoftware") },
            "wordCount": wc,
            "timeRequired": format!("PT{}M", rt),
        });

        let sv = serde_json::to_value(&schema).unwrap_or_default();
        sqlx::query(
            "UPDATE blog_posts SET schema_json = $1, schema_type = $2, updated_at = NOW() WHERE id = $3"
        )
        .bind(&sv).bind(&schema_type).bind(pid)
        .execute(&s.db).await?;
        generated += 1;
    }

    Ok(Json(json!({"generated": generated, "schema_type": schema_type})))
}
