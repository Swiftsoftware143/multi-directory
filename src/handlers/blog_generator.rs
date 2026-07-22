//! Blog post generator — template-based AI content creation with media injection.
//!
//! Supports configurable merge fields, per-directory and admin-level generation,
//! multi-LLM provider selection (DeepSeek, OpenAI, Gemini), and image/video injection.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use chrono::NaiveDateTime;
use std::collections::HashMap;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Blog Template CRUD ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BlogTemplate {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category: String,
    pub content_template: String,
    pub merge_fields: Option<serde_json::Value>,
    pub is_global: Option<bool>,
    pub directory_id: Option<Uuid>,
    pub template_type: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub image_provider: Option<String>,
    pub image_model: Option<String>,
    pub word_count: Option<i32>,
    pub is_admin: Option<bool>,
    pub status: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub content_template: String,
    pub merge_fields: Option<serde_json::Value>,
    pub is_global: Option<bool>,
    pub directory_id: Option<Uuid>,
    pub template_type: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub image_provider: Option<String>,
    pub image_model: Option<String>,
    pub word_count: Option<i32>,
    pub is_admin: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub content_template: Option<String>,
    pub merge_fields: Option<serde_json::Value>,
    pub is_global: Option<bool>,
    pub template_type: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub image_provider: Option<String>,
    pub image_model: Option<String>,
    pub word_count: Option<i32>,
    pub status: Option<String>,
}

// ── Generation Request/Response ──

#[derive(Debug, Deserialize)]
pub struct GenerateBlogRequest {
    pub template_id: Uuid,
    pub directory_ids: Vec<Uuid>,
    pub field_values: HashMap<String, String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub publish: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct GeneratedPost {
    pub id: Uuid,
    pub title: String,
    pub directory_id: Uuid,
    pub slug: String,
    pub status: String,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct GenerateBlogResponse {
    pub posts: Vec<GeneratedPost>,
    pub total_generated: usize,
    pub total_failed: usize,
}

// ── Template CRUD ──

pub async fn list_templates(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let dir_id = params.get("directory_id").and_then(|v| Uuid::parse_str(v).ok());
    let template_type = params.get("type");
    let category = params.get("category");

    let mut sql = "SELECT * FROM blog_templates WHERE status = 'active'".to_string();
    let mut p = 2;

    if dir_id.is_some() {
        sql.push_str(&format!(" AND (directory_id = ${p} OR is_global = true)"));
        p += 1;
    }
    if template_type.is_some() {
        sql.push_str(&format!(" AND template_type = ${p}"));
        p += 1;
    }
    if category.is_some() {
        sql.push_str(&format!(" AND category = ${p}"));
        p += 1;
    }
    sql.push_str(" ORDER BY name ASC");

    let mut q = sqlx::query_as::<_, BlogTemplate>(&sql);
    if let Some(did) = dir_id {
        q = q.bind(did);
    }
    if let Some(tt) = template_type {
        q = q.bind(tt);
    }
    if let Some(cat) = category {
        q = q.bind(cat);
    }
    let templates = q.fetch_all(&s.db).await?;

    Ok(Json(templates))
}

pub async fn get_template(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let tpl = sqlx::query_as::<_, BlogTemplate>("SELECT * FROM blog_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("template not found".into()))?;

    Ok(Json(tpl))
}

pub async fn create_template(
    State(s): State<AppState>,
    Json(req): Json<CreateTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    let tpl = sqlx::query_as::<_, BlogTemplate>(
        r#"INSERT INTO blog_templates (name, slug, description, category, content_template, merge_fields, is_global, directory_id, template_type, llm_provider, llm_model, image_provider, image_model, word_count, is_admin)
           VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10, $11, $12, $13, $14, $15)
           RETURNING *"#
    )
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.category.unwrap_or_else(|| "seo".to_string()))
    .bind(&req.content_template)
    .bind(req.merge_fields.unwrap_or(json!([])))
    .bind(req.is_global.unwrap_or(true))
    .bind(req.directory_id)
    .bind(req.template_type.unwrap_or_else(|| "article".to_string()))
    .bind(req.llm_provider.unwrap_or_else(|| "deepseek".to_string()))
    .bind(req.llm_model.unwrap_or_else(|| "deepseek-chat".to_string()))
    .bind(req.image_provider.unwrap_or_else(|| "none".to_string()))
    .bind(req.image_model.unwrap_or_else(|| "none".to_string()))
    .bind(req.word_count.unwrap_or(1000))
    .bind(req.is_admin.unwrap_or(false))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(tpl)))
}

pub async fn update_template(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    let _existing = sqlx::query_as::<_, BlogTemplate>("SELECT * FROM blog_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("template not found".into()))?;

    let tpl = sqlx::query_as::<_, BlogTemplate>(
        r#"UPDATE blog_templates SET
            name = COALESCE($1, name),
            description = COALESCE($2, description),
            category = COALESCE($3, category),
            content_template = COALESCE($4, content_template),
            merge_fields = COALESCE($5::jsonb, merge_fields),
            is_global = COALESCE($6, is_global),
            template_type = COALESCE($7, template_type),
            llm_provider = COALESCE($8, llm_provider),
            llm_model = COALESCE($9, llm_model),
            image_provider = COALESCE($10, image_provider),
            image_model = COALESCE($11, image_model),
            word_count = COALESCE($12, word_count),
            status = COALESCE($13, status),
            updated_at = NOW()
           WHERE id = $14
           RETURNING *"#
    )
    .bind(req.name)
    .bind(req.description)
    .bind(req.category)
    .bind(req.content_template)
    .bind(req.merge_fields)
    .bind(req.is_global)
    .bind(req.template_type)
    .bind(req.llm_provider)
    .bind(req.llm_model)
    .bind(req.image_provider)
    .bind(req.image_model)
    .bind(req.word_count)
    .bind(req.status)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(tpl))
}

pub async fn delete_template(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let r = sqlx::query("DELETE FROM blog_templates WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("template not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Template Directory Mappings ──

pub async fn set_template_directories(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(dirs): Json<Vec<Uuid>>,
) -> ApiResult<impl IntoResponse> {
    // Verify template exists
    sqlx::query_as::<_, BlogTemplate>("SELECT * FROM blog_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("template not found".into()))?;

    // Clear existing mappings
    sqlx::query("DELETE FROM blog_template_directories WHERE template_id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    // Insert new ones
    for did in &dirs {
        sqlx::query(
            "INSERT INTO blog_template_directories (template_id, directory_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        )
        .bind(id)
        .bind(did)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({"template_id": id, "directories": dirs})))
}

pub async fn get_template_directories(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dirs: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT directory_id FROM blog_template_directories WHERE template_id = $1"
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(dirs.into_iter().map(|d| d.0).collect::<Vec<_>>()))
}

// ── Blog Generation ──

pub async fn generate_blog_posts(
    State(s): State<AppState>,
    Json(req): Json<GenerateBlogRequest>,
) -> ApiResult<impl IntoResponse> {
    let tpl = sqlx::query_as::<_, BlogTemplate>("SELECT * FROM blog_templates WHERE id = $1")
        .bind(req.template_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("template not found".into()))?;

    let provider = req.llm_provider.as_deref().unwrap_or(tpl.llm_provider.as_deref().unwrap_or("deepseek"));
    let model = req.llm_model.as_deref().unwrap_or(tpl.llm_model.as_deref().unwrap_or("deepseek-chat"));
    let publish = req.publish.unwrap_or(false);
    let word_count = tpl.word_count.unwrap_or(1000);

    let mut posts: Vec<GeneratedPost> = Vec::new();
    let mut failed = 0usize;
    let mut fields = req.field_values.clone();

    for dir_id in &req.directory_ids {
        // Get directory info for merge fields
        let dir_info: Option<(String, String, String)> = sqlx::query_as(
            "SELECT name, slug, COALESCE(city, '') FROM directories WHERE id = $1"
        )
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        let (dir_name, dir_slug, dir_city) = dir_info.unwrap_or_else(|| ("Directory".into(), "directory".into(), String::new()));

        // Auto-fill directory merge fields
        fields.insert("directory_name".to_string(), dir_name.clone());
        fields.insert("directory_url".to_string(), format!("https://{}.{}", dir_slug, s.config.base_domain));
        fields.insert("directory_slug".to_string(), dir_slug.clone());
        fields.insert("city".to_string(), if dir_city.is_empty() { fields.get("city").cloned().unwrap_or_default() } else { dir_city.clone() });

        // Build the prompt using the template
        let filled_template = fill_template(&tpl.content_template, &fields);
        let merge_fields_str = serde_json::to_string(&tpl.merge_fields).unwrap_or_default();

        let prompt = build_llm_prompt(&filled_template, &fields, word_count, provider, &merge_fields_str);

        // Call LLM
        match call_llm(&s.db, provider, model, &prompt).await {
            Ok(generated) => {
                // Parse title from generated content (first h1 or first line)
                let (title, mut content) = extract_title_and_content(&generated, &fields, &tpl.name);

                let date_str = Utc::now().format("%B %d, %Y").to_string();
                let byline = "Admin";
                let meta = format!(r#"<p class="post-meta" style="color:#6b7280;font-size:.9rem;margin-bottom:1.5rem">Posted on {} by {}</p>"#, date_str, byline);
                content = meta + "\n" + &content;

                // Generate slug
                let slug = slugify(&title);

                // Create the blog post
                let post = sqlx::query_as::<_, (Uuid,)>(
                    r#"INSERT INTO blog_posts (title, slug, content, directory_id, published, template_id, template_data, focus_keyword, blog_category, feature_image, feature_video, media_json, scheduled_at)
                       VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8, $9, $10, $11, $12::jsonb, $13)
                       RETURNING id"#
                )
                .bind(&title)
                .bind(&slug)
                .bind(&content)
                .bind(dir_id)
                .bind(publish)
                .bind(req.template_id)
                .bind(json!({"template_name": &tpl.name, "fields": &fields, "provider": provider, "model": model}))
                .bind(fields.get("focus_keyword").cloned().unwrap_or_default())
                .bind(tpl.template_type.as_deref().unwrap_or("article"))
                .bind(fields.get("feature_image").cloned().unwrap_or_default())
                .bind(fields.get("feature_video").cloned().unwrap_or_default())
                .bind(json!([]))
                .fetch_one(&s.db)
                .await;

                match post {
                    Ok((post_id,)) => {
                        // Try to inject media after post is created
                        if tpl.image_provider.as_deref() != Some("none") {
                            let _ = inject_media(&s, post_id, &title, &fields, &tpl).await;
                        }

                        posts.push(GeneratedPost {
                            id: post_id,
                            title,
                            directory_id: *dir_id,
                            slug,
                            status: if publish { "published".to_string() } else { "draft".to_string() },
                            generated_at: Utc::now(),
                        });
                    }
                    Err(e) => {
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                failed += 1;
            }
        }
    }

    let total_generated = posts.len();
    Ok((StatusCode::CREATED, Json(GenerateBlogResponse {
        posts,
        total_generated,
        total_failed: failed,
    })))
}

// ── Regenerate Single Post ──

pub async fn regenerate_blog_post(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let post = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<serde_json::Value>)>(
        "SELECT id, template_id, template_data FROM blog_posts WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("post not found".into()))?;

    let tpl_id = match post.1 {
        Some(tid) => tid,
        None => return Err(AppError::BadRequest("post has no template reference".into())),
    };

    let tpl = sqlx::query_as::<_, BlogTemplate>("SELECT * FROM blog_templates WHERE id = $1")
        .bind(tpl_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("template for this post not found".into()))?;

    let template_data = post.2.unwrap_or(json!({}));
    let fields: HashMap<String, String> = serde_json::from_value(template_data.get("fields").cloned().unwrap_or(json!({}))).unwrap_or_default();
    let provider = template_data.get("provider").and_then(|v| v.as_str()).unwrap_or("deepseek");
    let model = template_data.get("model").and_then(|v| v.as_str()).unwrap_or("deepseek-chat");

    let filled_template = fill_template(&tpl.content_template, &fields);
    let prompt = build_llm_prompt(&filled_template, &fields, tpl.word_count.unwrap_or(1000), provider, "{}");

    match call_llm(&s.db, provider, model, &prompt).await {
        Ok(generated) => {
            let (title, mut content) = extract_title_and_content(&generated, &fields, &tpl.name);

                let date_str = Utc::now().format("%B %d, %Y").to_string();
                let byline = "Admin";
                let meta = format!(r#"<p class="post-meta" style="color:#6b7280;font-size:.9rem;margin-bottom:1.5rem">Posted on {} by {}</p>"#, date_str, byline);
                content = meta + "\n" + &content;
            let slug = slugify(&title);

            sqlx::query(
                "UPDATE blog_posts SET title = $1, slug = $2, content = $3, updated_at = NOW() WHERE id = $4"
            )
            .bind(&title)
            .bind(&slug)
            .bind(&content)
            .bind(id)
            .execute(&s.db)
            .await?;

            Ok(Json(json!({"id": id, "title": title, "slug": slug, "status": "regenerated"})))
        }
        Err(e) => Err(AppError::Internal(format!("LLM call failed: {}", e))),
    }
}

// ── Media Injection ──

async fn inject_media(
    s: &AppState,
    post_id: Uuid,
    _title: &str,
    fields: &HashMap<String, String>,
    tpl: &BlogTemplate,
) -> Result<(), AppError> {
    // Try to find relevant YouTube videos
    let query = fields.get("focus_keyword").cloned().unwrap_or_else(|| "business services".to_string());
    let city = fields.get("city").cloned().unwrap_or_default();

    let search_query = if city.is_empty() {
        format!("{} guide tips", query)
    } else {
        format!("{} {} guide", city, query)
    };

    // Use existing YouTube API if configured, or try to fetch relevant videos
    // For now, use a simple approach — search for free stock image and video
    let image_url = generate_image_url(&search_query);
    let video_url = search_youtube_video(&s, &search_query).await.ok().unwrap_or_default();

    if !image_url.is_empty() || !video_url.is_empty() {
        sqlx::query(
            "UPDATE blog_posts SET feature_image = COALESCE($1, feature_image), feature_video = COALESCE($2, feature_video), updated_at = NOW() WHERE id = $3"
        )
        .bind(if image_url.is_empty() { None } else { Some(&image_url) })
        .bind(if video_url.is_empty() { None } else { Some(&video_url) })
        .bind(post_id)
        .execute(&s.db)
        .await?;

        if !video_url.is_empty() {
            let _ = sqlx::query(
                "INSERT INTO blog_media (blog_post_id, media_type, url, alt_text, source) VALUES ($1, 'video', $2, $3, 'youtube')"
            )
            .bind(post_id)
            .bind(&video_url)
            .bind(&query)
            .execute(&s.db)
            .await;
        }
    }

    Ok(())
}

fn generate_image_url(query: &str) -> String {
    // Placeholder — integrate with Unsplash/Pexels or Gemini image generation
    // Returns empty for now; will be wired up with actual API later
    String::new()
}

async fn search_youtube_video(_s: &AppState, query: &str) -> Result<String, String> {
    // Placeholder — will be wired to YouTube Data API
    // For now, construct a search URL
    let encoded: String = query.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect();
    Ok(format!("https://www.youtube.com/results?search_query={}", encoded.replace(' ', "+")))
}

// ── LLM Integration ──

fn build_llm_prompt(filled_template: &str, fields: &HashMap<String, String>, word_count: i32, provider: &str, _merge_fields_schema: &str) -> String {
    let keyword = fields.get("focus_keyword").cloned().unwrap_or_default();
    let city = fields.get("city").cloned().unwrap_or_default();
    let industry = fields.get("industry").cloned().unwrap_or_else(|| fields.get("service_area").cloned().unwrap_or_default());

    format!(
        r#"You are an expert SEO content writer. Write a high-quality, well-structured blog post in HTML format.

REQUIREMENTS:
- Word count: approximately {} words
- Use proper HTML: <h2>, <h3>, <p>, <ul>, <li>, <strong>
- Include a FAQ section with <h3> questions and answers
- Write in natural, informative tone — not promotional
- Optimize for featured snippets and AI overviews
- Focus keyword: "{}"
- Target location/city: "{}"
- Industry/service: "{}"

TEMPLATE TO FILL:
{}

Generate the complete HTML blog post now. Start with an <h1> title tag."#,
        word_count, keyword, city, industry, filled_template
    )
}

async fn call_llm(db: &sqlx::PgPool, provider: &str, model: &str, prompt: &str) -> Result<String, AppError> {
    match provider {
        "deepseek" => call_deepseek(db, model, prompt).await,
        "openai" => call_openai(db, model, prompt).await,
        "gemini" => call_gemini(db, model, prompt).await,
        _ => call_deepseek(db, model, prompt).await,
    }
}

async fn call_deepseek(db: &sqlx::PgPool, model: &str, prompt: &str) -> Result<String, AppError> {
    let api_key = fetch_provider_key(db, "deepseek").await
        .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
        .ok_or_else(|| AppError::Internal("DeepSeek API key not configured. Add it in Provider Keys.".into()))?;
    let url = "https://api.deepseek.com/v1/chat/completions";

    let body = json!({
        "model": if model.contains('/') { "deepseek-chat" } else { model },
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.7,
        "max_tokens": 4096,
    });

    let client = reqwest::Client::new();
    let resp = client.post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("DeepSeek request failed: {}", e)))?;

    let data: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("DeepSeek parse failed: {}", e)))?;

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| AppError::Internal("DeepSeek returned no content".into()))?
        .to_string();

    Ok(content)
}

async fn call_openai(db: &sqlx::PgPool, model: &str, prompt: &str) -> Result<String, AppError> {
    let api_key = fetch_provider_key(db, "openai").await
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .ok_or_else(|| AppError::Internal("OpenAI API key not configured. Add it in Provider Keys.".into()))?;
    let url = "https://api.openai.com/v1/chat/completions";

    let body = json!({
        "model": if model.is_empty() { "gpt-4o" } else { model },
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.7,
        "max_tokens": 4096,
    });

    let client = reqwest::Client::new();
    let resp = client.post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("OpenAI request failed: {}", e)))?;

    let data: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Internal(format!("OpenAI parse failed: {}", e)))?;

    let content = data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| AppError::Internal("OpenAI returned no content".into()))?
        .to_string();

    Ok(content)
}

async fn call_gemini(db: &sqlx::PgPool, _model: &str, _prompt: &str) -> Result<String, AppError> {
    // Gemini integration placeholder — will use Google AI API
    // For now, forward to DeepSeek as fallback
    call_deepseek(db, "deepseek-chat", _prompt).await
}

// ── Helpers ──

fn fill_template(template: &str, fields: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in fields {
        let placeholder = format!("{{{}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

fn extract_title_and_content(generated: &str, fields: &HashMap<String, String>, template_name: &str) -> (String, String) {
    // Try to extract <h1> title
    if let Some(start) = generated.find("<h1") {
        if let Some(h1_start) = generated[start..].find('>') {
            let content_start = start + h1_start + 1;
            if let Some(h1_end) = generated[content_start..].find("</h1>") {
                let title = generated[content_start..content_start + h1_end].trim().to_string();
                if !title.is_empty() {
                    return (title, generated.to_string());
                }
            }
        }
    }

    // Fallback: use the template name with city context
    let city = fields.get("city").cloned().unwrap_or_default();
    let industry = fields.get("industry").cloned().unwrap_or_else(|| fields.get("service_area").cloned().unwrap_or_default());

    let title = if city.is_empty() {
        format!("{} - Expert Guide", template_name)
    } else {
        format!("{} in {} - Expert Guide", template_name, city)
    };

    (title, generated.to_string())
}

fn slugify(s: &str) -> String {
    let slug: String = s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();

    let slug = slug.split_whitespace()
        .collect::<Vec<_>>()
        .join("-");

    // Remove consecutive dashes
    let mut result = String::new();
    let mut prev_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_dash { result.push(c); }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }

    result.trim_matches('-').to_string()
}

/// Fetch the first active API key for a provider from the provider_keys table.
async fn fetch_provider_key(db: &sqlx::PgPool, provider: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT api_key FROM provider_keys WHERE provider = $1 AND is_active = true LIMIT 1"
    )
    .bind(provider)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

// ── Shared public helpers used by blog_qa ──

/// Call LLM and return parsed JSON array. Used by blog_qa for keyword generation.
pub async fn call_llm_json(
    db: &sqlx::PgPool,
    _config: &crate::config::AppConfig,
    prompt: &str,
    provider_config: Option<&serde_json::Value>,
) -> Result<Vec<serde_json::Value>, String> {
    let (api_key, model) = if let Some(pc) = provider_config {
        let key = pc.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
        let mdl = pc.get("model").and_then(|v| v.as_str()).unwrap_or("gpt-4o-mini");
        (key.to_string(), mdl.to_string())
    } else {
        let mut key = fetch_provider_key(db, "openai").await;
        if key.is_none() {
            key = fetch_provider_key(db, "deepseek").await;
        }
        let k = key.unwrap_or_default();
        let mdl = "deepseek-chat".to_string();
        (k, mdl)
    };

    let client = reqwest::Client::new();
    let url = if model.contains("deepseek") {
        "https://api.deepseek.com/chat/completions"
    } else if model.contains("gemini") {
        "https://generativelanguage.googleapis.com/v1/models/gemini-pro:generateContent"
    } else {
        "https://api.openai.com/v1/chat/completions"
    };

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.7,
        "max_tokens": 4096
    });

    let resp = client.post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI request failed: {}", e))?;

    let json: serde_json::Value = resp.json().await
        .map_err(|e| format!("Failed to parse AI response: {}", e))?;

    // Extract content from response
    let content = json.pointer("/choices/0/message/content")
        .or_else(|| json.pointer("/candidates/0/content/parts/0/text"))
        .and_then(|v| v.as_str())
        .unwrap_or("[]");

    // Try to parse as JSON array, stripping markdown fences if needed
    let cleaned = content.trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    serde_json::from_str::<Vec<serde_json::Value>>(cleaned)
        .map_err(|e| format!("Failed to parse AI JSON output: {} — content: {}", e, cleaned.chars().take(200).collect::<String>()))
}

/// Generate blog content from a prompt. Used by blog_qa for post generation.
pub async fn generate_blog_content(
    db: &sqlx::PgPool,
    _config: &crate::config::AppConfig,
    prompt: &str,
) -> Result<String, String> {
    let mut api_key = fetch_provider_key(db, "openai").await;
    if api_key.is_none() {
        api_key = fetch_provider_key(db, "deepseek").await;
    }
    let api_key = api_key.unwrap_or_default();
    let model = "deepseek-chat".to_string();

    let client = reqwest::Client::new();
    let url = if model.contains("deepseek") {
        "https://api.deepseek.com/chat/completions"
    } else if model.contains("gemini") {
        "https://generativelanguage.googleapis.com/v1/models/gemini-pro:generateContent"
    } else {
        "https://api.openai.com/v1/chat/completions"
    };

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.7,
        "max_tokens": 4096
    });

    let resp = client.post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Blog content request failed: {}", e))?;

    let json: serde_json::Value = resp.json().await
        .map_err(|e| format!("Failed to parse blog content response: {}", e))?;

    let content = json.pointer("/choices/0/message/content")
        .or_else(|| json.pointer("/candidates/0/content/parts/0/text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(content.to_string())
}
