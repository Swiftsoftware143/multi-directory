//! Local Q&A on a business listing (card B90, Nextdoor-style "ask the community").
//!
//! The public side is what a visitor sees on `/biz/:id`: every answered (and unanswered) question
//! a directory has chosen to publish, plus an anonymous ask box. The admin side is the
//! directory-scoped queue a non-technical operator works from the ZaarHub admin panel — answer,
//! hide or delete — with no SQL and no shell (the sellable-standard bar of card B84).
//!
//! Mirrors the report-a-listing handler (card B90) exactly: the anonymous write is rate-limited
//! per client address with a global ceiling, the business is resolved before anything is written
//! (so an unknown id 404s and a probe learns nothing), and the caller's email is stored for
//! follow-up but never leaves the admin queue.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::{validate_pagination, ApiResult, AppError};
use crate::handlers::tenant_scope::{
    assert_directory_admin, caller_tenant, claims_from_headers, is_platform_operator,
};
use crate::AppState;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref QUESTION_LIMITER: Mutex<HashMap<String, Vec<Instant>>> = Mutex::new(HashMap::new());
}

const QUESTION_MAX_PER_HOUR_PER_KEY: usize = 8;
const QUESTION_MAX_PER_HOUR_GLOBAL: usize = 200;

/// Sliding-window guard for `POST /businesses/:id/questions` (an anonymous write). Keyed by the
/// caller's address with a global hourly ceiling, so a stuck client cannot flood the queue.
fn check_question_rate(key: &str) -> Result<(), AppError> {
    let now = Instant::now();
    let window = Duration::from_secs(3600);
    let mut map = QUESTION_LIMITER
        .lock()
        .map_err(|_| AppError::Internal("question rate limiter unavailable".into()))?;

    for v in map.values_mut() {
        v.retain(|t| now.duration_since(*t) < window);
    }

    let global = map.get("__global__").map(|v| v.len()).unwrap_or(0);
    let per_key = map.get(key).map(|v| v.len()).unwrap_or(0);
    if global >= QUESTION_MAX_PER_HOUR_GLOBAL || per_key >= QUESTION_MAX_PER_HOUR_PER_KEY {
        return Err(AppError::TooManyRequests(
            "too many questions from this address — try again later".into(),
        ));
    }

    map.entry(key.to_string()).or_default().push(now);
    map.entry("__global__".to_string()).or_default().push(now);
    Ok(())
}

const QUESTION_STATUSES: [&str; 2] = ["published", "hidden"];

/// Full row, as the admin queue sees it (includes the asker email for follow-up).
#[derive(Debug, Serialize, FromRow)]
pub struct BusinessQuestion {
    pub id: Uuid,
    pub business_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub asker_name: Option<String>,
    pub asker_email: Option<String>,
    pub question: String,
    pub answer: Option<String>,
    pub answered_at: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: Option<DateTime<Utc>>,
}

/// Admin row plus the listing name, so the panel can show *which* business a question is about.
#[derive(Debug, Serialize, FromRow)]
pub struct BusinessQuestionAdmin {
    pub id: Uuid,
    pub business_id: Uuid,
    pub directory_id: Option<Uuid>,
    pub asker_name: Option<String>,
    pub asker_email: Option<String>,
    pub question: String,
    pub answer: Option<String>,
    pub answered_at: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: Option<DateTime<Utc>>,
    pub business_name: Option<String>,
}

/// Public projection — deliberately drops `asker_email` and the moderation fields.
#[derive(Debug, Serialize, FromRow)]
pub struct PublicQuestion {
    pub id: Uuid,
    pub asker_name: Option<String>,
    pub question: String,
    pub answer: Option<String>,
    pub answered_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateQuestionRequest {
    pub question: String,
    pub asker_name: Option<String>,
    pub asker_email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnswerQuestionRequest {
    pub answer: Option<String>,
    pub status: Option<String>,
}

fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// POST /api/v1/businesses/:id/questions — public, rate-limited. Publishes immediately; a
/// moderator can hide or delete it afterwards. 404 for an unknown business, 422 for a question
/// outside the length window.
pub async fn create_question(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CreateQuestionRequest>,
) -> ApiResult<impl IntoResponse> {
    let question: String = req.question.trim().chars().take(1500).collect();
    if question.chars().count() < 5 {
        return Err(AppError::Validation(
            "Please write a question of at least 5 characters.".into(),
        ));
    }

    let asker_name: Option<String> = req
        .asker_name
        .map(|n| n.trim().chars().take(120).collect::<String>())
        .filter(|n| !n.is_empty());
    let asker_email: Option<String> = req
        .asker_email
        .map(|e| e.trim().chars().take(254).collect::<String>())
        .filter(|e| !e.is_empty());

    // Resolve the business (and its directory) before anything is written.
    let row = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT id, directory_id FROM businesses WHERE id = $1",
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?;
    let (biz_id, directory_id) =
        row.ok_or_else(|| AppError::NotFound("Business not found".into()))?;

    check_question_rate(&client_key(&headers))?;

    let q = sqlx::query_as::<_, BusinessQuestion>(
        r#"INSERT INTO business_questions
               (business_id, directory_id, asker_name, asker_email, question, status)
           VALUES ($1, $2, $3, $4, $5, 'published')
           RETURNING *"#,
    )
    .bind(biz_id)
    .bind(directory_id)
    .bind(asker_name)
    .bind(asker_email)
    .bind(&question)
    .fetch_one(&s.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message": "Thanks — your question is posted. Check back for an answer.",
            "id": q.id,
        })),
    ))
}

/// GET /api/v1/businesses/:id/questions — public list of published questions for one listing,
/// answered questions first (newest answer last), then the still-open questions newest first.
pub async fn list_questions(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound("Business not found".into()));
    }

    let rows = sqlx::query_as::<_, PublicQuestion>(concat!(
        "SELECT id, asker_name, question, answer, answered_at, created_at ",
        "FROM business_questions ",
        "WHERE business_id = $1 AND status = 'published' ",
        "ORDER BY (answer IS NOT NULL) DESC, answered_at DESC NULLS LAST, created_at DESC"
    ))
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "data": rows,
        "count": rows.len(),
    })))
}

/// GET /api/v1/admin/questions?status=&directory_id=&page=&per_page= — the moderation queue.
/// A directory owner sees only their own directories' questions; the platform operator sees all.
pub async fn list_admin_questions(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;

    let requested_dir = params
        .get("directory_id")
        .and_then(|v| Uuid::parse_str(v).ok());
    if let Some(d) = requested_dir {
        assert_directory_admin(&s.db, &claims, d).await?;
    }
    let is_op = is_platform_operator(&claims);
    let tenant = if is_op {
        Uuid::nil()
    } else {
        caller_tenant(&claims)?
    };
    let status_filter = params.get("status").cloned().filter(|v| !v.is_empty());

    let page = params
        .get("page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT COUNT(*) FROM business_questions q WHERE ",
        "($1::text IS NULL OR q.status = $1) ",
        "AND ($2::uuid IS NULL OR q.directory_id = $2) ",
        "AND ($3::bool OR q.directory_id IN (SELECT d.id FROM directories d ",
        "JOIN users u ON u.id = d.owner_id WHERE u.tenant_id = $4))"
    ))
    .bind(status_filter.as_deref())
    .bind(requested_dir)
    .bind(is_op)
    .bind(tenant)
    .fetch_one(&s.db)
    .await?;

    let rows = sqlx::query_as::<_, BusinessQuestionAdmin>(concat!(
        "SELECT q.*, b.name AS business_name FROM business_questions q ",
        "LEFT JOIN businesses b ON b.id = q.business_id WHERE ",
        "($1::text IS NULL OR q.status = $1) ",
        "AND ($2::uuid IS NULL OR q.directory_id = $2) ",
        "AND ($3::bool OR q.directory_id IN (SELECT d.id FROM directories d ",
        "JOIN users u ON u.id = d.owner_id WHERE u.tenant_id = $4)) ",
        "ORDER BY (q.answer IS NULL) DESC, q.created_at DESC LIMIT $5 OFFSET $6"
    ))
    .bind(status_filter.as_deref())
    .bind(requested_dir)
    .bind(is_op)
    .bind(tenant)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!({
        "data": rows,
        "page": page,
        "per_page": per_page,
        "total": total,
        "total_pages": total_pages,
    })))
}

/// PATCH /api/v1/admin/questions/:id — set the answer (an empty answer clears it) and/or the
/// publish status from the panel.
pub async fn answer_question(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<AnswerQuestionRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;

    let status: Option<String> = match req.status {
        Some(v) => {
            let v = v.trim().to_lowercase();
            if !QUESTION_STATUSES.contains(&v.as_str()) {
                return Err(AppError::Validation(
                    "That question status is not recognised.".into(),
                ));
            }
            Some(v)
        }
        None => None,
    };

    let dir: Option<Uuid> =
        sqlx::query_scalar("SELECT directory_id FROM business_questions WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Question not found".into()))?;

    match dir {
        Some(d) => assert_directory_admin(&s.db, &claims, d).await?,
        None => {
            if !is_platform_operator(&claims) {
                return Err(AppError::NotFound("Question not found".into()));
            }
        }
    }

    // Trim (and cap) the answer; an explicitly empty answer clears it.
    let answer: Option<String> = req
        .answer
        .map(|a| a.trim().chars().take(4000).collect::<String>());

    let q = sqlx::query_as::<_, BusinessQuestion>(
        r#"UPDATE business_questions
              SET answer = CASE WHEN $1::text IS NULL THEN answer
                                WHEN btrim($1) = '' THEN NULL
                                ELSE btrim($1) END,
                  answered_at = CASE WHEN $1::text IS NULL THEN answered_at
                                     WHEN btrim($1) = '' THEN NULL
                                     ELSE now() END,
                  status = COALESCE($2, status)
            WHERE id = $3
        RETURNING *"#,
    )
    .bind(answer.as_deref())
    .bind(status.as_deref())
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(q)))
}

/// DELETE /api/v1/admin/questions/:id — remove a question entirely from the panel.
pub async fn delete_question(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;

    let dir: Option<Uuid> =
        sqlx::query_scalar("SELECT directory_id FROM business_questions WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Question not found".into()))?;

    match dir {
        Some(d) => assert_directory_admin(&s.db, &claims, d).await?,
        None => {
            if !is_platform_operator(&claims) {
                return Err(AppError::NotFound("Question not found".into()));
            }
        }
    }

    sqlx::query("DELETE FROM business_questions WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({ "message": "Question deleted." })))
}
