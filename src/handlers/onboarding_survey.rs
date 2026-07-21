//! Onboarding Survey Framework
//!
//! Provides survey configuration per directory, public survey endpoints
//! for visitors, and admin endpoints for managing surveys.
//! Integrates with IncentiveSwift via fire-and-forget pipeline.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::auth::middleware::is_admin;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// ── Data Types ───────────────────────────────────────────────────────────────

/// Full survey config as stored in the database
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SurveyConfig {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub enabled: bool,
    pub title: String,
    pub description: Option<String>,
    pub questions: Value,           // JSONB
    pub completion_tags: Value,     // JSONB
    pub trigger_event: String,
    pub required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Survey response record as stored in the database
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SurveyResponse {
    pub id: Uuid,
    pub survey_id: Uuid,
    pub visitor_account_id: Option<Uuid>,
    pub visitor_fingerprint: Option<String>,
    pub directory_id: Uuid,
    pub answers: Value,             // JSONB
    pub applied_tags: Vec<String>,  // TEXT[]
    pub completed_at: DateTime<Utc>,
}

/// Public-facing survey config (no internal fields exposed)
#[derive(Debug, Clone, Serialize)]
pub struct PublicSurveyConfig {
    pub enabled: bool,
    pub title: String,
    pub description: Option<String>,
    pub questions: Value,
    pub trigger_event: String,
    pub required: bool,
}

/// Request payload for upserting survey config (admin)
#[derive(Debug, Deserialize)]
pub struct UpsertSurveyRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub questions: Option<Value>,
    pub enabled: Option<bool>,
    pub trigger_event: Option<String>,
    pub required: Option<bool>,
    pub completion_tags: Option<Value>,
}

/// Public request to submit survey answers
#[derive(Debug, Deserialize)]
pub struct SubmitSurveyRequest {
    pub visitor_account_id: Option<Uuid>,
    pub visitor_fingerprint: Option<String>,
    pub answers: Value,
}

// ── Admin: Get Survey Config ────────────────────────────────────────────────

/// GET /api/v1/admin/directories/:id/survey
pub async fn get_survey_config(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1"#
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    match config {
        Some(c) => Ok(Json(json!(c))),
        None => Ok(Json(json!({
            "directory_id": directory_id,
            "enabled": false,
            "title": "Help us personalize your experience",
            "description": null,
            "questions": [],
            "completion_tags": [],
            "trigger_event": "first_visit",
            "required": false,
        }))),
    }
}

// ── Admin: Upsert Survey Config ─────────────────────────────────────────────

/// PUT /api/v1/admin/directories/:id/survey
pub async fn upsert_survey_config(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<UpsertSurveyRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Verify directory exists
    let dir_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = $1"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    if dir_exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    // Check if config already exists
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directory_surveys WHERE directory_id = $1"
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    let enabled = req.enabled.unwrap_or(false);
    let title = req.title.clone().unwrap_or_else(|| "Help us personalize your experience".to_string());
    let description = req.description;
    let questions = req.questions.unwrap_or(json!([]));
    let trigger_event = req.trigger_event.unwrap_or_else(|| "first_visit".to_string());
    let required = req.required.unwrap_or(false);
    let completion_tags = req.completion_tags.unwrap_or(json!([]));

    if existing > 0 {
        // Update existing
        sqlx::query(
            r#"UPDATE directory_surveys SET
                enabled = $1, title = $2, description = $3, questions = $4,
                completion_tags = $5, trigger_event = $6, required = $7,
                updated_at = NOW()
               WHERE directory_id = $8"#
        )
        .bind(enabled)
        .bind(&title)
        .bind(&description)
        .bind(&questions)
        .bind(&completion_tags)
        .bind(&trigger_event)
        .bind(required)
        .bind(directory_id)
        .execute(&s.db)
        .await?;
    } else {
        // Insert new
        sqlx::query(
            r#"INSERT INTO directory_surveys
                (directory_id, enabled, title, description, questions,
                 completion_tags, trigger_event, required)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
        )
        .bind(directory_id)
        .bind(enabled)
        .bind(&title)
        .bind(&description)
        .bind(&questions)
        .bind(&completion_tags)
        .bind(&trigger_event)
        .bind(required)
        .execute(&s.db)
        .await?;
    }

    // Sync feature_config.onboarding_survey
    sync_feature_config(&s.db, directory_id, enabled).await?;

    // Return updated config
    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1"#
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(config)))
}

// ── Admin: Toggle Survey ────────────────────────────────────────────────────

/// POST /api/v1/admin/directories/:id/survey/toggle
pub async fn toggle_survey(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1"#
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    // If no config exists yet, create one (off by default, toggle to on)
    let new_enabled = match &config {
        Some(c) => !c.enabled,
        None => true,
    };

    let title = config.as_ref().map(|c| c.title.clone()).unwrap_or_else(|| "Help us personalize your experience".to_string());
    let description = config.as_ref().and_then(|c| c.description.clone());
    let questions = config.as_ref().map(|c| c.questions.clone()).unwrap_or(json!([]));
    let completion_tags = config.as_ref().map(|c| c.completion_tags.clone()).unwrap_or(json!([]));
    let trigger_event = config.as_ref().map(|c| c.trigger_event.clone()).unwrap_or_else(|| "first_visit".to_string());
    let required = config.as_ref().map(|c| c.required).unwrap_or(false);

    if config.is_some() {
        sqlx::query(
            r#"UPDATE directory_surveys SET
                enabled = $1, updated_at = NOW()
               WHERE directory_id = $2"#
        )
        .bind(new_enabled)
        .bind(directory_id)
        .execute(&s.db)
        .await?;
    } else {
        sqlx::query(
            r#"INSERT INTO directory_surveys
                (directory_id, enabled, title, description, questions,
                 completion_tags, trigger_event, required)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
        )
        .bind(directory_id)
        .bind(new_enabled)
        .bind(&title)
        .bind(&description)
        .bind(&questions)
        .bind(&completion_tags)
        .bind(&trigger_event)
        .bind(required)
        .execute(&s.db)
        .await?;
    }

    // Sync feature_config.onboarding_survey
    sync_feature_config(&s.db, directory_id, new_enabled).await?;

    Ok(Json(json!({
        "directory_id": directory_id,
        "enabled": new_enabled,
    })))
}

// ── Public: Get Survey Config ───────────────────────────────────────────────

/// GET /api/v1/public/directories/:slug/survey
/// Returns only the fields needed by the frontend survey UI
pub async fn public_get_survey(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 AND enabled = true"#
    )
    .bind(dir.0)
    .fetch_optional(&s.db)
    .await?;

    match config {
        Some(c) => {
            let public = PublicSurveyConfig {
                enabled: c.enabled,
                title: c.title,
                description: c.description,
                questions: c.questions,
                trigger_event: c.trigger_event,
                required: c.required,
            };
            Ok(Json(json!(public)))
        }
        None => Ok(Json(json!(PublicSurveyConfig {
            enabled: false,
            title: String::new(),
            description: None,
            questions: json!([]),
            trigger_event: "first_visit".to_string(),
            required: false,
        }))),
    }
}

// ── Public: Submit Survey Response ──────────────────────────────────────────

/// POST /api/v1/public/directories/:slug/survey/respond
pub async fn public_submit_survey(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<SubmitSurveyRequest>,
) -> ApiResult<impl IntoResponse> {
    // Resolve directory by slug
    let dir = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, name FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let directory_id = dir.0;
    let directory_name = dir.1;

    // Look up the enabled survey for this directory
    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 AND enabled = true"#
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("No active survey for this directory".to_string()))?;

    // Extract tags from completion_tags JSON array
    let applied_tags: Vec<String> = config.completion_tags
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Derive tags from answer values (classification + channel questions)
    let mut answer_tags: Vec<String> = Vec::new();
    let mut wants_newsletter: bool = false;
    if let Some(answers_arr) = req.answers.as_array() {
        for ans in answers_arr {
            let qid = ans.get("question_id").and_then(|v| v.as_str()).unwrap_or("");
            let value = ans.get("value");
            match qid {
                // Supplier classification → granular tag
                "supplier_classification" => {
                    if let Some(v) = value.and_then(|v| v.as_str()) {
                        let tag = match v {
                            "Farmer / Agricultural Producer" => "Farmer",
                            "Wholesale Distributor" => "Wholesale Distributor",
                            "Manufacturer / Factory Producer" => "Manufacturer",
                            "Trade Association / Co-op" => "Co-op",
                            "Food Hub / Aggregator" => "Food Hub",
                            "Artisan / Specialty Craft Producer" => "Artisan",
                            "Importer / Exporter" => "Importer / Exporter",
                            "Logistics & Freight Provider" => "Logistics Provider",
                            "Raw Material Supplier" => "Raw Material Supplier",
                            _ => v,
                        };
                        if !answer_tags.contains(&tag.to_string()) {
                            answer_tags.push(tag.to_string());
                        }
                    }
                }
                // Visitor channel → Weekly Email Digest → Newsletter auto-subscribe + tag
                "visitor_channel" => {
                    if let Some(v) = value.and_then(|v| v.as_str()) {
                        if v == "Weekly Email Digest" {
                            wants_newsletter = true;
                            // Short code based on directory slug for ZaarHub newsletter tags
                            // Format: {code}-zh-newsletter (e.g., pc-zh-newsletter, pb-zh-newsletter)
                            let newsletter_tag = format!(
                                "{}-zh-newsletter",
                                directory_slug_to_code(&slug)
                            );
                            if !answer_tags.contains(&newsletter_tag) {
                                answer_tags.push(newsletter_tag);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut all_tags = applied_tags.clone();
    for t in &answer_tags {
        if !all_tags.contains(&t) {
            all_tags.push(t.to_string());
        }
    }

    // Insert survey response
    let response = sqlx::query_as::<_, SurveyResponse>(
        r#"INSERT INTO survey_responses
            (survey_id, visitor_account_id, visitor_fingerprint, directory_id, answers, applied_tags)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#
    )
    .bind(config.id)
    .bind(req.visitor_account_id)
    .bind(&req.visitor_fingerprint)
    .bind(directory_id)
    .bind(&req.answers)
    .bind(&all_tags)
    .fetch_one(&s.db)
    .await?;

    // Apply tags to visitor account if one was provided
    if let Some(visitor_id) = req.visitor_account_id {
        if !all_tags.is_empty() {
            sqlx::query(
                r#"UPDATE visitor_accounts
                   SET interest_tags = array_cat(
                       COALESCE(interest_tags, '{}'::text[]),
                       $1::text[]
                   ), updated_at = NOW()
                   WHERE id = $2"#
            )
            .bind(&all_tags)
            .bind(visitor_id)
            .execute(&s.db)
            .await?;
        }
    }

    // ── Resolve visitor email for IncentiveSwift pipeline ──
    let visitor_email: Option<String> = if let Some(visitor_id) = req.visitor_account_id {
        sqlx::query_scalar("SELECT email FROM visitor_accounts WHERE id = $1")
            .bind(visitor_id)
            .fetch_optional(&s.db)
            .await
            .unwrap_or_default()
    } else {
        None
    };

    // ── Fire-and-forget: Pipeline to IncentiveSwift (rewards + points) ──
    let pipeline_payload = json!({
        "directory_slug": slug,
        "survey_id": config.id,
        "visitor_account_id": req.visitor_account_id,
        "visitor_email": visitor_email,
        "answers": req.answers,
        "applied_tags": all_tags,
    });

    let client = reqwest::Client::new();
    tokio::spawn(async move {
        match client
            .post("http://localhost:8083/api/v1/campaigns/external/survey-response")
            .json(&pipeline_payload)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(_) => tracing::info!("Survey response forwarded to IncentiveSwift"),
            Err(e) => tracing::warn!("Failed to forward survey response to IncentiveSwift: {}", e),
        }
    });

    // ── Auto-subscribe to newsletter if visitor chose Weekly Email Digest ──
    if wants_newsletter {
        if let Some(visitor_id) = req.visitor_account_id {
            let visitor_email_for_nl = visitor_email.clone().unwrap_or_default();
            if !visitor_email_for_nl.is_empty() {
                let nl_directory_id = directory_id;
                let db_nl = s.db.clone();
                tokio::spawn(async move {
                    // Upsert into newsletter_subscribers — on conflict (email + directory) do nothing
                    match sqlx::query(
                        r#"INSERT INTO newsletter_subscribers (directory_id, email, name, status)
                           VALUES ($1, $2, '', 'active')
                           ON CONFLICT (directory_id, email) DO NOTHING"#
                    )
                    .bind(nl_directory_id)
                    .bind(&visitor_email_for_nl)
                    .execute(&db_nl)
                    .await
                    {
                        Ok(r) => {
                            if r.rows_affected() > 0 {
                                tracing::info!(
                                    "[newsletter] Auto-subscribed {} to directory {}",
                                    visitor_email_for_nl, nl_directory_id
                                );
                            } else {
                                tracing::info!(
                                    "[newsletter] {} already subscribed to directory {}",
                                    visitor_email_for_nl, nl_directory_id
                                );
                            }
                        }
                        Err(e) => tracing::warn!(
                            "[newsletter] Failed to auto-subscribe {}: {}",
                            visitor_email_for_nl, e
                        ),
                    }
                });
            }
        }
    }

    // ── Fire tag sync to CoreSwift + IncentiveSwift (tag propagation) ──
    let survey_answer_tags = answer_tags.clone();
    let survey_visitor_email = visitor_email.clone();
    if !survey_answer_tags.is_empty() {
        if let Some(visitor_email_str) = survey_visitor_email {
            let survey_slug = slug.to_string();
            let db = s.db.clone();
            tokio::spawn(async move {
                crate::handlers::tag_sync::fire_tag_sync(
                    &db,
                    visitor_email_str.clone(),
                    None,
                    None,
                    None,
                    survey_answer_tags,
                    None,
                    None,
                    Some(survey_slug),
                    Some("onboarding_survey".to_string()),
                    Some("2944af81-2086-44b8-93c1-d83e93a5dec1".to_string()),
                    None,
                );
            });
        }
    }

    Ok(Json(json!({
        "id": response.id,
        "survey_id": response.survey_id,
        "completed_at": response.completed_at,
        "applied_tags": response.applied_tags,
    })))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Sync the onboarding_survey toggle into the directory's feature_config JSONB
async fn sync_feature_config(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let current_config: Value = sqlx::query_scalar(
        r#"SELECT COALESCE(feature_config, '{}'::jsonb) FROM directories WHERE id = $1"#
    )
    .bind(directory_id)
    .fetch_one(db)
    .await
    .unwrap_or(json!({}));

    let mut config = current_config.as_object().cloned().unwrap_or_default();
    config.insert("onboarding_survey".to_string(), json!(enabled));
    let new_config = Value::Object(config);

    sqlx::query(
        r#"UPDATE directories SET feature_config = $1, updated_at = NOW() WHERE id = $2"#
    )
    .bind(&new_config)
    .bind(directory_id)
    .execute(db)
    .await?;

    Ok(())
}

/// Map a directory slug to its short city code for ZaarHub newsletter tags.
/// Format: {code}-zh-newsletter (e.g., pc-zh-newsletter, pb-zh-newsletter)
fn directory_slug_to_code(slug: &str) -> &str {
    match slug {
        "apopka" => "ap",
        "boca-raton" => "br",
        "hollywood" => "hw",
        "lake-nona" => "ln",
        "palm-bay" => "pb",
        "palm-coast" => "pc",
        "pompano-beach" => "pp",
        "st-cloud" => "sc",
        "st-petersburg" => "sp",
        "winter-garden" => "wg",
        _ => slug, // fallback: use slug as-is
    }
}
