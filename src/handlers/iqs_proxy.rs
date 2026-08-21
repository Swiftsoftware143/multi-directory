//! IQS Proxy — routes portal IQS (Intelligent Qualifying Survey) requests
//! to IncentiveSwift's IQS backend.
//!
//! These routes sit INSIDE the auth guard (need MD JWT).
//! They resolve the MD user -> IS account (by email) and generate an IS-compatible
//! JWT on-the-fly to proxy the request through.
//!
//! Shared proxy utilities live in super::proxy_common.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use serde_json::Value;

use crate::auth::models::Claims;
use crate::error::ApiResult;
use crate::AppState;

use super::proxy_common::*;

// ── Funnels ──

/// GET /iqs/funnels — list all IQS funnels for the authenticated tenant
pub async fn list_funnels(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/iqs/funnels", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// POST /iqs/funnels — create a new IQS funnel
pub async fn create_funnel(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/iqs/funnels", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// GET /iqs/funnels/:id — get a single IQS funnel
pub async fn get_funnel(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(&format!("/iqs/funnels/{}", id), &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// PUT /iqs/funnels/:id — update an IQS funnel
pub async fn update_funnel(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_put(
        &format!("/iqs/funnels/{}", id),
        &body,
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// DELETE /iqs/funnels/:id — delete an IQS funnel
pub async fn delete_funnel(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_delete(&format!("/iqs/funnels/{}", id), &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// GET /iqs/funnels/:id/play — get funnel play data (by funnel ID)
pub async fn get_play_funnel(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(
        &format!("/iqs/funnels/{}/play", id),
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// POST /iqs/funnels/:id/submit — submit a funnel response
pub async fn submit_funnel(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post(
        &format!("/iqs/funnels/{}/submit", id),
        &body,
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

// ── Questions ──

/// GET /iqs/funnels/:id/questions — list questions for a funnel
pub async fn list_questions(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(
        &format!("/iqs/funnels/{}/questions", id),
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// POST /iqs/funnels/:id/questions — create a question in a funnel
pub async fn create_question(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post(
        &format!("/iqs/funnels/{}/questions", id),
        &body,
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// PUT /iqs/funnels/:id/questions/:question_id — update a question
pub async fn update_question(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path((funnel_id, question_id)): axum::extract::Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_put(
        &format!("/iqs/funnels/{}/questions/{}", funnel_id, question_id),
        &body,
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// DELETE /iqs/funnels/:id/questions/:question_id — delete a question
pub async fn delete_question(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path((funnel_id, question_id)): axum::extract::Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_delete(
        &format!("/iqs/funnels/{}/questions/{}", funnel_id, question_id),
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

// ── Submissions ──

/// GET /iqs/funnels/:id/submissions — list submissions for a funnel
pub async fn list_submissions(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(
        &format!("/iqs/funnels/{}/submissions", id),
        &aid,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// GET /campaigns/list — proxies to IS /api/v1/campaigns
/// Fetches the user's IncentiveSwift campaigns for the dropdown picker.
pub async fn list_campaigns(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/campaigns", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}
