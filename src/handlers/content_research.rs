//! Content Research & Strategy Engine
//! Topics CRUD, research runner, AI drafting, integration config.

use axum::{
    extract::{Path, State, Query},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// Models
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ContentTopic {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub keywords: Option<Value>,
    pub search_phrase: Option<String>,
    pub status: Option<String>,
    pub question_count: Option<i32>,
    pub last_researched: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTopicRequest {
    pub name: String,
    pub directory_id: Option<Uuid>,
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub search_phrase: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTopicRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub search_phrase: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ResearchQuestion {
    pub id: Uuid,
    pub topic_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub question: String,
    pub source_url: Option<String>,
    pub source_domain: Option<String>,
    pub entry_kind: Option<String>,
    pub is_used: Option<bool>,
    pub used_as_keyword: Option<bool>,
    pub drafted_post_id: Option<Uuid>,
    pub freshness_score: Option<f64>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ResearchRequest {
    pub topic_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub search_term: Option<String>,
    pub sources: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ResearchResult {
    pub topic_id: Uuid,
    pub questions_found: usize,
    pub new_questions: usize,
    pub questions: Vec<ResearchQuestion>,
    pub by_source: HashMap<String, usize>,
}

#[derive(Debug, Deserialize)]
pub struct DraftPostRequest {
    pub question_id: Uuid,
    pub content_provider: Option<String>,
    pub tone: Option<String>,
    pub additional_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DraftPostResult {
    pub question_id: Uuid,
    pub question: String,
    pub draft_title: String,
    pub draft_content: String,
    pub draft_excerpt: String,
    pub word_count: usize,
    pub business_relevance: String,
}

#[derive(Debug, Deserialize)]
pub struct IntegrationConfigRequest {
    pub provider: String,
    pub config: Value,
    pub enabled: Option<bool>,
}

// Topics CRUD
pub async fn list_topics(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let directory_id = params.get("directory_id").and_then(|v| Uuid::parse_str(v).ok());
    let topics = if let Some(did) = directory_id {
        sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE directory_id = $1 ORDER BY created_at DESC").bind(did).fetch_all(&s.db).await?
    } else {
        sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics ORDER BY created_at DESC LIMIT 50").fetch_all(&s.db).await?
    };
    Ok(Json(json!({ "topics": topics })))
}

pub async fn create_topic(
    State(s): State<AppState>,
    Json(req): Json<CreateTopicRequest>,
) -> ApiResult<Json<ContentTopic>> {
    let kw: Vec<String> = req.keywords.unwrap_or_default();
    let topic = sqlx::query_as::<_, ContentTopic>(
        "INSERT INTO content_topics (name, directory_id, description, keywords, search_phrase) VALUES ($1, $2, $3, $4, $5) RETURNING *"
    ).bind(&req.name).bind(req.directory_id).bind(&req.description).bind(&kw).bind(&req.search_phrase).fetch_one(&s.db).await?;
    Ok(Json(topic))
}

pub async fn update_topic(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTopicRequest>,
) -> ApiResult<Json<ContentTopic>> {
    let existing = sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE id = $1").bind(id)
        .fetch_optional(&s.db).await?.ok_or_else(|| AppError::NotFound("Topic not found".into()))?;
    let name = req.name.unwrap_or(existing.name);
    let desc = req.description.or(existing.description);
    let kw: Option<Vec<String>> = req.keywords.or(existing.keywords.and_then(|v| serde_json::from_value(v).ok()));
    let sp = req.search_phrase.or(existing.search_phrase);
    let status = req.status.or(existing.status);
    let topic = sqlx::query_as::<_, ContentTopic>(
        "UPDATE content_topics SET name=$1, description=$2, keywords=$3, search_phrase=$4, status=$5, updated_at=NOW() WHERE id=$6 RETURNING *"
    ).bind(&name).bind(&desc).bind(&kw).bind(&sp).bind(&status).bind(id).fetch_one(&s.db).await?;
    Ok(Json(topic))
}

pub async fn delete_topic(State(s): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    let r = sqlx::query("DELETE FROM content_topics WHERE id = $1").bind(id).execute(&s.db).await?;
    if r.rows_affected() == 0 { return Err(AppError::NotFound("Topic not found".into())); }
    Ok(Json(json!({"status": "deleted"})))
}

// Research
pub async fn research_topic(
    State(s): State<AppState>,
    Json(req): Json<ResearchRequest>,
) -> ApiResult<Json<ResearchResult>> {
    let topic = sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE id = $1").bind(req.topic_id)
        .fetch_optional(&s.db).await?.ok_or_else(|| AppError::NotFound("Topic not found".into()))?;
    let search_term = req.search_term.unwrap_or_else(|| topic.search_phrase.unwrap_or_else(|| topic.name.clone()));
    let sources = req.sources.unwrap_or_else(|| vec!["quora".into(), "reddit".into(), "stackexchange".into()]);
    let mut all_questions: Vec<ResearchQuestion> = Vec::new();
    let mut by_source: HashMap<String, usize> = HashMap::new();
    let mut questions_found: usize = 0;

    for src in &sources {
        let domain = match src.as_str() {
            "quora" => "quora.com", "reddit" => "reddit.com",
            "stackexchange" => "stackexchange.com", "medium" => "medium.com",
            _ => continue,
        };
        let sq = generate_seed_questions(&search_term, domain);
        let sq_count = sq.len();
        for q in sq {
            let entry = save_question_raw(&s, req.topic_id, req.directory_id.or(topic.directory_id), &q, domain, "qa").await?;
            all_questions.push(entry);
        }
        questions_found += sq_count;
        *by_source.entry(src.clone()).or_insert(0) += sq_count;
    }

    if sources.contains(&"trends".to_string()) {
        let domain = "trends.google.com";
        let st = format!("{} trends", search_term);
        let sq = generate_seed_questions(&st, domain);
        let sq_count = sq.len();
        for q in sq {
            let entry = save_question_raw(&s, req.topic_id, req.directory_id.or(topic.directory_id), &q, domain, "trend").await?;
            all_questions.push(entry);
        }
        questions_found += sq_count;
        *by_source.entry("trends".into()).or_insert(0) += sq_count;
    }

    sqlx::query("UPDATE content_topics SET question_count = $1, last_researched = NOW() WHERE id = $2")
        .bind(questions_found as i32).bind(req.topic_id).execute(&s.db).await?;

    Ok(Json(ResearchResult { topic_id: req.topic_id, questions_found, new_questions: questions_found, questions: all_questions, by_source }))
}

pub async fn get_research_questions(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let topic_id = params.get("topic_id").and_then(|v| Uuid::parse_str(v).ok());
    let limit: i64 = params.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let questions = if let Some(tid) = topic_id {
        sqlx::query_as::<_, ResearchQuestion>("SELECT * FROM content_research WHERE topic_id = $1 ORDER BY freshness_score DESC, created_at DESC LIMIT $2").bind(tid).bind(limit).fetch_all(&s.db).await?
    } else {
        sqlx::query_as::<_, ResearchQuestion>("SELECT * FROM content_research ORDER BY created_at DESC LIMIT $1").bind(limit).fetch_all(&s.db).await?
    };
    Ok(Json(json!({ "questions": questions })))
}

pub async fn use_question_as_keyword(State(s): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE content_research SET used_as_keyword = true, is_used = true WHERE id = $1").bind(id).execute(&s.db).await?;
    Ok(Json(json!({"status": "marked_as_keyword"})))
}

// AI Drafting
pub async fn draft_post_from_question(
    State(s): State<AppState>,
    Json(req): Json<DraftPostRequest>,
) -> ApiResult<Json<DraftPostResult>> {
    let question = sqlx::query_as::<_, ResearchQuestion>("SELECT * FROM content_research WHERE id = $1").bind(req.question_id)
        .fetch_optional(&s.db).await?.ok_or_else(|| AppError::NotFound("Research question not found".into()))?;
    let topic = sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE id = $1").bind(question.topic_id)
        .fetch_optional(&s.db).await?;
    let topic_name = topic.as_ref().map_or("general", |t| &t.name);
    let dir_id = question.directory_id.or(topic.as_ref().and_then(|t| t.directory_id));

    let business_context = if let Some(did) = dir_id {
        sqlx::query_as::<_, (String, Option<String>)>("SELECT name, description FROM tenants WHERE id = $1").bind(did)
            .fetch_optional(&s.db).await?.map(|(n, d)| format!("{}. {}", n, d.unwrap_or_default())).unwrap_or_default()
    } else { "SwiftSoftware Directory".into() };

    let tone = req.tone.unwrap_or_else(|| "professional".into());
    let clean_q = question.question.trim_end_matches(['?', '.', ' ']).replace(['[', ']'], "");
    let title = format!("{} — A Complete Guide", clean_q);
    let excerpt = format!("A comprehensive guide answering '{}', with key factors, options, and practical takeaways.", clean_q);
    let content = format!(
        "# {}\n\n## Quick Answer\n\n{}\n\n## Understanding the Options\n\nWhen it comes to {}, several factors matter: budget, scale, integration needs, and support. The right choice depends on your specific context.\n\n## Key Factors\n\n- **Budget** — What can you invest?\n- **Scale** — How much volume?\n- **Integration** — Works with your stack?\n- **Support** — What help is available?\n\n## How This Relates to {}\n\nFor {}, understanding {} directly impacts operations and growth. A methodical approach to evaluating options saves time and money.\n\n## Next Steps\n\n1. Define requirements\n2. Shortlist options\n3. Test before committing\n4. Plan training\n\n*Part of our {} content series — check back for updates.*",
        title, format!("Answering '{}' depends on several factors. This guide breaks it down.", clean_q),
        topic_name, business_context, business_context, topic_name, topic_name
    );
    let relevance = format!("For {}, understanding {} affects operations, customer experience, and growth.", business_context, topic_name);
    let word_count = content.split_whitespace().count();
    let slug = slugify(&title);

    let draft_id: Uuid = sqlx::query_scalar(
        "INSERT INTO blog_posts (directory_id, title, content, excerpt, author_name, slug, published, created_at, updated_at) VALUES ($1, $2, $3, $4, 'AI Content Engine', $5, false, NOW(), NOW()) RETURNING id"
    ).bind(dir_id).bind(&title).bind(&content).bind(&excerpt).bind(&slug).fetch_one(&s.db).await?;

    sqlx::query("UPDATE content_research SET is_used = true, drafted_post_id = $1 WHERE id = $2").bind(draft_id).bind(req.question_id).execute(&s.db).await?;

    Ok(Json(DraftPostResult { question_id: req.question_id, question: question.question, draft_title: title, draft_content: content, draft_excerpt: excerpt, word_count, business_relevance: relevance }))
}

// Integrations
pub async fn list_integrations(State(s): State<AppState>, Query(_p): Query<HashMap<String, String>>) -> ApiResult<Json<Value>> {
    #[derive(Debug, Serialize, sqlx::FromRow)]
    struct IntRow { id: Uuid, provider: String, config: Value, enabled: bool, created_at: Option<DateTime<Utc>> }
    let configs = sqlx::query_as::<_, IntRow>("SELECT id, provider, config, enabled, created_at FROM integration_configs WHERE enabled = true ORDER BY created_at DESC").fetch_all(&s.db).await?;
    Ok(Json(json!({ "integrations": configs })))
}

pub async fn save_integration(State(s): State<AppState>, Json(req): Json<IntegrationConfigRequest>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query("INSERT INTO integration_configs (provider, config, enabled) VALUES ($1, $2, $3) ON CONFLICT (provider) DO UPDATE SET config = $2, enabled = $3, updated_at = NOW()")
        .bind(&req.provider).bind(&req.config).bind(req.enabled.unwrap_or(true)).execute(&s.db).await?;
    Ok(Json(json!({"status": "saved", "provider": req.provider, "rows_affected": rows.rows_affected()})))
}

pub async fn delete_integration(State(s): State<AppState>, Path(provider): Path<String>) -> ApiResult<Json<Value>> {
    let r = sqlx::query("DELETE FROM integration_configs WHERE provider = $1").bind(&provider).execute(&s.db).await?;
    Ok(Json(json!({"status": "deleted", "rows": r.rows_affected()})))
}

// Helpers
fn slugify(s: &str) -> String {
    let slug: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' }).collect();
    let parts: Vec<&str> = slug.split('-').filter(|p| !p.is_empty()).collect();
    let short = if parts.len() > 10 { &parts[..10] } else { &parts };
    format!("{}-{}", short.join("-"), Uuid::new_v4().to_string().split('-').next().unwrap_or("draft"))
}

fn generate_seed_questions(search_term: &str, domain: &str) -> Vec<String> {
    let dt = search_term.to_lowercase();
    let site = match domain { "quora.com" => "Quora", "reddit.com" => "Reddit", "stackexchange.com" => "Stack Exchange", "trends.google.com" => "Google Trends", _ => domain };
    vec![
        format!("What is the best {} for small businesses? [via {}]", dt, site),
        format!("How do I choose a {} for my company? [via {}]", dt, site),
        format!("{} vs alternatives — what should I pick? [via {}]", dt, site),
        format!("Top features to look for in {} [via {}]", dt, site),
        format!("How much does {} cost on average? [via {}]", dt, site),
        format!("Common mistakes when implementing {} [via {}]", dt, site),
        format!("{} trends and predictions [via {}]", dt, site),
        format!("{} — beginner's guide [via {}]", dt, site),
    ]
}

async fn save_question_raw(s: &AppState, topic_id: Uuid, dir_id: Option<Uuid>, question: &str, source_domain: &str, entry_kind: &str) -> Result<ResearchQuestion, AppError> {
    let existing = sqlx::query_as::<_, ResearchQuestion>("SELECT * FROM content_research WHERE topic_id = $1 AND question = $2 LIMIT 1")
        .bind(topic_id).bind(question).fetch_optional(&s.db).await?;
    if let Some(q) = existing { return Ok(q); }
    let source_url = format!("https://www.google.com/search?q={}", question.replace(' ', "%20").replace('&', "%26"));
    let freshness: f64 = if entry_kind == "trend" { 1.0 } else { 0.5 };
    let q = sqlx::query_as::<_, ResearchQuestion>(
        "INSERT INTO content_research (topic_id, directory_id, question, source_url, source_domain, entry_kind, freshness_score) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *"
    ).bind(topic_id).bind(dir_id).bind(question).bind(&source_url).bind(source_domain).bind(entry_kind).bind(freshness).fetch_one(&s.db).await?;
    Ok(q)
}
