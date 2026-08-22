//! Content Research & Strategy Engine
//! Topics CRUD, research runner, AI drafting, integration config.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

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
    let directory_id = params
        .get("directory_id")
        .and_then(|v| Uuid::parse_str(v).ok());
    let topics = if let Some(did) = directory_id {
        sqlx::query_as::<_, ContentTopic>(
            "SELECT * FROM content_topics WHERE directory_id = $1 ORDER BY created_at DESC",
        )
        .bind(did)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, ContentTopic>(
            "SELECT * FROM content_topics ORDER BY created_at DESC LIMIT 50",
        )
        .fetch_all(&s.db)
        .await?
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
    let existing = sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Topic not found".into()))?;
    let name = req.name.unwrap_or(existing.name);
    let desc = req.description.or(existing.description);
    let kw: Option<Vec<String>> = req.keywords.or(existing
        .keywords
        .and_then(|v| serde_json::from_value(v).ok()));
    let sp = req.search_phrase.or(existing.search_phrase);
    let status = req.status.or(existing.status);
    let topic = sqlx::query_as::<_, ContentTopic>(
        "UPDATE content_topics SET name=$1, description=$2, keywords=$3, search_phrase=$4, status=$5, updated_at=NOW() WHERE id=$6 RETURNING *"
    ).bind(&name).bind(&desc).bind(&kw).bind(&sp).bind(&status).bind(id).fetch_one(&s.db).await?;
    Ok(Json(topic))
}

pub async fn delete_topic(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let r = sqlx::query("DELETE FROM content_topics WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("Topic not found".into()));
    }
    Ok(Json(json!({"status": "deleted"})))
}

// Research
pub async fn research_topic(
    State(s): State<AppState>,
    Json(req): Json<ResearchRequest>,
) -> ApiResult<Json<ResearchResult>> {
    let topic = sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE id = $1")
        .bind(req.topic_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Topic not found".into()))?;
    let search_term = req
        .search_term
        .unwrap_or_else(|| topic.search_phrase.unwrap_or_else(|| topic.name.clone()));
    let sources = req
        .sources
        .unwrap_or_else(|| vec!["quora".into(), "reddit".into(), "stackexchange".into()]);
    let mut all_questions: Vec<ResearchQuestion> = Vec::new();
    let mut by_source: HashMap<String, usize> = HashMap::new();
    let mut questions_found: usize = 0;

    for src in &sources {
        let domain = match src.as_str() {
            "quora" => "quora.com",
            "reddit" => "reddit.com",
            "stackexchange" => "stackexchange.com",
            "medium" => "medium.com",
            _ => continue,
        };
        let sq = generate_seed_questions(&search_term, domain);
        let sq_count = sq.len();
        for q in sq {
            let entry = save_question_raw(
                &s,
                req.topic_id,
                req.directory_id.or(topic.directory_id),
                &q,
                domain,
                "qa",
            )
            .await?;
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
            let entry = save_question_raw(
                &s,
                req.topic_id,
                req.directory_id.or(topic.directory_id),
                &q,
                domain,
                "trend",
            )
            .await?;
            all_questions.push(entry);
        }
        questions_found += sq_count;
        *by_source.entry("trends".into()).or_insert(0) += sq_count;
    }

    sqlx::query(
        "UPDATE content_topics SET question_count = $1, last_researched = NOW() WHERE id = $2",
    )
    .bind(questions_found as i32)
    .bind(req.topic_id)
    .execute(&s.db)
    .await?;

    Ok(Json(ResearchResult {
        topic_id: req.topic_id,
        questions_found,
        new_questions: questions_found,
        questions: all_questions,
        by_source,
    }))
}

pub async fn get_research_questions(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let topic_id = params.get("topic_id").and_then(|v| Uuid::parse_str(v).ok());
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let questions = if let Some(tid) = topic_id {
        sqlx::query_as::<_, ResearchQuestion>("SELECT * FROM content_research WHERE topic_id = $1 ORDER BY freshness_score DESC, created_at DESC LIMIT $2").bind(tid).bind(limit).fetch_all(&s.db).await?
    } else {
        sqlx::query_as::<_, ResearchQuestion>(
            "SELECT * FROM content_research ORDER BY created_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&s.db)
        .await?
    };
    Ok(Json(json!({ "questions": questions })))
}

pub async fn use_question_as_keyword(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE content_research SET used_as_keyword = true, is_used = true WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;
    Ok(Json(json!({"status": "marked_as_keyword"})))
}

// AI Drafting
pub async fn draft_post_from_question(
    State(s): State<AppState>,
    Json(req): Json<DraftPostRequest>,
) -> ApiResult<Json<DraftPostResult>> {
    let question =
        sqlx::query_as::<_, ResearchQuestion>("SELECT * FROM content_research WHERE id = $1")
            .bind(req.question_id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Research question not found".into()))?;
    let topic = sqlx::query_as::<_, ContentTopic>("SELECT * FROM content_topics WHERE id = $1")
        .bind(question.topic_id)
        .fetch_optional(&s.db)
        .await?;
    let topic_name = topic.as_ref().map_or("general", |t| &t.name);
    let dir_id = question
        .directory_id
        .or(topic.as_ref().and_then(|t| t.directory_id));

    let business_context = if let Some(did) = dir_id {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT name, description FROM tenants WHERE id = $1",
        )
        .bind(did)
        .fetch_optional(&s.db)
        .await?
        .map(|(n, d)| format!("{}. {}", n, d.unwrap_or_default()))
        .unwrap_or_default()
    } else {
        "SwiftSoftware Directory".into()
    };

    let tone = req.tone.unwrap_or_else(|| "professional".into());
    let clean_q = question
        .question
        .trim_end_matches(['?', '.', ' '])
        .replace(['[', ']'], "");
    let title = format!("{} — A Complete Guide", clean_q);
    let excerpt = format!(
        "A comprehensive guide answering '{}', with key factors, options, and practical takeaways.",
        clean_q
    );
    let content = format!(
        "# {}\n\n## Quick Answer\n\n{}\n\n## Understanding the Options\n\nWhen it comes to {}, several factors matter: budget, scale, integration needs, and support. The right choice depends on your specific context.\n\n## Key Factors\n\n- **Budget** — What can you invest?\n- **Scale** — How much volume?\n- **Integration** — Works with your stack?\n- **Support** — What help is available?\n\n## How This Relates to {}\n\nFor {}, understanding {} directly impacts operations and growth. A methodical approach to evaluating options saves time and money.\n\n## Next Steps\n\n1. Define requirements\n2. Shortlist options\n3. Test before committing\n4. Plan training\n\n*Part of our {} content series — check back for updates.*",
        title, format!("Answering '{}' depends on several factors. This guide breaks it down.", clean_q),
        topic_name, business_context, business_context, topic_name, topic_name
    );
    let relevance = format!(
        "For {}, understanding {} affects operations, customer experience, and growth.",
        business_context, topic_name
    );
    let word_count = content.split_whitespace().count();
    let slug = slugify(&title);

    let draft_id: Uuid = sqlx::query_scalar(
        "INSERT INTO blog_posts (directory_id, title, content, excerpt, author_name, slug, published, created_at, updated_at) VALUES ($1, $2, $3, $4, 'AI Content Engine', $5, false, NOW(), NOW()) RETURNING id"
    ).bind(dir_id).bind(&title).bind(&content).bind(&excerpt).bind(&slug).fetch_one(&s.db).await?;

    sqlx::query("UPDATE content_research SET is_used = true, drafted_post_id = $1 WHERE id = $2")
        .bind(draft_id)
        .bind(req.question_id)
        .execute(&s.db)
        .await?;

    Ok(Json(DraftPostResult {
        question_id: req.question_id,
        question: question.question,
        draft_title: title,
        draft_content: content,
        draft_excerpt: excerpt,
        word_count,
        business_relevance: relevance,
    }))
}

// Integrations
pub async fn list_integrations(
    State(s): State<AppState>,
    Query(_p): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    #[derive(Debug, Serialize, sqlx::FromRow)]
    struct IntRow {
        id: Uuid,
        provider: String,
        config: Value,
        enabled: bool,
        created_at: Option<DateTime<Utc>>,
    }
    let configs = sqlx::query_as::<_, IntRow>("SELECT id, provider, config, enabled, created_at FROM integration_configs WHERE enabled = true ORDER BY created_at DESC").fetch_all(&s.db).await?;
    Ok(Json(json!({ "integrations": configs })))
}

pub async fn save_integration(
    State(s): State<AppState>,
    Json(req): Json<IntegrationConfigRequest>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query("INSERT INTO integration_configs (provider, config, enabled) VALUES ($1, $2, $3) ON CONFLICT (provider) DO UPDATE SET config = $2, enabled = $3, updated_at = NOW()")
        .bind(&req.provider).bind(&req.config).bind(req.enabled.unwrap_or(true)).execute(&s.db).await?;
    Ok(Json(
        json!({"status": "saved", "provider": req.provider, "rows_affected": rows.rows_affected()}),
    ))
}

pub async fn delete_integration(
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<Json<Value>> {
    let r = sqlx::query("DELETE FROM integration_configs WHERE provider = $1")
        .bind(&provider)
        .execute(&s.db)
        .await?;
    Ok(Json(
        json!({"status": "deleted", "rows": r.rows_affected()}),
    ))
}

// Helpers
fn slugify(s: &str) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let parts: Vec<&str> = slug.split('-').filter(|p| !p.is_empty()).collect();
    let short = if parts.len() > 10 {
        &parts[..10]
    } else {
        &parts
    };
    format!(
        "{}-{}",
        short.join("-"),
        Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("draft")
    )
}

fn generate_seed_questions(search_term: &str, domain: &str) -> Vec<String> {
    let dt = search_term.to_lowercase();
    let site = match domain {
        "quora.com" => "Quora",
        "reddit.com" => "Reddit",
        "stackexchange.com" => "Stack Exchange",
        "trends.google.com" => "Google Trends",
        _ => domain,
    };
    vec![
        format!(
            "What is the best {} for small businesses? [via {}]",
            dt, site
        ),
        format!("How do I choose a {} for my company? [via {}]", dt, site),
        format!(
            "{} vs alternatives — what should I pick? [via {}]",
            dt, site
        ),
        format!("Top features to look for in {} [via {}]", dt, site),
        format!("How much does {} cost on average? [via {}]", dt, site),
        format!("Common mistakes when implementing {} [via {}]", dt, site),
        format!("{} trends and predictions [via {}]", dt, site),
        format!("{} — beginner's guide [via {}]", dt, site),
    ]
}

async fn save_question_raw(
    s: &AppState,
    topic_id: Uuid,
    dir_id: Option<Uuid>,
    question: &str,
    source_domain: &str,
    entry_kind: &str,
) -> Result<ResearchQuestion, AppError> {
    let existing = sqlx::query_as::<_, ResearchQuestion>(
        "SELECT * FROM content_research WHERE topic_id = $1 AND question = $2 LIMIT 1",
    )
    .bind(topic_id)
    .bind(question)
    .fetch_optional(&s.db)
    .await?;
    if let Some(q) = existing {
        return Ok(q);
    }
    let source_url = format!(
        "https://www.google.com/search?q={}",
        question.replace(' ', "%20").replace('&', "%26")
    );
    let freshness: f64 = if entry_kind == "trend" { 1.0 } else { 0.5 };
    let q = sqlx::query_as::<_, ResearchQuestion>(
        "INSERT INTO content_research (topic_id, directory_id, question, source_url, source_domain, entry_kind, freshness_score) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *"
    ).bind(topic_id).bind(dir_id).bind(question).bind(&source_url).bind(source_domain).bind(entry_kind).bind(freshness).fetch_one(&s.db).await?;
    Ok(q)
}
// ── Bulk Research ──

#[derive(Debug, Deserialize)]
pub struct BulkResearchRequest {
    pub topics: Vec<BulkResearchTopic>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct BulkResearchTopic {
    pub name: String,
    pub keywords: Option<Vec<String>>,
    pub search_phrase: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkResearchWithCitiesRequest {
    pub topics: Vec<BulkResearchTopic>,
    pub cities: Vec<String>,
    pub directory_id: Option<Uuid>,
    pub sources: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct BulkResearchResult {
    pub topics_created: usize,
    pub topics_existing: usize,
    pub questions_generated: usize,
    pub questions_skipped: usize,
    pub total_combinations: usize,
    pub by_topic: Vec<TopicResearchSummary>,
}

#[derive(Debug, Serialize)]
pub struct TopicResearchSummary {
    pub topic_name: String,
    pub topic_id: Uuid,
    pub search_phrase_used: String,
    pub questions_found: usize,
    pub by_source: HashMap<String, usize>,
    pub city_variants: usize,
}

#[derive(Debug, Serialize)]
pub struct BulkPreviewResult {
    pub estimated_topics: usize,
    pub estimated_combinations: usize,
    pub estimated_questions: usize,
    pub sample_queries: Vec<String>,
}

/// POST /api/v1/research/bulk
///
/// Takes multiple topics + optional cities, creates topics and runs research for every
/// topic × city combination. Same cross-product pattern as trap_doors.
pub async fn bulk_research(
    State(s): State<AppState>,
    Json(req): Json<BulkResearchWithCitiesRequest>,
) -> ApiResult<Json<BulkResearchResult>> {
    if req.topics.is_empty() {
        return Err(AppError::Validation(
            "At least one topic is required".into(),
        ));
    }
    if req.topics.len() > 50 {
        return Err(AppError::Validation(
            "Maximum 50 topics per bulk request".into(),
        ));
    }
    if req.cities.len() > 200 {
        return Err(AppError::Validation(
            "Maximum 200 cities per bulk request".into(),
        ));
    }

    let sources = req
        .sources
        .unwrap_or_else(|| vec!["quora".into(), "reddit".into(), "stackexchange".into()]);
    let mut result = BulkResearchResult {
        topics_created: 0,
        topics_existing: 0,
        questions_generated: 0,
        questions_skipped: 0,
        total_combinations: 0,
        by_topic: Vec::new(),
    };

    for topic_req in &req.topics {
        let base_search = topic_req
            .search_phrase
            .clone()
            .unwrap_or_else(|| topic_req.name.clone());
        let kw: Vec<String> = topic_req.keywords.clone().unwrap_or_default();

        // Create or find the base topic
        let topic_id = match sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM content_topics WHERE name = $1 AND directory_id IS NOT DISTINCT FROM $2 LIMIT 1"
        ).bind(&topic_req.name).bind(req.directory_id).fetch_optional(&s.db).await? {
            Some(id) => { result.topics_existing += 1; id }
            None => {
                let id = sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO content_topics (name, directory_id, description, keywords, search_phrase) VALUES ($1, $2, $3, $4, $5) RETURNING id"
                ).bind(&topic_req.name).bind(req.directory_id).bind(&topic_req.description).bind(&kw).bind(&base_search).fetch_one(&s.db).await?;
                result.topics_created += 1;
                id
            }
        };

        let mut topic_qs = 0usize;
        let mut by_source: HashMap<String, usize> = HashMap::new();
        let city_count = if req.cities.is_empty() {
            1usize
        } else {
            req.cities.len()
        };

        // For each city, generate city-variant search phrases and research them
        let search_variants: Vec<String> = if req.cities.is_empty() {
            vec![base_search.clone()]
        } else {
            req.cities
                .iter()
                .map(|city| format!("{} in {}", base_search, city))
                .collect()
        };

        for variant in &search_variants {
            for src in &sources {
                let domain = match src.as_str() {
                    "quora" => "quora.com",
                    "reddit" => "reddit.com",
                    "stackexchange" => "stackexchange.com",
                    "medium" => "medium.com",
                    _ => continue,
                };
                let questions = generate_seed_questions(variant, domain);
                let src_count = questions.len();
                for q in questions {
                    match save_question_raw(&s, topic_id, req.directory_id, &q, domain, "qa").await
                    {
                        Ok(_) => {
                            result.questions_generated += 1;
                            topic_qs += 1;
                        }
                        Err(_) => {
                            result.questions_skipped += 1;
                        }
                    }
                }
                *by_source.entry(src.clone()).or_insert(0) += src_count;
            }

            // Google Trends variant
            let trend_domain = "trends.google.com";
            let trend_search = format!("{} trends {}", variant, chrono::Utc::now().format("%Y"));
            let trend_qs = generate_seed_questions(&trend_search, trend_domain);
            let trend_count = trend_qs.len();
            for q in trend_qs {
                match save_question_raw(&s, topic_id, req.directory_id, &q, trend_domain, "trend")
                    .await
                {
                    Ok(_) => {
                        result.questions_generated += 1;
                        topic_qs += 1;
                    }
                    Err(_) => {
                        result.questions_skipped += 1;
                    }
                }
            }
            *by_source.entry("trends".into()).or_insert(0) += trend_count;
        }

        sqlx::query("UPDATE content_topics SET question_count = $1, last_researched = NOW(), updated_at = NOW() WHERE id = $2")
            .bind(topic_qs as i32).bind(topic_id).execute(&s.db).await?;

        result.total_combinations += search_variants.len();
        result.by_topic.push(TopicResearchSummary {
            topic_name: topic_req.name.clone(),
            topic_id,
            search_phrase_used: base_search.clone(),
            questions_found: topic_qs,
            by_source,
            city_variants: city_count,
        });
    }

    Ok(Json(result))
}

/// POST /api/v1/research/bulk/preview
///
/// Preview how many questions bulk_research would generate without writing anything.
pub async fn bulk_research_preview(
    Json(req): Json<BulkResearchWithCitiesRequest>,
) -> ApiResult<Json<BulkPreviewResult>> {
    if req.topics.is_empty() {
        return Err(AppError::Validation(
            "At least one topic is required".into(),
        ));
    }
    let sources = req
        .sources
        .unwrap_or_else(|| vec!["quora".into(), "reddit".into(), "stackexchange".into()]);
    let effective_cities = if req.cities.is_empty() {
        1
    } else {
        req.cities.len()
    };
    let estimates: Vec<String> = req
        .topics
        .iter()
        .take(5)
        .flat_map(|t| {
            let phrase = t.search_phrase.clone().unwrap_or_else(|| t.name.clone());
            if req.cities.is_empty() {
                vec![phrase]
            } else {
                req.cities
                    .iter()
                    .take(3)
                    .map(move |c| format!("{} in {}", phrase, c))
                    .collect::<Vec<_>>()
            }
        })
        .collect();
    Ok(Json(BulkPreviewResult {
        estimated_topics: req.topics.len(),
        estimated_combinations: req.topics.len() * effective_cities,
        estimated_questions: req.topics.len() * effective_cities * (sources.len() + 1) * 8,
        sample_queries: estimates,
    }))
}
