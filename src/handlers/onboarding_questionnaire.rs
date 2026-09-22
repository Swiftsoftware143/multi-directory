//! Onboarding questionnaire builder + responses view — card B65.
//!
//! In-house onboarding: the directory admin authors the questionnaire here (per audience,
//! per directory) and reads the answers back. Nothing about a question requires SQL or a
//! code change — the shape is validated against [`QUESTION_TYPES`] and stored as JSONB on
//! `directory_surveys`, and this module serves the type list to the UI so the editor is
//! data-driven rather than a fixed form.
//!
//! Draft vs published: a questionnaire is only reachable by the public GET when it is
//! `status = 'published'` AND `enabled`, so a half-written questionnaire can never be
//! half-served to a visitor.

use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::is_super_admin;
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

// ── The builder contract ────────────────────────────────────────────────────

/// Every answer type a question may use. The admin panel renders its type picker from
/// `GET /api/v1/admin/onboarding/question-types`, so this list is the single source of truth.
pub const QUESTION_TYPES: [&str; 9] = [
    "short_text",
    "long_text",
    "single_choice",
    "multiple_choice",
    "dropdown",
    "number",
    "yes_no",
    "rating",
    "date",
];

/// Who a questionnaire is for. A new audience is added here AND in the migration CHECK —
/// that is the only pair of places, and no code paths branch on audience identity.
pub const AUDIENCES: [&str; 3] = ["customer", "supplier", "business"];

/// Guards for a public form: an unbounded builder is a denial-of-service on your own API.
const MAX_QUESTIONS: usize = 100;
const MAX_OPTIONS: usize = 50;
const MAX_LABEL_LEN: usize = 500;
const MAX_HELP_LEN: usize = 500;
const MAX_RESPONSE_PAGE: i64 = 200;

pub fn is_known_audience(audience: &str) -> bool {
    AUDIENCES.contains(&audience)
}

/// Legacy type aliases used by questionnaires authored before the builder existed.
/// They are normalised on write AND on read, so the public widget sees one shape.
fn canonical_type(raw: &str) -> Option<&'static str> {
    let t = raw.trim().to_ascii_lowercase();
    Some(match t.as_str() {
        "short_text" | "text" | "string" => "short_text",
        "long_text" | "textarea" | "paragraph" => "long_text",
        "single_choice" | "choice" | "radio" => "single_choice",
        "multiple_choice" | "multi" | "checkbox" => "multiple_choice",
        "dropdown" | "select" => "dropdown",
        "number" | "numeric" => "number",
        "yes_no" | "boolean" | "bool" => "yes_no",
        "rating" | "scale" | "rating_scale" => "rating",
        "date" => "date",
        _ => return None,
    })
}

fn is_choice_type(t: &str) -> bool {
    matches!(t, "single_choice" | "multiple_choice" | "dropdown")
}

/// Normalise one option (a bare string or `{label, value}`) to `{value, label}`.
fn normalize_option(raw: &Value) -> Option<Value> {
    let (label, value) = match raw {
        Value::String(s) => {
            let label = s.trim();
            if label.is_empty() {
                return None;
            }
            (label.to_string(), slug_key(label))
        }
        Value::Object(o) => {
            let label = o
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let value = o
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if label.is_empty() && value.is_empty() {
                return None;
            }
            let label = if label.is_empty() {
                value.clone()
            } else {
                label
            };
            let value = if value.is_empty() {
                slug_key(&label)
            } else {
                value
            };
            (label, value)
        }
        _ => return None,
    };
    Some(json!({ "value": value, "label": label }))
}

/// A stable machine key for a human label (used for option values and CoreSwift field keys).
pub fn slug_key(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_us = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "field".to_string()
    } else {
        trimmed.chars().take(60).collect()
    }
}

/// Validate + normalise a submitted `questions` array.
///
/// Returns the array exactly as it will be stored (ids assigned, types canonical, options
/// normalised, `order` set) or a human-readable reason it was rejected. A questionnaire the
/// admin cannot render is rejected at write time rather than discovered by a visitor.
pub fn normalize_questions(raw: &Value) -> Result<Value, String> {
    let arr = raw
        .as_array()
        .ok_or_else(|| "questions must be a JSON array".to_string())?;
    if arr.len() > MAX_QUESTIONS {
        return Err(format!(
            "at most {MAX_QUESTIONS} questions per questionnaire"
        ));
    }

    let mut out: Vec<Value> = Vec::with_capacity(arr.len());
    let mut seen_ids: Vec<String> = Vec::new();

    for (i, q) in arr.iter().enumerate() {
        // A bare string is a legacy short-text question.
        let (label, raw_type, help, required, options, scale_min, scale_max, tags) = match q {
            Value::String(s) => (
                s.trim().to_string(),
                "short_text".to_string(),
                None,
                false,
                Vec::new(),
                1i64,
                5i64,
                Vec::new(),
            ),
            Value::Object(o) => {
                let label = o
                    .get("label")
                    .or_else(|| o.get("question"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let raw_type = o
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("short_text")
                    .to_string();
                let help = o
                    .get("help_text")
                    .or_else(|| o.get("help"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let required = o.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
                let options = o
                    .get("options")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let scale_min = o.get("scale_min").and_then(|v| v.as_i64()).unwrap_or(1);
                let scale_max = o.get("scale_max").and_then(|v| v.as_i64()).unwrap_or(5);
                let tags = o
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (
                    label, raw_type, help, required, options, scale_min, scale_max, tags,
                )
            }
            _ => return Err(format!("question {} is not an object or a string", i + 1)),
        };

        if label.is_empty() {
            return Err(format!("question {} has no label", i + 1));
        }
        if label.chars().count() > MAX_LABEL_LEN {
            return Err(format!(
                "question {} label is longer than {MAX_LABEL_LEN} characters",
                i + 1
            ));
        }
        if let Some(h) = &help {
            if h.chars().count() > MAX_HELP_LEN {
                return Err(format!(
                    "question {} help text is longer than {MAX_HELP_LEN} characters",
                    i + 1
                ));
            }
        }

        let qtype = canonical_type(&raw_type)
            .ok_or_else(|| {
                format!(
                    "question {} has unknown type '{}' — allowed: {}",
                    i + 1,
                    raw_type,
                    QUESTION_TYPES.join(", ")
                )
            })?
            .to_string();

        // Options: required (and complete) for the three choice types, ignored otherwise.
        let mut norm_options: Vec<Value> = Vec::new();
        if is_choice_type(&qtype) {
            if options.len() > MAX_OPTIONS {
                return Err(format!(
                    "question {} has more than {MAX_OPTIONS} options",
                    i + 1
                ));
            }
            for o in &options {
                if let Some(v) = normalize_option(o) {
                    norm_options.push(v);
                }
            }
            if norm_options.len() < 2 {
                return Err(format!(
                    "question {} is a {} — it needs at least 2 options",
                    i + 1,
                    qtype
                ));
            }
        } else if qtype == "yes_no" {
            norm_options = vec![
                json!({ "value": "yes", "label": "Yes" }),
                json!({ "value": "no", "label": "No" }),
            ];
        }

        if qtype == "rating" && scale_max <= scale_min {
            return Err(format!(
                "question {} rating scale is invalid (min {} must be below max {})",
                i + 1,
                scale_min,
                scale_max
            ));
        }

        let id = match q.get("id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => format!("q_{}", Uuid::new_v4().simple()),
        };
        if seen_ids.contains(&id) {
            return Err(format!("question id '{id}' is used twice"));
        }
        seen_ids.push(id.clone());

        out.push(json!({
            "id": id,
            "type": qtype,
            "label": label,
            "help_text": help,
            "required": required,
            "options": norm_options,
            "scale_min": scale_min,
            "scale_max": scale_max,
            "tags": tags,
            "order": i,
        }));
    }

    Ok(Value::Array(out))
}

/// Answer types with a human label and an options hint — what the admin UI renders.
pub fn question_type_catalogue() -> Value {
    json!([
        { "type": "short_text", "label": "Short text", "options": false, "help": "A single line answer." },
        { "type": "long_text", "label": "Long text", "options": false, "help": "A paragraph answer." },
        { "type": "single_choice", "label": "Single choice (radio)", "options": true, "help": "One option out of several." },
        { "type": "multiple_choice", "label": "Multiple choice (checkboxes)", "options": true, "help": "Any number of options." },
        { "type": "dropdown", "label": "Dropdown", "options": true, "help": "One option, compact." },
        { "type": "number", "label": "Number", "options": false, "help": "A numeric answer." },
        { "type": "yes_no", "label": "Yes / No", "options": false, "help": "A two-button answer." },
        { "type": "rating", "label": "Rating scale", "options": false, "help": "A 1–N scale; set the range on the question." },
        { "type": "date", "label": "Date", "options": false, "help": "A calendar date." }
    ])
}

// ── Rows / payloads ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct QuestionnaireRow {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub audience: String,
    pub status: String,
    pub enabled: bool,
    pub title: String,
    pub description: Option<String>,
    pub questions: Value,
    pub completion_tags: Value,
    pub trigger_event: String,
    pub required: bool,
    pub reward_units: i32,
    pub network_id: Option<Uuid>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Everything the builder can set. All fields optional so a partial PUT is additive.
#[derive(Debug, Deserialize)]
pub struct QuestionnaireUpsert {
    pub title: Option<String>,
    pub description: Option<String>,
    pub questions: Option<Value>,
    pub completion_tags: Option<Value>,
    pub trigger_event: Option<String>,
    pub required: Option<bool>,
    /// `draft` | `published` | `publish` | `unpublish`
    pub status: Option<String>,
    pub reward_units: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseQuery {
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Inclusive RFC3339 date bounds on `completed_at`.
    pub from: Option<String>,
    pub to: Option<String>,
}

async fn load_questionnaire(
    db: &sqlx::PgPool,
    directory_id: &Uuid,
    audience: &str,
) -> Result<Option<QuestionnaireRow>, AppError> {
    let row = sqlx::query_as::<_, QuestionnaireRow>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 AND audience = $2"#,
    )
    .bind(directory_id)
    .bind(audience)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

async fn assert_directory(db: &sqlx::PgPool, directory_id: &Uuid) -> Result<(), AppError> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM directories WHERE id = $1)")
            .bind(directory_id)
            .fetch_one(db)
            .await?;
    if !exists {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }
    Ok(())
}

// ── GET /api/v1/admin/onboarding/question-types ─────────────────────────────

/// The builder's type picker is served from the backend constant, never hardcoded in the page.
pub async fn question_types(
    State(_s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    Ok(Json(json!({
        "question_types": question_type_catalogue(),
        "audiences": AUDIENCES,
    })))
}

// ── GET /api/v1/admin/directories/:id/questionnaires ────────────────────────

/// Every questionnaire of a directory, with the question count and the response count —
/// what the admin needs to see which audiences are actually collecting answers.
pub async fn list_questionnaires(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    assert_directory(&s.db, &directory_id).await?;

    let rows = sqlx::query_as::<_, QuestionnaireRow>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 ORDER BY audience"#,
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let counts = sqlx::query_as::<_, (String, i64)>(
        r#"SELECT audience, COUNT(*) FROM survey_responses
           WHERE directory_id = $1 GROUP BY audience"#,
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let mut data: Vec<Value> = Vec::new();
    for row in rows {
        let responses = counts
            .iter()
            .find(|(a, _)| a == &row.audience)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        let question_count = row.questions.as_array().map(|a| a.len()).unwrap_or(0);
        data.push(json!({
            "id": row.id,
            "audience": row.audience,
            "status": row.status,
            "enabled": row.enabled,
            "title": row.title,
            "question_count": question_count,
            "responses": responses,
            "reward_units": row.reward_units,
            "network_id": row.network_id,
            "updated_at": row.updated_at,
            "published_at": row.published_at,
        }));
    }

    // Audiences with no questionnaire yet still need a tab in the builder, so the UI can
    // author the first one without inventing the audience client-side.
    let authored: Vec<&str> = data
        .iter()
        .filter_map(|d| d.get("audience").and_then(|a| a.as_str()))
        .collect();
    let missing: Vec<&str> = AUDIENCES
        .iter()
        .filter(|a| !authored.contains(*a))
        .copied()
        .collect();

    Ok(Json(json!({
        "directory_id": directory_id,
        "questionnaires": data,
        "audiences_without_questionnaire": missing,
        "known_audiences": AUDIENCES,
    })))
}

// ── GET /api/v1/admin/directories/:id/questionnaires/:audience ──────────────

pub async fn get_questionnaire(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((directory_id, audience)): Path<(Uuid, String)>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    assert_directory(&s.db, &directory_id).await?;
    if !is_known_audience(&audience) {
        return Err(AppError::BadRequest(format!(
            "Unknown audience '{audience}' — allowed: {}",
            AUDIENCES.join(", ")
        )));
    }

    match load_questionnaire(&s.db, &directory_id, &audience).await? {
        Some(row) => {
            // Normalise on read as well: a legacy row (type 'choice') must render in the
            // builder exactly as the public widget will see it.
            let questions = normalize_questions(&row.questions).unwrap_or(row.questions.clone());
            Ok(Json(json!({
                "exists": true,
                "id": row.id,
                "directory_id": row.directory_id,
                "audience": row.audience,
                "status": row.status,
                "enabled": row.enabled,
                "title": row.title,
                "description": row.description,
                "questions": questions,
                "completion_tags": row.completion_tags,
                "trigger_event": row.trigger_event,
                "required": row.required,
                "reward_units": row.reward_units,
                "published_at": row.published_at,
                "updated_at": row.updated_at,
            })))
        }
        None => Ok(Json(json!({
            "exists": false,
            "directory_id": directory_id,
            "audience": audience,
            "status": "draft",
            "enabled": false,
            "title": "",
            "description": null,
            "questions": [],
            "completion_tags": [],
            "trigger_event": "first_visit",
            "required": false,
            "reward_units": 0,
        }))),
    }
}

// ── PUT /api/v1/admin/directories/:id/questionnaires/:audience ──────────────

pub async fn upsert_questionnaire(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((directory_id, audience)): Path<(Uuid, String)>,
    Json(req): Json<QuestionnaireUpsert>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    assert_directory(&s.db, &directory_id).await?;
    if !is_known_audience(&audience) {
        return Err(AppError::BadRequest(format!(
            "Unknown audience '{audience}' — allowed: {}",
            AUDIENCES.join(", ")
        )));
    }

    let existing = load_questionnaire(&s.db, &directory_id, &audience).await?;

    let title = req
        .title
        .or_else(|| existing.as_ref().map(|e| e.title.clone()))
        .unwrap_or_else(|| "Help us personalize your experience".to_string());
    if title.trim().is_empty() {
        return Err(AppError::Validation(
            "Questionnaire title is required".to_string(),
        ));
    }
    let description = match req.description {
        Some(d) => Some(d),
        None => existing.as_ref().and_then(|e| e.description.clone()),
    };
    let raw_questions = req
        .questions
        .or_else(|| existing.as_ref().map(|e| e.questions.clone()))
        .unwrap_or_else(|| json!([]));
    let questions = normalize_questions(&raw_questions).map_err(AppError::Validation)?;
    let completion_tags = req
        .completion_tags
        .or_else(|| existing.as_ref().map(|e| e.completion_tags.clone()))
        .unwrap_or_else(|| json!([]));
    if !completion_tags.is_array() {
        return Err(AppError::Validation(
            "completion_tags must be an array of tags".to_string(),
        ));
    }
    let trigger_event = req
        .trigger_event
        .or_else(|| existing.as_ref().map(|e| e.trigger_event.clone()))
        .unwrap_or_else(|| "first_visit".to_string());
    let required = req
        .required
        .or_else(|| existing.as_ref().map(|e| e.required))
        .unwrap_or(false);
    let reward_units = req
        .reward_units
        .or_else(|| existing.as_ref().map(|e| e.reward_units))
        .unwrap_or(0)
        .max(0);

    // status: an explicit draft/published wins; `enabled` alone maps onto the lifecycle so
    // the legacy toggle and the builder can never disagree.
    let prev_status = existing
        .as_ref()
        .map(|e| e.status.clone())
        .unwrap_or_else(|| "draft".to_string());
    let status = match req.status.as_deref().map(str::trim) {
        Some("publish") => "published".to_string(),
        Some("unpublish") => "draft".to_string(),
        Some("published") => "published".to_string(),
        Some("draft") => "draft".to_string(),
        Some(other) => {
            return Err(AppError::Validation(format!(
                "status must be draft or published (got '{other}')"
            )))
        }
        None => match req.enabled {
            Some(true) => "published".to_string(),
            Some(false) => "draft".to_string(),
            None => prev_status,
        },
    };

    let question_count = questions.as_array().map(|a| a.len()).unwrap_or(0);
    if status == "published" && question_count == 0 {
        return Err(AppError::Validation(
            "A questionnaire needs at least one question before it can be published".to_string(),
        ));
    }
    let enabled = status == "published";

    let network_id: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_one(&s.db)
            .await?;

    // One statement for both create and update: the unique index (directory_id, audience)
    // is what makes "a questionnaire per audience" true rather than hopeful.
    sqlx::query(
        r#"INSERT INTO directory_surveys
              (directory_id, audience, status, enabled, title, description, questions,
               completion_tags, trigger_event, required, reward_units, network_id, published_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                   CASE WHEN $3 = 'published' THEN NOW() ELSE NULL END)
           ON CONFLICT (directory_id, audience) DO UPDATE SET
              status = EXCLUDED.status,
              enabled = EXCLUDED.enabled,
              title = EXCLUDED.title,
              description = EXCLUDED.description,
              questions = EXCLUDED.questions,
              completion_tags = EXCLUDED.completion_tags,
              trigger_event = EXCLUDED.trigger_event,
              required = EXCLUDED.required,
              reward_units = EXCLUDED.reward_units,
              network_id = EXCLUDED.network_id,
              published_at = CASE WHEN EXCLUDED.status = 'published'
                                  THEN COALESCE(directory_surveys.published_at, NOW())
                                  ELSE NULL END,
              updated_at = NOW()"#,
    )
    .bind(directory_id)
    .bind(&audience)
    .bind(&status)
    .bind(enabled)
    .bind(title.trim())
    .bind(&description)
    .bind(&questions)
    .bind(&completion_tags)
    .bind(&trigger_event)
    .bind(required)
    .bind(reward_units)
    .bind(network_id)
    .execute(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("Could not save the questionnaire: {e}")))?;

    // The public GET gates on feature_config.onboarding_survey too, so keep it in step.
    let current_config: Value = sqlx::query_scalar(
        "SELECT COALESCE(feature_config, '{}'::jsonb) FROM directories WHERE id = $1",
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or_else(|_| json!({}));
    let mut cfg = current_config.as_object().cloned().unwrap_or_default();
    cfg.insert("onboarding_survey".to_string(), json!(enabled));
    sqlx::query("UPDATE directories SET feature_config = $1, updated_at = NOW() WHERE id = $2")
        .bind(Value::Object(cfg))
        .bind(directory_id)
        .execute(&s.db)
        .await?;

    let row = load_questionnaire(&s.db, &directory_id, &audience)
        .await?
        .ok_or_else(|| AppError::Internal("Questionnaire disappeared after save".to_string()))?;

    tracing::info!(
        "[onboarding] questionnaire saved: directory={} audience={} status={} questions={} reward_units={}",
        directory_id,
        audience,
        row.status,
        question_count,
        row.reward_units
    );

    Ok(Json(json!({
        "ok": true,
        "questionnaire": row,
        "question_count": question_count,
        "public_url": format!("/api/v1/public/directories/{}/survey?audience={}",
            sqlx::query_scalar::<_, String>("SELECT slug FROM directories WHERE id = $1")
                .bind(directory_id)
                .fetch_one(&s.db)
                .await
                .unwrap_or_default(),
            audience),
    })))
}

// ── DELETE /api/v1/admin/directories/:id/questionnaires/:audience ───────────

pub async fn delete_questionnaire(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((directory_id, audience)): Path<(Uuid, String)>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    if !is_known_audience(&audience) {
        return Err(AppError::BadRequest(format!(
            "Unknown audience '{audience}' — allowed: {}",
            AUDIENCES.join(", ")
        )));
    }

    // survey_responses.survey_id cascades, so refuse rather than silently delete answers.
    let responses: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM survey_responses r JOIN directory_surveys d ON d.id = r.survey_id
          WHERE d.directory_id = $1 AND d.audience = $2",
    )
    .bind(directory_id)
    .bind(&audience)
    .fetch_one(&s.db)
    .await?;
    if responses > 0 {
        return Err(AppError::BadRequest(format!(
            "This questionnaire has {responses} response(s) — unpublish it instead of deleting it"
        )));
    }

    let deleted =
        sqlx::query("DELETE FROM directory_surveys WHERE directory_id = $1 AND audience = $2")
            .bind(directory_id)
            .bind(&audience)
            .execute(&s.db)
            .await?
            .rows_affected();

    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

// ── GET .../questionnaires/:audience/responses ──────────────────────────────

/// The responses view: every answer readable, per questionnaire, filterable by free text
/// and by completion date, with what actually happened to the reward and to the CRM push.
pub async fn list_responses(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((directory_id, audience)): Path<(Uuid, String)>,
    Query(params): Query<ResponseQuery>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }
    assert_directory(&s.db, &directory_id).await?;

    let limit = params.limit.unwrap_or(50).clamp(1, MAX_RESPONSE_PAGE);
    let offset = params.offset.unwrap_or(0).max(0);
    let q = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{s}%"));

    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Value,
            Vec<String>,
            DateTime<Utc>,
            i32,
            Option<String>,
            bool,
            Option<String>,
            Option<Uuid>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"SELECT r.id, r.survey_id, r.audience, r.answers, r.applied_tags, r.completed_at,
                  r.reward_units_awarded, r.currency_name, r.coreswift_pushed,
                  r.coreswift_push_error, r.visitor_account_id,
                  v.email, v.name, v.business_type
             FROM survey_responses r
             LEFT JOIN visitor_accounts v ON v.id = r.visitor_account_id
            WHERE r.directory_id = $1
              AND ($2::text IS NULL OR r.audience = $2)
              AND ($3::text IS NULL OR r.answers::text ILIKE $3
                   OR v.email ILIKE $3 OR v.name ILIKE $3)
              AND ($4::timestamptz IS NULL OR r.completed_at >= $4::timestamptz)
              AND ($5::timestamptz IS NULL OR r.completed_at <= $5::timestamptz)
            ORDER BY r.completed_at DESC
            LIMIT $6 OFFSET $7"#,
    )
    .bind(directory_id)
    .bind(&audience)
    .bind(&q)
    .bind(&params.from)
    .bind(&params.to)
    .bind(limit)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM survey_responses WHERE directory_id = $1 AND audience = $2"#,
    )
    .bind(directory_id)
    .bind(&audience)
    .fetch_one(&s.db)
    .await?;

    // The question labels are joined in so an answer stays readable even after a question
    // was renamed or removed from the questionnaire.
    let questionnaire = load_questionnaire(&s.db, &directory_id, &audience).await?;
    let question_labels: Vec<(String, String, String)> = questionnaire
        .as_ref()
        .and_then(|qq| normalize_questions(&qq.questions).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|qq| {
                    Some((
                        qq.get("id")?.as_str()?.to_string(),
                        qq.get("label")?.as_str()?.to_string(),
                        qq.get("type")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    let items: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                survey_id,
                audience,
                answers,
                applied_tags,
                completed_at,
                reward_units_awarded,
                currency_name,
                coreswift_pushed,
                coreswift_push_error,
                visitor_account_id,
                visitor_email,
                visitor_name,
                business_type,
            )| {
                let mut readable: Vec<Value> = Vec::new();
                if let Some(arr) = answers.as_array() {
                    for a in arr {
                        let qid = a.get("question_id").and_then(|v| v.as_str()).unwrap_or("");
                        let label = a
                            .get("question_label")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .or_else(|| {
                                question_labels
                                    .iter()
                                    .find(|(qid2, _, _)| qid2 == qid)
                                    .map(|(_, l, _)| l.clone())
                            })
                            .unwrap_or_else(|| qid.to_string());
                        let qtype = a
                            .get("type")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or_else(|| {
                                question_labels
                                    .iter()
                                    .find(|(qid2, _, _)| qid2 == qid)
                                    .map(|(_, _, t)| t.clone())
                            })
                            .unwrap_or_else(|| "short_text".to_string());
                        readable.push(json!({
                            "question_id": qid,
                            "question": label,
                            "type": qtype,
                            "value": a.get("value").cloned().unwrap_or(Value::Null),
                        }));
                    }
                } else if let Some(obj) = answers.as_object() {
                    for (k, v) in obj {
                        readable.push(json!({ "question_id": k, "question": k, "type": "short_text", "value": v }));
                    }
                }
                json!({
                    "id": id,
                    "survey_id": survey_id,
                    "audience": audience,
                    "completed_at": completed_at,
                    "visitor_account_id": visitor_account_id,
                    "respondent_email": visitor_email,
                    "respondent_name": visitor_name,
                    "business_type": business_type,
                    "reward_units_awarded": reward_units_awarded,
                    "currency_name": currency_name,
                    "coreswift_pushed": coreswift_pushed,
                    "coreswift_push_error": coreswift_push_error,
                    "applied_tags": applied_tags,
                    "answers": readable,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "directory_id": directory_id,
        "audience": audience,
        "questionnaire_exists": questionnaire.is_some(),
        "questionnaire_status": questionnaire.as_ref().map(|qq| qq.status.clone()),
        "question_labels": question_labels
            .iter()
            .map(|(id, label, t)| json!({ "id": id, "label": label, "type": t }))
            .collect::<Vec<_>>(),
        "total": total,
        "limit": limit,
        "offset": offset,
        "responses": items,
    })))
}
