//! Micro-polls: one-question polls for directories.
//! Visitors can vote once per poll; admins create/close polls.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::AppState;
use crate::auth::middleware::verify_token;
use crate::error::{AppError, ApiResult};

// ── Auth Helpers ──

/// Extract admin user ID from JWT (requires auth_guard to have run first).
/// Falls back to manual header extraction for routes outside auth_guard scope.
pub fn extract_admin_id(headers: &HeaderMap, jwt_secret: &str) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    // Verify the user has admin role
    if claims.role != "admin" && claims.role != "superadmin" {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

/// Extract visitor account ID from JWT (same as visitors.rs helper).
/// Returns 401 if missing/invalid.
pub fn extract_visitor_account_id(headers: &HeaderMap, jwt_secret: &str) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

// ── Data Types ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Poll {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub question: String,
    pub options: Vec<String>,
    pub created_by: Uuid,
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PollWithCounts {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub question: String,
    pub options: Vec<String>,
    pub created_by: Uuid,
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub total_votes: i64,
    pub option_votes: Vec<i64>,
    pub user_vote: Option<i32>, // the option_index the calling user voted for, if any
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PollVote {
    pub id: Uuid,
    pub poll_id: Uuid,
    pub visitor_account_id: Uuid,
    pub option_index: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePollRequest {
    pub directory_id: Uuid,
    pub question: String,
    pub options: Vec<String>,
    pub ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CastVoteRequest {
    pub option_index: i32,
}

#[derive(Debug, Deserialize)]
pub struct PollListQuery {
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
}

// ── Handlers ──

/// POST /api/v1/polls — create a poll (admin only)
pub async fn create_poll(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreatePollRequest>,
) -> ApiResult<impl IntoResponse> {
    let admin_id = extract_admin_id(&headers, &s.config.jwt_secret)?;

    if req.question.trim().is_empty() {
        return Err(AppError::Validation("Question is required".to_string()));
    }
    if req.options.len() < 2 {
        return Err(AppError::Validation("At least 2 options required".to_string()));
    }
    if req.options.len() > 20 {
        return Err(AppError::Validation("Maximum 20 options allowed".to_string()));
    }

    // Verify directory exists
    let dir_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = $1"
    )
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0) > 0;

    if !dir_exists {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    let poll = sqlx::query_as::<_, Poll>(
        r#"INSERT INTO polls (directory_id, question, options, created_by, ends_at)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, directory_id, question, options, created_by, status, starts_at, ends_at, created_at"#
    )
    .bind(req.directory_id)
    .bind(req.question.trim())
    .bind(&req.options)
    .bind(admin_id)
    .bind(req.ends_at)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({
        "poll": poll,
        "message": "Poll created successfully"
    }))))
}

/// GET /api/v1/polls?directory_id=X&status=active — list polls for a directory
pub async fn list_polls(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PollListQuery>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_id_optional(&headers, &s.config.jwt_secret);

    let mut polls: Vec<Poll> = if let Some(dir_id) = q.directory_id {
        if let Some(ref status) = q.status {
            sqlx::query_as::<_, Poll>(
                "SELECT * FROM polls WHERE directory_id = $1 AND status = $2 ORDER BY created_at DESC"
            )
            .bind(dir_id)
            .bind(status)
            .fetch_all(&s.db)
            .await?
        } else {
            sqlx::query_as::<_, Poll>(
                "SELECT * FROM polls WHERE directory_id = $1 ORDER BY created_at DESC"
            )
            .bind(dir_id)
            .fetch_all(&s.db)
            .await?
        }
    } else if let Some(ref status) = q.status {
        sqlx::query_as::<_, Poll>(
            "SELECT * FROM polls WHERE status = $1 ORDER BY created_at DESC"
        )
        .bind(status)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, Poll>(
            "SELECT * FROM polls ORDER BY created_at DESC"
        )
        .fetch_all(&s.db)
        .await?
    };

    // Attach vote counts for each poll
    let mut result: Vec<PollWithCounts> = Vec::new();
    for poll in polls {
        let total_votes = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM poll_votes WHERE poll_id = $1"
        )
        .bind(poll.id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        // Get per-option vote counts
        let mut option_votes: Vec<i64> = vec![0i64; poll.options.len()];
        let vote_rows = sqlx::query_as::<_, (i32, i64)>(
            "SELECT option_index, COUNT(*)::bigint FROM poll_votes WHERE poll_id = $1 GROUP BY option_index"
        )
        .bind(poll.id)
        .fetch_all(&s.db)
        .await
        .unwrap_or_default();

        for (idx, count) in vote_rows {
            if (idx as usize) < option_votes.len() {
                option_votes[idx as usize] = count;
            }
        }

        // Check if this visitor has voted
        let user_vote: Option<i32> = if let Some(vid) = visitor_id {
            sqlx::query_scalar::<_, i32>(
                "SELECT option_index FROM poll_votes WHERE poll_id = $1 AND visitor_account_id = $2"
            )
            .bind(poll.id)
            .bind(vid)
            .fetch_optional(&s.db)
            .await
            .unwrap_or(None)
        } else {
            None
        };

        result.push(PollWithCounts {
            id: poll.id,
            directory_id: poll.directory_id,
            question: poll.question,
            options: poll.options,
            created_by: poll.created_by,
            status: poll.status,
            starts_at: poll.starts_at,
            ends_at: poll.ends_at,
            created_at: poll.created_at,
            total_votes,
            option_votes,
            user_vote: user_vote.map(|v| v as i32),
        });
    }

    Ok(Json(json!({
        "polls": result,
        "count": result.len(),
    })))
}

/// GET /api/v1/polls/{id} — get single poll with results
pub async fn get_poll(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(poll_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_id_optional(&headers, &s.config.jwt_secret);

    let poll = sqlx::query_as::<_, Poll>(
        "SELECT * FROM polls WHERE id = $1"
    )
    .bind(poll_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Poll not found".to_string()))?;

    let total_votes = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM poll_votes WHERE poll_id = $1"
    )
    .bind(poll.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Get per-option vote counts
    let mut option_votes: Vec<i64> = vec![0i64; poll.options.len()];
    let vote_rows = sqlx::query_as::<_, (i32, i64)>(
        "SELECT option_index, COUNT(*)::bigint FROM poll_votes WHERE poll_id = $1 GROUP BY option_index"
    )
    .bind(poll.id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    for (idx, count) in vote_rows {
        if (idx as usize) < option_votes.len() {
            option_votes[idx as usize] = count;
        }
    }

    let user_vote: Option<i32> = if let Some(vid) = visitor_id {
        sqlx::query_scalar::<_, i32>(
            "SELECT option_index FROM poll_votes WHERE poll_id = $1 AND visitor_account_id = $2"
        )
        .bind(poll.id)
        .bind(vid)
        .fetch_optional(&s.db)
        .await
        .unwrap_or(None)
    } else {
        None
    };

    Ok(Json(json!({
        "poll": PollWithCounts {
            id: poll.id,
            directory_id: poll.directory_id,
            question: poll.question,
            options: poll.options,
            created_by: poll.created_by,
            status: poll.status,
            starts_at: poll.starts_at,
            ends_at: poll.ends_at,
            created_at: poll.created_at,
            total_votes,
            option_votes,
            user_vote: user_vote.map(|v| v as i32),
        }
    })))
}

/// POST /api/v1/polls/{id}/vote — cast or update a vote (visitor auth)
pub async fn cast_vote(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(poll_id): Path<Uuid>,
    Json(req): Json<CastVoteRequest>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_account_id(&headers, &s.config.jwt_secret)?;

    // Verify the poll exists and is active
    let poll = sqlx::query_as::<_, Poll>(
        "SELECT * FROM polls WHERE id = $1"
    )
    .bind(poll_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Poll not found".to_string()))?;

    if poll.status != "active" {
        return Err(AppError::BadRequest("Poll is not active".to_string()));
    }

    // Check if poll has ended
    if let Some(ends_at) = poll.ends_at {
        if Utc::now() > ends_at {
            return Err(AppError::BadRequest("Poll has ended".to_string()));
        }
    }

    // Validate option_index
    if req.option_index < 0 || (req.option_index as usize) >= poll.options.len() {
        return Err(AppError::Validation("Invalid option index".to_string()));
    }

    // Upsert: delete existing vote then insert new one
    sqlx::query(
        "DELETE FROM poll_votes WHERE poll_id = $1 AND visitor_account_id = $2"
    )
    .bind(poll_id)
    .bind(visitor_id)
    .execute(&s.db)
    .await?;

    sqlx::query(
        "INSERT INTO poll_votes (poll_id, visitor_account_id, option_index) VALUES ($1, $2, $3)"
    )
    .bind(poll_id)
    .bind(visitor_id)
    .bind(req.option_index)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "message": "Vote recorded",
        "poll_id": poll_id,
        "option_index": req.option_index,
    })))
}

/// POST /api/v1/polls/{id}/close — close a poll (admin/creator only)
pub async fn close_poll(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(poll_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let admin_id = extract_admin_id(&headers, &s.config.jwt_secret)?;

    // Verify the poll exists and user is the creator
    let poll = sqlx::query_as::<_, Poll>(
        "SELECT * FROM polls WHERE id = $1"
    )
    .bind(poll_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Poll not found".to_string()))?;

    if poll.created_by != admin_id {
        return Err(AppError::Forbidden("Only the poll creator can close it".to_string()));
    }

    if poll.status != "active" {
        return Err(AppError::BadRequest("Poll is already closed".to_string()));
    }

    sqlx::query(
        "UPDATE polls SET status = 'closed' WHERE id = $1"
    )
    .bind(poll_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "message": "Poll closed",
        "poll_id": poll_id,
    })))
}

/// Helper: extract visitor ID if present (no error if missing).
/// Mirrors the function in visitors.rs
fn extract_visitor_id_optional(headers: &HeaderMap, jwt_secret: &str) -> Option<Uuid> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())?;

    let token = auth_header.strip_prefix("Bearer ")?;

    let claims = verify_token(token, jwt_secret).ok()?;

    Uuid::parse_str(&claims.sub).ok()
}
