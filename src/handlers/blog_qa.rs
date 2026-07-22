//! Blog Q&A Automation — keyword fetching, post generation, newsletter digests, integration configs
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

// ─── Keyword Request/Response types ───

#[derive(Deserialize)]
pub struct FetchKeywordsReq {
    pub directory_id: i32,
    pub seed_keywords: Vec<String>,
    pub source: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct KeywordItem {
    pub id: Uuid,
    pub directory_id: i32,
    pub question: String,
    pub keyword: String,
    pub intent: String,
    pub source: String,
    pub frequency: i32,
    pub status: String,
    pub target_category: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct KeywordListQuery {
    pub directory_id: Option<i32>,
    pub status: Option<String>,
    pub source: Option<String>,
    pub intent: Option<String>,
    pub page: Option<i32>,
}

#[derive(Serialize)]
pub struct KeywordListResponse {
    pub keywords: Vec<KeywordItem>,
    pub total: i64,
    pub page: i32,
}

#[derive(Deserialize)]
pub struct GeneratePostsReq {
    pub directory_id: i32,
    pub count: Option<i32>,
    pub template_id: Option<String>,
}

#[derive(Serialize)]
pub struct GeneratePostsResponse {
    pub generated: usize,
    pub posts: Vec<String>,
}

#[derive(Deserialize)]
pub struct DigestReq {
    pub directory_id: i32,
}

#[derive(Serialize)]
pub struct DigestResponse {
    pub digest_id: Uuid,
    pub title: String,
    pub body: String,
    pub source_count: usize,
}

#[derive(Deserialize)]
pub struct SendDigestReq {
    pub digest_id: Uuid,
}

#[derive(Deserialize)]
pub struct ScheduleReq {
    pub directory_id: i32,
    pub day_of_week: String,
    pub hour: i32,
    pub posts_per_week: i32,
}

#[derive(Serialize)]
pub struct ScheduleResponse {
    pub scheduled: bool,
    pub config: Value,
}

// ─── Integration Config types ───

#[derive(Deserialize)]
pub struct SaveConfigReq {
    pub provider: String,
    pub config: Value,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ConfigItem {
    pub id: Uuid,
    pub provider: String,
    pub config: Value,
    pub enabled: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

fn mask_api_key(val: &Value) -> Value {
    if let Some(s) = val.as_str() {
        if s.len() > 4 {
            let masked = format!("{}****", &s[s.len()-4..]);
            return Value::String(masked);
        }
        return Value::String("****".to_string());
    }
    val.clone()
}

fn mask_config(config: &mut Value) {
    if let Some(obj) = config.as_object_mut() {
        if let Some(key) = obj.get("api_key") {
            obj.insert("api_key".to_string(), mask_api_key(key));
        }
        if let Some(key) = obj.get("login") {
            obj.insert("login".to_string(), mask_api_key(key));
        }
    }
}

// ─── Handlers ───

/// POST /api/v1/blog-qa/fetch-keywords
pub async fn fetch_keywords(
    State(s): State<AppState>,
    Json(req): Json<FetchKeywordsReq>,
) -> ApiResult<impl IntoResponse> {
    let directory_name = sqlx::query_scalar::<_, String>(
        "SELECT name FROM directories WHERE id = $1"
    )
    .bind(req.directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let ai_prompt = format!(
        "You are a local SEO expert for {dir}. Generate 50 common questions people search for \
         related to: {keywords}. Return ONLY a valid JSON array of objects, no other text. \
         Each object: {{ \"question\": \"...\", \"keyword\": \"...\", \"intent\": \"how|what|why|where|when|which\" }}. \
         Make questions specific to {dir} and local searches.",
        dir = directory_name,
        keywords = req.seed_keywords.join(", ")
    );

    // Call the existing AI blog generator provider
    let ai_result = call_ai_for_keywords(&s, &ai_prompt).await?;

    let mut count = 0i32;
    for item in &ai_result {
        let intent = item.get("intent").and_then(|v| v.as_str()).unwrap_or("how");
        let keyword = item.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
        let question = item.get("question").and_then(|v| v.as_str()).unwrap_or("");

        sqlx::query(
            "INSERT INTO blog_qa_keywords (directory_id, question, keyword, intent, source, frequency, status) \
             VALUES ($1, $2, $3, $4, $5, 1, 'unused') \
             ON CONFLICT DO NOTHING"
        )
        .bind(req.directory_id)
        .bind(question)
        .bind(keyword)
        .bind(intent)
        .bind(&req.source)
        .execute(&s.db)
        .await
        .ok();
        count += 1;
    }

    Ok(Json(serde_json::json!({
        "count": count,
        "keywords": ai_result
    })))
}

async fn call_ai_for_keywords(state: &AppState, prompt: &str) -> Result<Vec<Value>, AppError> {
    // Try to read AI provider config, fall back to blog_generator defaults
    let config = sqlx::query_as::<_, (Option<String>,)>(
        "SELECT config::text FROM integration_configs WHERE provider = 'ai_provider' AND enabled = true"
    )
    .fetch_optional(&state.db)
    .await?
    .and_then(|(c,)| c)
    .and_then(|c| serde_json::from_str::<Value>(&c).ok());

    // Pass to the existing blog_generator's AI call mechanism
    // For now, generate via the same pipeline the blog_generator uses
    let result = crate::handlers::blog_generator::call_llm_json(
        &state.db,
        &state.config,
        prompt,
        config.as_ref(),
    )
    .await
    .map_err(|e| AppError::Internal(format!("AI keyword generation failed: {}", e)))?;

    Ok(result)
}

/// POST /api/v1/blog-qa/generate-posts
pub async fn generate_posts(
    State(s): State<AppState>,
    Json(req): Json<GeneratePostsReq>,
) -> ApiResult<impl IntoResponse> {
    let count = req.count.unwrap_or(5).max(1).min(20);

    let rows = sqlx::query(
        "SELECT id, question, keyword, target_category FROM blog_qa_keywords \
         WHERE directory_id = $1 AND status = 'unused' \
         ORDER BY frequency DESC LIMIT $2"
    )
    .bind(req.directory_id)
    .bind(count)
    .fetch_all(&s.db)
    .await?;

    let mut generated_titles: Vec<String> = Vec::new();

    for row in &rows {
        let kw_id: Uuid = row.get("id");
        let question: String = row.get("question");
        let keyword: String = row.get("keyword");
        let target_category: Option<String> = row.get("target_category");

        // Build AI prompt for the blog post
        let category_ctx = target_category.as_deref().unwrap_or("local businesses");
        let prompt = format!(
            "Write a thorough 800-word blog post answering: '{question}'. \
             Format with H2 and H3 headings. Short paragraphs. Include actionable advice. \
             The topic is about {category_val}. \
             At the end, add a section with the text '[INSERT_DIRECTORY_LINK]' \
             as a placeholder for a relevant directory category link.",
            category_val = category_ctx
        );

        // Generate content via blog_generator
        let content = crate::handlers::blog_generator::generate_blog_content(
            &s.db, &s.config, &prompt
        )
        .await
        .map_err(|e| AppError::Internal(format!("Blog generation failed: {}", e)))?;

        // Replace placeholder with actual link if target category exists
        let final_content = if let Some(ref cat) = target_category {
            content.replace("[INSERT_DIRECTORY_LINK]",
                &format!("<a href=\"/api/v1/d/{}/categories/{}\">Browse {} {}</a>",
                    req.directory_id, cat, cat, "services"))
        } else {
            content
        };

        // Insert blog post as draft
        let slug = question.to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-")
            .chars().take(100).collect::<String>();

        let blog_id = sqlx::query_scalar::<_, i32>(
            "INSERT INTO blog_posts (title, slug, excerpt, content, directory_id, published) \
             VALUES ($1, $2, $3, $4, $5, false) \
             RETURNING id"
        )
        .bind(&question)
        .bind(&slug)
        .bind(&format!("Answering: {}", question))
        .bind(&final_content)
        .bind(req.directory_id)
        .fetch_one(&s.db)
        .await?;

        // Record QA association
        sqlx::query(
            "INSERT INTO blog_qa_posts (directory_id, blog_post_id, question, keyword, status) \
             VALUES ($1, $2, $3, $4, 'draft')"
        )
        .bind(req.directory_id)
        .bind(blog_id)
        .bind(&question)
        .bind(&keyword)
        .execute(&s.db)
        .await?;

        // Mark keyword as drafted
        sqlx::query("UPDATE blog_qa_keywords SET status = 'drafted' WHERE id = $1")
            .bind(kw_id)
            .execute(&s.db)
            .await?;

        generated_titles.push(question);
    }

    Ok(Json(GeneratePostsResponse {
        generated: generated_titles.len(),
        posts: generated_titles,
    }))
}

/// GET /api/v1/blog-qa/keywords
pub async fn list_keywords(
    State(s): State<AppState>,
    Query(q): Query<KeywordListQuery>,
) -> ApiResult<impl IntoResponse> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = 50;
    let offset = (page - 1) * per_page;

    let mut where_clauses = Vec::new();
    let mut bind_idx = 1i32;

    if let Some(ref dir_id) = q.directory_id {
        where_clauses.push(format!("directory_id = ${}", bind_idx));
        bind_idx += 1;
    }
    if let Some(ref status) = q.status {
        where_clauses.push(format!("status = ${}", bind_idx));
        bind_idx += 1;
    }
    if let Some(ref source) = q.source {
        where_clauses.push(format!("source = ${}", bind_idx));
        bind_idx += 1;
    }
    if let Some(ref intent) = q.intent {
        where_clauses.push(format!("intent = ${}", bind_idx));
        bind_idx += 1;
    }

    let where_str = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    // Count query
    let count_sql = format!("SELECT COUNT(*) FROM blog_qa_keywords {}", where_str);
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(ref dir_id) = q.directory_id { count_query = count_query.bind(dir_id); }
    if let Some(ref status) = q.status { count_query = count_query.bind(status); }
    if let Some(ref source) = q.source { count_query = count_query.bind(source); }
    if let Some(ref intent) = q.intent { count_query = count_query.bind(intent); }
    let total = count_query.fetch_one(&s.db).await.unwrap_or(0);

    // Data query
    let data_sql = format!(
        "SELECT id, directory_id, question, keyword, intent, source, frequency, status, \
                target_category, created_at \
         FROM blog_qa_keywords {} ORDER BY frequency DESC, created_at DESC LIMIT ${} OFFSET ${}",
        where_str, bind_idx, bind_idx + 1
    );
    let mut data_query = sqlx::query_as::<_, KeywordItem>(&data_sql);
    if let Some(ref dir_id) = q.directory_id { data_query = data_query.bind(dir_id); }
    if let Some(ref status) = q.status { data_query = data_query.bind(status); }
    if let Some(ref source) = q.source { data_query = data_query.bind(source); }
    if let Some(ref intent) = q.intent { data_query = data_query.bind(intent); }
    data_query = data_query.bind(per_page).bind(offset);
    let keywords = data_query.fetch_all(&s.db).await.unwrap_or_default();

    Ok(Json(KeywordListResponse { keywords, total, page }))
}

/// POST /api/v1/blog-qa/generate-digest
pub async fn generate_digest(
    State(s): State<AppState>,
    Json(req): Json<DigestReq>,
) -> ApiResult<impl IntoResponse> {
    let dir_info = sqlx::query_as::<_, (String, String)>(
        "SELECT name, slug FROM directories WHERE id = $1"
    )
    .bind(req.directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let (dir_name, dir_slug) = dir_info;

    // Query latest 10 published QA blog posts
    let posts = sqlx::query(
        "SELECT bp.title, bp.excerpt, bp.slug, bp.created_at \
         FROM blog_qa_posts bqp JOIN blog_posts bp ON bqp.blog_post_id = bp.id \
         WHERE bqp.directory_id = $1 AND bp.published = true \
         ORDER BY bp.created_at DESC LIMIT 10"
    )
    .bind(req.directory_id)
    .fetch_all(&s.db)
    .await?;

    let mut body = format!(
        "<h1>{} — Weekly Roundup</h1><p>Latest insights and answers from the community.</p>",
        dir_name
    );
    let mut source_count = 0usize;

    for row in &posts {
        let title: String = row.get("title");
        let excerpt: Option<String> = row.get("excerpt");
        let slug: Option<String> = row.get("slug");

        let snippet = excerpt.as_deref().unwrap_or("").chars().take(200).collect::<String>();
        let slug_str = slug.as_deref().unwrap_or("post").to_string();
        body.push_str(&format!(
            "<div style=\"margin:20px 0;padding:16px;border:1px solid #e2e8f0;border-radius:8px\">\
             <h2>{}</h2><p>{}</p>\
             <a href=\"/api/v1/d/{}/blog/{}\">Read More &rarr;</a></div>",
            title, snippet, dir_slug, slug_str
        ));
        source_count += 1;
    }

    if source_count == 0 {
        body.push_str("<p>No published posts yet. Generate some Q&A posts first!</p>");
    }

    let digest_title = format!("{} Weekly Roundup — {}", dir_name, chrono::Utc::now().format("%B %d, %Y"));

    let digest_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO newsletter_digests (directory_id, title, body, status) \
         VALUES ($1, $2, $3, 'draft') RETURNING id"
    )
    .bind(req.directory_id)
    .bind(&digest_title)
    .bind(&body)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(DigestResponse {
        digest_id,
        title: digest_title,
        body,
        source_count,
    }))
}

/// POST /api/v1/blog-qa/send-digest
pub async fn send_digest(
    State(s): State<AppState>,
    Json(req): Json<SendDigestReq>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE newsletter_digests SET status = 'sent', sent_at = NOW() WHERE id = $1"
    )
    .bind(req.digest_id)
    .execute(&s.db)
    .await?;

    Ok(Json(serde_json::json!({"status": "sent"})))
}

/// POST /api/v1/blog-qa/schedule-weekly
pub async fn schedule_weekly(
    State(s): State<AppState>,
    Json(req): Json<ScheduleReq>,
) -> ApiResult<impl IntoResponse> {
    let config = serde_json::json!({
        "qa_automation": {
            "day_of_week": req.day_of_week,
            "hour": req.hour,
            "posts_per_week": req.posts_per_week
        }
    });

    // Store in directory's feature_config
    sqlx::query(
        "UPDATE directories SET template_config = COALESCE(template_config, '{}'::jsonb) || $1::jsonb WHERE id = $2"
    )
    .bind(config.to_string())
    .bind(req.directory_id)
    .execute(&s.db)
    .await?;

    Ok(Json(ScheduleResponse {
        scheduled: true,
        config: config,
    }))
}

// ─── Integration Config handlers ───

/// GET /api/v1/integration-configs
pub async fn list_configs(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let mut configs = sqlx::query_as::<_, ConfigItem>(
        "SELECT id, provider, config, enabled, created_at, updated_at FROM integration_configs ORDER BY provider"
    )
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    // Mask API keys
    for c in &mut configs {
        mask_config(&mut c.config);
    }

    Ok(Json(configs))
}

/// POST /api/v1/integration-configs
pub async fn save_config(
    State(s): State<AppState>,
    Json(req): Json<SaveConfigReq>,
) -> ApiResult<impl IntoResponse> {
    let config_json = serde_json::to_string(&req.config)
        .unwrap_or_else(|_| "{}".to_string());

    let item = sqlx::query_as::<_, ConfigItem>(
        "INSERT INTO integration_configs (provider, config) VALUES ($1, $2::jsonb) \
         ON CONFLICT (provider) DO UPDATE SET config = $2::jsonb, updated_at = NOW() \
         RETURNING id, provider, config, enabled, created_at, updated_at"
    )
    .bind(&req.provider)
    .bind(&config_json)
    .fetch_one(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to save config: {}", e)))?;

    let mut result = item;
    mask_config(&mut result.config);

    Ok(Json(result))
}

/// GET /api/v1/integration-configs/:provider
pub async fn get_config(
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let mut item = sqlx::query_as::<_, ConfigItem>(
        "SELECT id, provider, config, enabled, created_at, updated_at FROM integration_configs WHERE provider = $1"
    )
    .bind(&provider)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Provider config not found".into()))?;

    mask_config(&mut item.config);
    Ok(Json(item))
}

/// DELETE /api/v1/integration-configs/:provider
pub async fn delete_config(
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query("DELETE FROM integration_configs WHERE provider = $1")
        .bind(&provider)
        .execute(&s.db)
        .await?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

// ─── Search extension ───

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub source: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResultItem {
    pub r#type: String,
    pub title: String,
    pub excerpt: String,
    pub url: String,
    pub badge: String,
}

#[derive(Serialize)]
pub struct SearchResponse {
    pub source: String,
    pub results: Vec<SearchResultItem>,
    pub total: usize,
    pub query: String,
    pub engine: String,
}

/// GET /api/v1/search?q=...&source=all|web|blog|qa|news
pub async fn search_all(
    State(s): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<impl IntoResponse> {
    let query = q.q.trim();
    if query.is_empty() {
        return Ok(Json(SearchResponse {
            source: q.source.clone().unwrap_or_else(|| "all".into()),
            results: vec![],
            total: 0,
            query: query.to_string(),
            engine: "search".into(),
        }));
    }

    let source = q.source.as_deref().unwrap_or("all");
    let pattern = format!("%{}%", query);
    let mut results = Vec::new();

    match source {
        "blog" => {
            let rows = sqlx::query(
                "SELECT title, excerpt, slug, created_at FROM blog_posts \
                 WHERE (title ILIKE $1 OR excerpt ILIKE $1 OR content ILIKE $1) \
                 ORDER BY created_at DESC LIMIT 20"
            )
            .bind(&pattern)
            .fetch_all(&s.db)
            .await?;
            for row in rows {
                let title: String = row.get("title");
                let excerpt: Option<String> = row.get("excerpt");
                let slug: Option<String> = row.get("slug");
                results.push(SearchResultItem {
                    r#type: "blog_post".into(),
                    title,
                    excerpt: excerpt.unwrap_or_default(),
                    url: format!("/api/v1/d/blog/{}", slug.as_deref().unwrap_or("post")),
                    badge: "📝 Blog".into(),
                });
            }
        }
        "qa" => {
            let rows = sqlx::query(
                "SELECT question, keyword FROM blog_qa_keywords \
                 WHERE (question ILIKE $1 OR keyword ILIKE $1) \
                 ORDER BY frequency DESC LIMIT 20"
            )
            .bind(&pattern)
            .fetch_all(&s.db)
            .await?;
            for row in rows {
                let question: String = row.get("question");
                let keyword: String = row.get("keyword");
                results.push(SearchResultItem {
                    r#type: "qa_keyword".into(),
                    title: question,
                    excerpt: format!("Keyword: {}", keyword),
                    url: "#".into(),
                    badge: "❓ Q&A".into(),
                });
            }
        }
        "news" => {
            let two_days_ago = Utc::now() - chrono::Duration::hours(48);
            let rows = sqlx::query(
                "SELECT title, excerpt, slug FROM blog_posts \
                 WHERE (title ILIKE $1 OR excerpt ILIKE $1) \
                 AND created_at >= $2 \
                 ORDER BY created_at DESC LIMIT 20"
            )
            .bind(&pattern)
            .bind(two_days_ago)
            .fetch_all(&s.db)
            .await?;
            for row in rows {
                let title: String = row.get("title");
                let excerpt: Option<String> = row.get("excerpt");
                let slug: Option<String> = row.get("slug");
                results.push(SearchResultItem {
                    r#type: "news".into(),
                    title,
                    excerpt: excerpt.unwrap_or_default(),
                    url: format!("/api/v1/d/blog/{}", slug.as_deref().unwrap_or("post")),
                    badge: "📰 News".into(),
                });
            }
        }
        _ => {
            // "all" or "web" — existing full search
            // Businesses
            let biz_rows = sqlx::query(
                "SELECT name, description, slug FROM businesses \
                 WHERE name ILIKE $1 OR description ILIKE $1 \
                 LIMIT 10"
            )
            .bind(&pattern)
            .fetch_all(&s.db)
            .await?;
            for row in biz_rows {
                let name: String = row.get("name");
                let desc: Option<String> = row.get("description");
                let slug: Option<String> = row.get("slug");
                results.push(SearchResultItem {
                    r#type: "business".into(),
                    title: name,
                    excerpt: desc.unwrap_or_default(),
                    url: format!("/business/{}", slug.as_deref().unwrap_or("")),
                    badge: "🏪 Business".into(),
                });
            }
            // Blog posts too
            let blog_rows = sqlx::query(
                "SELECT title, excerpt, slug FROM blog_posts \
                 WHERE (title ILIKE $1 OR excerpt ILIKE $1) \
                 ORDER BY created_at DESC LIMIT 10"
            )
            .bind(&pattern)
            .fetch_all(&s.db)
            .await?;
            for row in blog_rows {
                let title: String = row.get("title");
                let excerpt: Option<String> = row.get("excerpt");
                let slug: Option<String> = row.get("slug");
                results.push(SearchResultItem {
                    r#type: "blog_post".into(),
                    title,
                    excerpt: excerpt.unwrap_or_default(),
                    url: format!("/api/v1/d/blog/{}", slug.as_deref().unwrap_or("post")),
                    badge: "📝 Blog".into(),
                });
            }
        }
    }

    let total = results.len();
    Ok(Json(SearchResponse {
        source: source.to_string(),
        results,
        total,
        query: query.to_string(),
        engine: "search".into(),
    }))
}
