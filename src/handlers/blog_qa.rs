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

    let keywords_json: Vec<Value> = match req.source.as_str() {
        "answer_the_public" => {
            fetch_atp_keywords(&s, &req.seed_keywords).await?
        }
        "dataforseo" => {
            fetch_dataforseo_keywords(&s, &req.seed_keywords, &directory_name).await?
        }
        _ => {
            // "ai_generated" — default
            let ai_prompt = format!(
                "You are a local SEO expert for {dir}. Generate 50 common questions people search for \
                 related to: {keywords}. Return ONLY a valid JSON array of objects, no other text. \
                 Each object: {{ \"question\": \"...\", \"keyword\": \"...\", \"intent\": \"how|what|why|where|when|which\" }}. \
                 Make questions specific to {dir} and local searches.",
                dir = directory_name,
                keywords = req.seed_keywords.join(", ")
            );
            let config = sqlx::query_as::<_, (Option<String>,)>(
                "SELECT config::text FROM integration_configs WHERE provider = 'ai_provider' AND enabled = true"
            )
            .fetch_optional(&s.db)
            .await?
            .and_then(|(c,)| c)
            .and_then(|c| serde_json::from_str::<Value>(&c).ok());

            crate::handlers::blog_generator::call_llm_json(
                &s.db,
                &s.config,
                &ai_prompt,
                config.as_ref(),
            )
            .await
            .map_err(|e| AppError::Internal(format!("AI keyword generation failed: {}", e)))?
        }
    };

    let mut count = 0i32;
    for item in &keywords_json {
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
        "keywords": keywords_json
    })))
}

// ── AnswerThePublic Integration ──
async fn fetch_atp_keywords(state: &AppState, seeds: &[String]) -> Result<Vec<Value>, AppError> {
    // Get configured API key from integration_configs or provider_keys
    let api_key = sqlx::query_scalar::<_, String>(
        r#"SELECT decrypt_provider_key(api_key_encrypted) 
         FROM provider_keys WHERE provider = 'answer_the_public' AND is_active = true
         UNION
         SELECT config->>'api_key' FROM integration_configs WHERE provider = 'answer_the_public' AND enabled = true
         LIMIT 1"#
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("AnswerThePublic API key not configured. Set it in Integrations page.".into()))?;

    let query = seeds.join(" ");
    let url = format!(
        "https://api.answerthepublic.com/v1/keywords?q={}&api_key={}&limit=50",
        urlencoding(query.as_str()),
        api_key
    );

    let client = reqwest::Client::new();
    let resp = client.get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("AnswerThePublic request failed: {}", e)))?;

    let atp_json: Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("AnswerThePublic response parse failed: {}", e)))?;

    // ATP response has { keyword: "...", questions: [ { question: "...", type: "question" }, ... ] }
    // Normalize to our format
    let mut results = Vec::new();
    if let Some(questions) = atp_json.get("results").or_else(|| atp_json.get("data")) {
        if let Some(arr) = questions.as_array() {
            for item in arr {
                let question = item.get("question")
                    .or_else(|| item.get("query"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !question.is_empty() {
                    // Infer a keyword from the seed or the question
                    let kw = seeds.get(0).cloned().unwrap_or_default();
                    let intent = if question.starts_with("how") { "how" }
                        else if question.starts_with("what") { "what" }
                        else if question.starts_with("why") { "why" }
                        else if question.starts_with("where") { "where" }
                        else if question.starts_with("when") { "when" }
                        else if question.starts_with("which") { "which" }
                        else { "what" };
                    results.push(serde_json::json!({
                        "question": question,
                        "keyword": kw,
                        "intent": intent
                    }));
                }
            }
        }
    }

    if results.is_empty() {
        return Err(AppError::NotFound("No questions returned from AnswerThePublic. Try broader seed keywords.".into()));
    }

    Ok(results)
}

// ── DataForSEO Integration ──
// Uses DataForSEO API v3 for keyword ideas: https://docs.dataforseo.com/v3/keywords_data/google_ads/keywords_for_site/live/
async fn fetch_dataforseo_keywords(state: &AppState, seeds: &[String], _directory: &str) -> Result<Vec<Value>, AppError> {
    let config_row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"SELECT decrypt_provider_key(api_key_encrypted) as api_key, 
                decrypt_provider_key(base_url_encrypted) as login
         FROM provider_keys WHERE provider = 'dataforseo' AND is_active = true
         LIMIT 1"#
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(
        "DataForSEO API key not configured. Set login + key in Integrations page.".into()
    ))?;

    let (api_key, login_opt) = config_row;
    let login = login_opt.unwrap_or_default();

    // DataForSEO uses Basic auth: login:api_key
    let auth = base64_encode(format!("{}:{}", login, api_key));

    let keywords = seeds.join(", ");
    let url = "https://api.dataforseo.com/v3/keywords_data/google_ads/keywords_for_keywords/live";
    let payload = serde_json::json!([{
        "keywords": seeds,
        "location_name": "United States",
        "language_name": "English",
        "include_serp_info": false,
        "clicks": true
    }]);

    let client = reqwest::Client::new();
    let resp = client.post(url)
        .header("Authorization", format!("Basic {}", auth))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("DataForSEO request failed: {}", e)))?;

    let dfs_json: Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("DataForSEO response parse failed: {}", e)))?;

    let mut results = Vec::new();
    if let Some(tasks) = dfs_json.get("tasks").and_then(|v| v.as_array()) {
        for task in tasks {
            if let Some(result) = task.get("result").and_then(|v| v.as_array()) {
                for item in result {
                    if let Some(keyword) = item.get("keyword").and_then(|v| v.as_str()) {
                        let competition = item.get("competition").and_then(|v| v.as_f64()).unwrap_or(0.5);
                        let search_volume = item.get("search_volume").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let intent = if competition > 0.7 { "how" }
                            else if search_volume > 1000.0 { "what" }
                            else { "question" };
                        let question = format!("What about {}?", keyword);
                        results.push(serde_json::json!({
                            "question": question,
                            "keyword": keyword,
                            "intent": intent
                        }));
                    }
                }
            }
        }
    }

    if results.is_empty() {
        return Err(AppError::NotFound(
            "No keywords returned from DataForSEO. Check API key validity.".into()
        ));
    }

    Ok(results)
}

fn urlencoding(s: &str) -> String {
    s.split_whitespace()
        .map(|w| w.to_string())
        .collect::<Vec<_>>()
        .join("%20")
}

fn base64_encode(s: String) -> String {
    use std::fmt::Write;
    let bytes = s.as_bytes();
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let tri = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((tri >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((tri >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((tri >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(tri & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
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
