//! Onboarding — the public side (card B65).
//!
//! Onboarding is Multi-Directory's own. This module serves the published questionnaire to
//! whoever is signing up (customer, supplier or business) and stores the answers:
//!
//!   1. the answers land in `survey_responses` — always, whatever happens downstream;
//!   2. the completion reward is credited by the NATIVE loyalty engine
//!      ([`crate::handlers::loyalty_native::credit_visitor_units`]) — one network-wide
//!      programme, no campaign, no external service;
//!   3. the answers are mapped into CoreSwift through the `coreswift` seam, whose credentials
//!      come from the database per directory. A CRM that is down or unconnected is logged and
//!      skipped — it never costs the visitor their answers or their currency, and it is never
//!      reported as a successful push when it was not one.
//!
//! There is NO IncentiveSwift call here any more: the previous fire-and-forget POST to a
//! hardcoded localhost:8083 was broken (IncentiveSwift had changed its reply shape) and only
//! ever fired for the regular visitor audience.
//!
//! The GET/POST pair is deliberately PUBLIC — a visitor completes onboarding without an
//! operator session. Authoring happens in the admin panel (`onboarding_questionnaire.rs`).

use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::middleware::is_super_admin;
use crate::auth::models::Claims;
use crate::coreswift::LeadPayload;
use crate::error::{ApiResult, AppError};
use crate::handlers::onboarding_questionnaire::{normalize_questions, slug_key};
use crate::AppState;

// ── Data Types ───────────────────────────────────────────────────────────────

/// Full survey config as stored in the database
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SurveyConfig {
    pub id: Uuid,
    pub directory_id: Uuid,
    /// customer | supplier | business — the audience this questionnaire was authored for.
    pub audience: String,
    /// draft | published. Only `published` is ever served to the public.
    pub status: String,
    pub enabled: bool,
    pub title: String,
    pub description: Option<String>,
    pub questions: Value,       // JSONB
    pub completion_tags: Value, // JSONB
    pub trigger_event: String,
    pub required: bool,
    /// Native currency units credited on completion (100 units = US$1). 0 = nothing to earn.
    pub reward_units: i32,
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
    /// customer | supplier | business — which questionnaire was answered.
    pub audience: String,
    pub answers: Value,            // JSONB
    pub applied_tags: Vec<String>, // TEXT[]
    pub completed_at: DateTime<Utc>,
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
    /// Which questionnaire the respondent filled in. Defaults to `customer`.
    pub audience: Option<String>,
    pub visitor_account_id: Option<Uuid>,
    pub visitor_fingerprint: Option<String>,
    pub answers: Value,
}

/// The audience for a public request. Absent = `customer`, so the pre-existing widget URL
/// (no query string) keeps resolving to the customer questionnaire.
fn audience_or_default(audience: Option<&str>) -> String {
    let a = audience.unwrap_or("customer").trim().to_ascii_lowercase();
    if crate::handlers::onboarding_questionnaire::is_known_audience(&a) {
        a
    } else {
        "customer".to_string()
    }
}

// ── Admin: Get Survey Config ────────────────────────────────────────────────

/// GET /api/v1/admin/directories/:id/survey
pub async fn get_survey_config(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 AND audience = 'customer'"#,
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    match config {
        Some(c) => Ok(Json(json!(c))),
        None => Ok(Json(json!({
            "directory_id": directory_id,
            "audience": "customer",
            "status": "draft",
            "enabled": false,
            "title": "Help us personalize your experience",
            "description": null,
            "questions": [],
            "completion_tags": [],
            "trigger_event": "first_visit",
            "required": false,
            "reward_units": 0,
        }))),
    }
}

// ── Admin: Upsert Survey Config ─────────────────────────────────────────────

/// PUT /api/v1/admin/directories/:id/survey
///
/// The legacy single-questionnaire endpoint. It now writes the `customer` questionnaire and
/// keeps its lifecycle in step with `enabled`, so it cannot disagree with the builder.
pub async fn upsert_survey_config(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<UpsertSurveyRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Verify directory exists
    let dir_exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = $1")
        .bind(directory_id)
        .fetch_one(&s.db)
        .await?;

    if dir_exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let enabled = req.enabled.unwrap_or(false);
    let title = req
        .title
        .clone()
        .unwrap_or_else(|| "Help us personalize your experience".to_string());
    let description = req.description;
    let questions =
        normalize_questions(&req.questions.unwrap_or(json!([]))).map_err(AppError::Validation)?;
    let trigger_event = req
        .trigger_event
        .unwrap_or_else(|| "first_visit".to_string());
    let required = req.required.unwrap_or(false);
    let completion_tags = req.completion_tags.unwrap_or(json!([]));
    let status = if enabled { "published" } else { "draft" };
    let network_id: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_one(&s.db)
            .await?;

    sqlx::query(
        r#"INSERT INTO directory_surveys
              (directory_id, audience, status, enabled, title, description, questions,
               completion_tags, trigger_event, required, reward_units, network_id, published_at)
           VALUES ($1, 'customer', $2, $3, $4, $5, $6, $7, $8, $9, 0, $10,
                   CASE WHEN $2 = 'published' THEN NOW() ELSE NULL END)
           ON CONFLICT (directory_id, audience) DO UPDATE SET
              status = EXCLUDED.status, enabled = EXCLUDED.enabled, title = EXCLUDED.title,
              description = EXCLUDED.description, questions = EXCLUDED.questions,
              completion_tags = EXCLUDED.completion_tags, trigger_event = EXCLUDED.trigger_event,
              required = EXCLUDED.required, network_id = EXCLUDED.network_id,
              published_at = CASE WHEN EXCLUDED.status = 'published'
                                  THEN COALESCE(directory_surveys.published_at, NOW())
                                  ELSE NULL END,
              updated_at = NOW()"#,
    )
    .bind(directory_id)
    .bind(status)
    .bind(enabled)
    .bind(&title)
    .bind(&description)
    .bind(&questions)
    .bind(&completion_tags)
    .bind(&trigger_event)
    .bind(required)
    .bind(network_id)
    .execute(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("Could not save the survey: {e}")))?;

    // Sync feature_config.onboarding_survey
    sync_feature_config(&s.db, directory_id, enabled).await?;

    // Return updated config
    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 AND audience = 'customer'"#,
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
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys WHERE directory_id = $1 AND audience = 'customer'"#,
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    // If no config exists yet, create one (off by default, toggle to on)
    let new_enabled = match &config {
        Some(c) => !c.enabled,
        None => true,
    };
    let status = if new_enabled { "published" } else { "draft" };

    let title = config
        .as_ref()
        .map(|c| c.title.clone())
        .unwrap_or_else(|| "Help us personalize your experience".to_string());
    let description = config.as_ref().and_then(|c| c.description.clone());
    let questions = config
        .as_ref()
        .map(|c| c.questions.clone())
        .unwrap_or(json!([]));
    let completion_tags = config
        .as_ref()
        .map(|c| c.completion_tags.clone())
        .unwrap_or(json!([]));
    let trigger_event = config
        .as_ref()
        .map(|c| c.trigger_event.clone())
        .unwrap_or_else(|| "first_visit".to_string());
    let required = config.as_ref().map(|c| c.required).unwrap_or(false);
    let network_id: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_one(&s.db)
            .await?;

    sqlx::query(
        r#"INSERT INTO directory_surveys
              (directory_id, audience, status, enabled, title, description, questions,
               completion_tags, trigger_event, required, reward_units, network_id, published_at)
           VALUES ($1, 'customer', $2, $3, $4, $5, $6, $7, $8, $9, 0, $10,
                   CASE WHEN $2 = 'published' THEN NOW() ELSE NULL END)
           ON CONFLICT (directory_id, audience) DO UPDATE SET
              status = EXCLUDED.status, enabled = EXCLUDED.enabled,
              published_at = CASE WHEN EXCLUDED.status = 'published'
                                  THEN COALESCE(directory_surveys.published_at, NOW())
                                  ELSE NULL END,
              updated_at = NOW()"#,
    )
    .bind(directory_id)
    .bind(status)
    .bind(new_enabled)
    .bind(&title)
    .bind(&description)
    .bind(&questions)
    .bind(&completion_tags)
    .bind(&trigger_event)
    .bind(required)
    .bind(network_id)
    .execute(&s.db)
    .await?;

    // Sync feature_config.onboarding_survey
    sync_feature_config(&s.db, directory_id, new_enabled).await?;

    Ok(Json(json!({
        "directory_id": directory_id,
        "audience": "customer",
        "status": status,
        "enabled": new_enabled,
    })))
}

// ── Public: Get Survey Config ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PublicSurveyQuery {
    pub audience: Option<String>,
}

/// The published questionnaire governing a directory for one audience. Only ever returns a
/// row that is BOTH published and enabled — a draft cannot leak to the public by any path.
async fn published_survey(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    audience: &str,
) -> Result<Option<SurveyConfig>, AppError> {
    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys
            WHERE directory_id = $1 AND audience = $2 AND enabled = true AND status = 'published'"#,
    )
    .bind(directory_id)
    .bind(audience)
    .fetch_optional(db)
    .await?;
    Ok(config)
}

fn public_json(config: &SurveyConfig, slug: &str) -> Value {
    json!({
        "enabled": config.enabled,
        "audience": config.audience,
        "directory_slug": slug,
        "title": config.title,
        "description": config.description,
        "questions": normalize_questions(&config.questions).unwrap_or(config.questions.clone()),
        "trigger_event": config.trigger_event,
        "required": config.required,
        "reward_units": config.reward_units,
    })
}

fn empty_public_json(audience: String) -> Value {
    json!({
        "enabled": false,
        "audience": audience,
        "directory_slug": Value::Null,
        "title": "",
        "description": Value::Null,
        "questions": [],
        "trigger_event": "first_visit",
        "required": false,
        "reward_units": 0,
    })
}

/// GET /api/v1/public/directories/:slug/survey[?audience=customer|supplier|business]
///
/// PUBLIC by design: a visitor fills in onboarding with no operator session. Only a
/// questionnaire that is published AND enabled is served — a draft never reaches a visitor.
pub async fn public_get_survey(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    axum::extract::Query(query): axum::extract::Query<PublicSurveyQuery>,
) -> ApiResult<impl IntoResponse> {
    let audience = audience_or_default(query.audience.as_deref());

    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    match published_survey(&s.db, dir.0, &audience).await? {
        Some(c) => Ok(Json(public_json(&c, &slug))),
        None => Ok(Json(empty_public_json(audience))),
    }
}

/// GET /api/v1/public/onboarding[?audience=...]
///
/// For the signup surfaces that have no city. A business or supplier account is created
/// network-wide (`visitor_accounts.directory_id` is NULL), so there is no city slug to address
/// — this resolves the questionnaire the admin most recently published for that audience and
/// returns the directory slug it belongs to, so the answers are posted back to that same
/// directory. Purely data-driven: if no admin has published one, `enabled` is false.
pub async fn public_get_onboarding_network(
    State(s): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PublicSurveyQuery>,
) -> ApiResult<impl IntoResponse> {
    let audience = audience_or_default(query.audience.as_deref());

    let row = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT d.id, d.slug
             FROM directory_surveys s
             JOIN directories d ON d.id = s.directory_id
            WHERE s.audience = $1 AND s.enabled = true AND s.status = 'published'
            ORDER BY s.updated_at DESC, d.created_at
            LIMIT 1"#,
    )
    .bind(&audience)
    .fetch_optional(&s.db)
    .await?;

    match row {
        Some((directory_id, slug)) => match published_survey(&s.db, directory_id, &audience).await?
        {
            Some(c) => Ok(Json(public_json(&c, &slug))),
            None => Ok(Json(empty_public_json(audience))),
        },
        None => {
            // Compatibility: questionnaires authored before the builder kept every audience in
            // ONE row and marked each question with a `tags` array. Rather than silently
            // dropping supplier/business onboarding that used to work, serve that row's
            // questions for this audience until the admin authors a per-audience questionnaire.
            let legacy = sqlx::query_as::<_, (Uuid, String, Value)>(
                r#"SELECT d.id, d.slug, s.questions
                     FROM directory_surveys s
                     JOIN directories d ON d.id = s.directory_id
                    WHERE s.enabled = true AND s.status = 'published'
                      AND s.questions @> $1::jsonb
                    ORDER BY s.updated_at DESC, d.created_at
                    LIMIT 1"#,
            )
            .bind(json!([{ "tags": [audience.clone()] }]))
            .fetch_optional(&s.db)
            .await?;

            match legacy {
                Some((directory_id, slug, questions)) => {
                    match published_survey(&s.db, directory_id, "customer").await? {
                        Some(c) => {
                            // Keep only the questions tagged for this audience.
                            let filtered: Vec<Value> = normalize_questions(&questions)
                                .ok()
                                .and_then(|v| v.as_array().cloned())
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|q| {
                                    q.get("tags")
                                        .and_then(|t| t.as_array())
                                        .map(|tags| {
                                            tags.is_empty()
                                                || tags
                                                    .iter()
                                                    .any(|t| t.as_str() == Some(audience.as_str()))
                                        })
                                        .unwrap_or(true)
                                })
                                .collect();
                            let mut cfg = c.clone();
                            cfg.questions = Value::Array(filtered);
                            Ok(Json(public_json(&cfg, &slug)))
                        }
                        None => Ok(Json(empty_public_json(audience))),
                    }
                }
                None => Ok(Json(empty_public_json(audience))),
            }
        }
    }
}

// ── Public: Submit Survey Response ──────────────────────────────────────────

/// Who answered, when we can tell. Currency can only be credited to a real account, so an
/// unidentified respondent is stored with `awarded: 0` rather than credited to nobody.
struct Respondent {
    visitor_account_id: Uuid,
    email: Option<String>,
    name: Option<String>,
    phone: Option<String>,
}

/// Resolve the respondent from (in order) an optional visitor bearer token — the visitor and
/// business portals hold one — or an explicit `visitor_account_id` in the body.
fn visitor_from_headers(headers: &HeaderMap, secret: &str) -> Option<Uuid> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?
        .trim();
    let claims = crate::auth::middleware::verify_token(token, secret).ok()?;
    if claims.role != "visitor" {
        return None;
    }
    Uuid::parse_str(&claims.sub).ok()
}

async fn load_respondent(db: &sqlx::PgPool, visitor_account_id: Uuid) -> Option<Respondent> {
    let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT email, name, phone FROM visitor_accounts WHERE id = $1",
    )
    .bind(visitor_account_id)
    .fetch_optional(db)
    .await
    .ok()?;
    row.map(|(email, name, phone)| Respondent {
        visitor_account_id,
        email: Some(email),
        name,
        phone,
    })
}

fn answer_is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty() || a.iter().all(answer_is_empty),
        _ => false,
    }
}

/// Accept both answer shapes a client may send: the array the widget posts, or a plain
/// `{question_id: value}` object. Everything is stored as one array shape.
fn normalize_answers(raw: &Value) -> Vec<Value> {
    match raw {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|item| match item {
                Value::Object(o) => {
                    let qid = o
                        .get("question_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if qid.is_empty() {
                        return None;
                    }
                    let mut out = Map::new();
                    out.insert("question_id".into(), json!(qid));
                    out.insert(
                        "question_label".into(),
                        o.get("question_label")
                            .or_else(|| o.get("question"))
                            .cloned()
                            .unwrap_or(json!("")),
                    );
                    out.insert(
                        "type".into(),
                        o.get("type").cloned().unwrap_or(json!("short_text")),
                    );
                    out.insert(
                        "value".into(),
                        o.get("value").cloned().unwrap_or(Value::Null),
                    );
                    let tags: Vec<String> = o
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    out.insert("tags".into(), json!(tags));
                    Some(Value::Object(out))
                }
                other => {
                    if other.is_null() {
                        None
                    } else {
                        None
                    }
                }
            })
            .collect(),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| {
                json!({
                    "question_id": k,
                    "question_label": "",
                    "type": "short_text",
                    "value": v,
                    "tags": [],
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A question's `required` flag is enforced against the answers actually received, so a
/// half-filled form is rejected with the labels it is missing instead of stored incomplete.
fn missing_required(config: &SurveyConfig, answers: &[Value]) -> Vec<String> {
    let Some(questions) = normalize_questions(&config.questions)
        .ok()
        .and_then(|v| v.as_array().cloned())
    else {
        return Vec::new();
    };

    let mut missing = Vec::new();
    for q in questions {
        if !q.get("required").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let Some(qid) = q.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let answered = answers.iter().any(|a| {
            a.get("question_id").and_then(|v| v.as_str()) == Some(qid)
                && !answer_is_empty(a.get("value").unwrap_or(&Value::Null))
        });
        if !answered {
            let label = q
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(qid)
                .to_string();
            missing.push(label);
        }
    }
    missing
}

/// A string value for a hub custom field (arrays are joined, objects serialised).
fn field_value_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(field_value_string)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The CoreSwift contact body for an onboarding response: identity where we have it, and one
/// custom field per answered question (the hub auto-provisions the fields per tenant).
fn lead_from_answers(
    audience: &str,
    title: &str,
    respondent: Option<&Respondent>,
    answers: &[Value],
    tags: &[String],
) -> LeadPayload {
    let mut lead = LeadPayload {
        email: respondent.and_then(|r| r.email.clone()),
        phone: respondent.and_then(|r| r.phone.clone()),
        name: respondent.and_then(|r| r.name.clone()),
        tags: tags.to_vec(),
        notes: Some(format!(
            "Onboarding questionnaire '{title}' completed ({audience}) via Multi-Directory"
        )),
        ..Default::default()
    };

    let mut used_keys: Vec<String> = Vec::new();
    for a in answers {
        let value = a.get("value").cloned().unwrap_or(Value::Null);
        if answer_is_empty(&value) {
            continue;
        }
        let qid = a.get("question_id").and_then(|v| v.as_str()).unwrap_or("");
        let label = a
            .get("question_label")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(qid);
        let flat = field_value_string(&value);

        // Identity callouts: a question that plainly asks for an email/phone/name fills the
        // hub contact's real fields as well as the custom field.
        let probe = format!(
            "{} {}",
            qid.to_ascii_lowercase(),
            label.to_ascii_lowercase()
        );
        if lead.email.is_none() && probe.contains("email") && flat.contains('@') {
            lead.email = Some(flat.clone());
        }
        if lead.phone.is_none()
            && (probe.contains("phone") || probe.contains("mobile") || probe.contains("telephone"))
            && flat.chars().filter(|c| c.is_ascii_digit()).count() >= 7
        {
            lead.phone = Some(flat.clone());
        }
        if lead.name.is_none() && probe.contains("name") && !probe.contains("business") {
            lead.name = Some(flat.clone());
        }
        if lead.company.is_none() && (probe.contains("company") || probe.contains("business name"))
        {
            lead.company = Some(flat.clone());
        }

        let mut key = slug_key(label);
        if key == "field" {
            key = slug_key(qid);
        }
        let mut unique = key.clone();
        let mut n = 2;
        while used_keys.contains(&unique) {
            unique = format!("{key}_{n}");
            n += 1;
        }
        used_keys.push(unique.clone());
        lead.fields.insert(unique, json!(flat));
    }

    lead
}

/// POST /api/v1/public/directories/:slug/survey/respond
pub async fn public_submit_survey(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SubmitSurveyRequest>,
) -> ApiResult<impl IntoResponse> {
    // Resolve directory by slug
    let dir =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM directories WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let directory_id = dir.0;
    let directory_name = dir.1;
    let audience = audience_or_default(req.audience.as_deref());

    // Look up the PUBLISHED questionnaire for this audience
    let config = sqlx::query_as::<_, SurveyConfig>(
        r#"SELECT * FROM directory_surveys
            WHERE directory_id = $1 AND audience = $2 AND enabled = true AND status = 'published'"#,
    )
    .bind(directory_id)
    .bind(&audience)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| {
        AppError::BadRequest(format!(
            "No published onboarding questionnaire for the '{audience}' audience in this directory"
        ))
    })?;

    // Normalise the answers into the stored array shape
    let answers = normalize_answers(&req.answers);
    if answers.is_empty() {
        return Err(AppError::Validation(
            "No answers were submitted".to_string(),
        ));
    }

    // Required questions must actually be answered
    let missing = missing_required(&config, &answers);
    if !missing.is_empty() {
        return Err(AppError::Validation(format!(
            "These questions are required: {}",
            missing.join(", ")
        )));
    }

    // ── Tags: the questionnaire's own tags, plus per-answer tags the form attached ──
    let mut all_tags: Vec<String> = config
        .completion_tags
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for a in &answers {
        if let Some(tags) = a.get("tags").and_then(|v| v.as_array()) {
            for t in tags.iter().filter_map(|v| v.as_str()) {
                let t = t.trim();
                if !t.is_empty() && !all_tags.iter().any(|existing| existing == t) {
                    all_tags.push(t.to_string());
                }
            }
        }
    }

    // ── Who is answering? (visitor token, else an explicit visitor id) ──
    let respondent = match visitor_from_headers(&headers, &s.config.jwt_secret) {
        Some(vid) => load_respondent(&s.db, vid).await,
        None => match req.visitor_account_id {
            Some(vid) => load_respondent(&s.db, vid).await,
            None => None,
        },
    };
    let respondent_id = respondent.as_ref().map(|r| r.visitor_account_id);

    // ── One response per respondent per questionnaire: re-submitting must not double-credit ──
    if let Some(vid) = respondent_id {
        let existing = sqlx::query_as::<_, (Uuid, i32, Option<String>, DateTime<Utc>)>(
            r#"SELECT id, reward_units_awarded, currency_name, completed_at
                 FROM survey_responses
                WHERE survey_id = $1 AND visitor_account_id = $2
                ORDER BY completed_at DESC LIMIT 1"#,
        )
        .bind(config.id)
        .bind(vid)
        .fetch_optional(&s.db)
        .await?;

        if let Some((id, awarded, currency, completed_at)) = existing {
            return Ok(Json(json!({
                "id": id,
                "survey_id": config.id,
                "audience": audience,
                "completed_at": completed_at,
                "already_completed": true,
                "reward": {
                    "status": if awarded > 0 { "already_credited" } else { "nothing_to_credit" },
                    "units": awarded,
                    "currency": currency,
                },
                "message": "You have already completed this questionnaire.",
            })));
        }
    }

    // ── Store the response. This is the step that must never fail silently. ──
    let response = sqlx::query_as::<_, SurveyResponse>(
        r#"INSERT INTO survey_responses
            (survey_id, visitor_account_id, visitor_fingerprint, directory_id, answers,
             applied_tags, audience)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING *"#,
    )
    .bind(config.id)
    .bind(respondent_id)
    .bind(&req.visitor_fingerprint)
    .bind(directory_id)
    .bind(Value::Array(answers.clone()))
    .bind(&all_tags)
    .bind(&audience)
    .fetch_one(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("Could not store the onboarding answers: {e}")))?;

    // ── Award natively (the loyalty engine, not a campaign) ──
    let mut reward = json!({
        "status": "not_identified",
        "units": 0,
        "currency": Value::Null,
        "detail": "No visitor account was identified, so nothing could be credited. The answers were still stored.",
    });
    if let Some(vid) = respondent_id {
        let description = format!("Onboarding: {} ({})", config.title, audience);
        match crate::handlers::loyalty_native::credit_visitor_units(
            &s.db,
            &directory_id,
            &vid,
            config.reward_units,
            "onboarding_survey",
            &description,
        )
        .await
        {
            Ok(Some(award)) => {
                if award.units > 0 {
                    sqlx::query(
                        "UPDATE survey_responses SET reward_units_awarded = $1, currency_name = $2 WHERE id = $3",
                    )
                    .bind(award.units)
                    .bind(&award.currency_name)
                    .bind(response.id)
                    .execute(&s.db)
                    .await
                    .map_err(|e| {
                        // The currency is already credited; a bookkeeping failure must be loud.
                        tracing::error!(
                            "[onboarding] credited {} {} but could not record it on response {}: {}",
                            award.units,
                            award.currency_name,
                            response.id,
                            e
                        );
                    })
                    .ok();
                }
                reward = json!({
                    "status": if award.units > 0 { "credited" } else { "nothing_to_credit" },
                    "units": award.units,
                    "currency": award.currency_name,
                    "icon": award.currency_icon,
                    "balance_after": award.balance_after,
                    "program_id": award.program_id,
                });
                tracing::info!(
                    "[onboarding] response {} (audience {}) credited {} {} to visitor {}",
                    response.id,
                    audience,
                    award.units,
                    award.currency_name,
                    vid
                );
            }
            Ok(None) => {
                reward = json!({
                    "status": "skipped_no_programme",
                    "units": 0,
                    "currency": Value::Null,
                    "detail": "No active loyalty programme governs this directory, so there is nothing to credit yet.",
                });
                tracing::warn!(
                    "[onboarding] response {} — no active loyalty programme for directory {}, nothing credited",
                    response.id,
                    directory_id
                );
            }
            Err(e) => {
                reward = json!({
                    "status": "failed",
                    "units": 0,
                    "currency": Value::Null,
                    "detail": format!("The reward could not be credited: {e}"),
                });
                tracing::error!(
                    "[onboarding] response {} — crediting failed for visitor {}: {}",
                    response.id,
                    vid,
                    e
                );
            }
        }
    }

    // ── Map the answers into CoreSwift (DB-held credentials, graceful failure) ──
    let lead = lead_from_answers(
        &audience,
        &config.title,
        respondent.as_ref(),
        &answers,
        &all_tags,
    );
    let has_identity = lead
        .email
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || lead
            .phone
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
        || lead
            .name
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

    let (pushed, push_error, coreswift_contact_id): (bool, Option<String>, Option<Uuid>) =
        if !has_identity {
            (
                false,
                Some(
                    "skipped: the response carries no email, phone or name to identify the contact"
                        .to_string(),
                ),
                None,
            )
        } else {
            match tokio::time::timeout(
                std::time::Duration::from_secs(8),
                crate::coreswift::push_lead_to_coreswift(&s.db, None, Some(directory_id), lead),
            )
            .await
            {
                Err(_) => (
                    false,
                    Some("CoreSwift push timed out after 8s".to_string()),
                    None,
                ),
                Ok(Ok(contact_id)) => (true, None, contact_id),
                Ok(Ok(None)) => (
                    false,
                    Some("skipped: no CoreSwift connection for this directory".to_string()),
                    None,
                ),
                Ok(Err(e)) => (false, Some(e), None),
            }
        };

    if let Some(reason) = &push_error {
        tracing::warn!(
            "[onboarding] response {} not pushed to CoreSwift — {}",
            response.id,
            reason
        );
    } else {
        tracing::info!(
            "[onboarding] response {} pushed to CoreSwift (audience {}, directory {})",
            response.id,
            audience,
            directory_name
        );
    }

    sqlx::query(
        "UPDATE survey_responses SET coreswift_pushed = $1, coreswift_push_error = $2, \
         coreswift_contact_id = COALESCE($4, coreswift_contact_id) WHERE id = $3",
    )
    .bind(pushed)
    .bind(&push_error)
    .bind(response.id)
    .bind(coreswift_contact_id)
    .execute(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("Could not record the CRM push result: {e}")))?;

    // Carry the identity across: the same person is ONE contact in CoreSwift, linked to
    // their Multi-Directory account (and the response the admin drills into in card B68).
    if let (Some(contact_id), Some(visitor_id)) = (coreswift_contact_id, respondent_id) {
        sqlx::query(
            "UPDATE visitor_accounts SET coreswift_contact_id = COALESCE(coreswift_contact_id, $1) \
             WHERE id = $2",
        )
        .bind(contact_id)
        .bind(visitor_id)
        .execute(&s.db)
        .await
        .ok();
    }

    // ── Newsletter opt-in straight from the questionnaire (native, in-house) ──
    if let Some(visitor_id) = respondent_id {
        let wants_newsletter = answers.iter().any(|a| {
            let qid = a
                .get("question_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let value = a.get("value").cloned().unwrap_or(Value::Null);
            let flat = field_value_string(&value).to_ascii_lowercase();
            (qid == "visitor_channel" && flat.contains("weekly email digest"))
                || (qid.contains("newsletter") && !answer_is_empty(&value) && flat != "no")
        });
        if wants_newsletter {
            if let Some(email) = respondent.as_ref().and_then(|r| r.email.clone()) {
                let newsletter_tag = format!("{}-zh-newsletter", directory_slug_to_code(&slug));
                if !all_tags.contains(&newsletter_tag) {
                    all_tags.push(newsletter_tag.clone());
                    sqlx::query("UPDATE survey_responses SET applied_tags = $1 WHERE id = $2")
                        .bind(&all_tags)
                        .bind(response.id)
                        .execute(&s.db)
                        .await
                        .ok();
                }
                match sqlx::query(
                    r#"INSERT INTO newsletter_subscribers (directory_id, email, name, status)
                       VALUES ($1, $2, '', 'active')
                       ON CONFLICT (directory_id, email) DO NOTHING"#,
                )
                .bind(directory_id)
                .bind(&email)
                .execute(&s.db)
                .await
                {
                    Ok(r) if r.rows_affected() > 0 => tracing::info!(
                        "[newsletter] Auto-subscribed {} to directory {}",
                        email,
                        directory_id
                    ),
                    Ok(_) => tracing::info!(
                        "[newsletter] {} already subscribed to directory {}",
                        email,
                        directory_id
                    ),
                    Err(e) => {
                        tracing::warn!("[newsletter] Failed to auto-subscribe {}: {}", email, e)
                    }
                }
            }
        }
    }

    // Nudge the home/loyalty views: the visitor's own dashboard is the natural next stop.
    if let Some(vid) = respondent_id {
        sqlx::query("UPDATE visitor_accounts SET survey_answered_at = NOW() WHERE id = $1")
            .bind(vid)
            .execute(&s.db)
            .await
            .ok();
    }

    Ok(Json(json!({
        "id": response.id,
        "survey_id": response.survey_id,
        "audience": response.audience,
        "completed_at": response.completed_at,
        "applied_tags": all_tags,
        "already_completed": false,
        "reward": reward,
        "coreswift": {
            "pushed": pushed,
            "detail": push_error,
        },
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
        r#"SELECT COALESCE(feature_config, '{}'::jsonb) FROM directories WHERE id = $1"#,
    )
    .bind(directory_id)
    .fetch_one(db)
    .await
    .unwrap_or(json!({}));

    let mut config = current_config.as_object().cloned().unwrap_or_default();
    config.insert("onboarding_survey".to_string(), json!(enabled));
    let new_config = Value::Object(config);

    sqlx::query(r#"UPDATE directories SET feature_config = $1, updated_at = NOW() WHERE id = $2"#)
        .bind(&new_config)
        .bind(directory_id)
        .execute(db)
        .await?;

    Ok(())
}

/// Map a directory slug to its short city code for ZaarHub newsletter tags.
/// Format: {code}-zh-newsletter (e.g. pc-zh-newsletter, pb-zh-newsletter)
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
