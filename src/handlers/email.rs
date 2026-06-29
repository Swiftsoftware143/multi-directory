//! Email template and campaign CRUD handlers for Multi-Directory API.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct EmailTemplate {
    pub id: Uuid,
    pub name: String,
    pub subject: String,
    pub body: String,
    pub variables: Option<Vec<String>>,
    pub category: Option<String>,
    pub directory_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct EmailCampaign {
    pub id: Uuid,
    pub name: String,
    pub template_id: Option<Uuid>,
    pub recipient_filter: Option<serde_json::Value>,
    pub sent_count: Option<i32>,
    pub opened_count: Option<i32>,
    pub status: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub directory_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub subject: String,
    pub body: String,
    pub variables: Option<Vec<String>>,
    pub category: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub variables: Option<Vec<String>>,
    pub category: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCampaignRequest {
    pub name: String,
    pub template_id: Option<Uuid>,
    pub recipient_filter: Option<serde_json::Value>,
    pub status: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCampaignRequest {
    pub name: Option<String>,
    pub template_id: Option<Uuid>,
    pub recipient_filter: Option<serde_json::Value>,
    pub status: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub directory_id: Option<Uuid>,
}

// ==================== Handlers ====================

pub async fn list_templates(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let templates = sqlx::query_as::<_, EmailTemplate>(
        "SELECT id, name, subject, body, variables, category, directory_id, created_at, updated_at FROM email_templates ORDER BY created_at DESC")
    .fetch_all(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(templates)))
}

pub async fn get_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, EmailTemplate>(
        "SELECT id, name, subject, body, variables, category, directory_id, created_at, updated_at FROM email_templates WHERE id = \x241")
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email template not found")))?;
    Ok((StatusCode::OK, Json(template)))
}

pub async fn create_template(
    State(state): State<AppState>,
    Json(body): Json<CreateTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, EmailTemplate>(
        "INSERT INTO email_templates (name, subject, body, variables, category, directory_id) VALUES (\x241, \x242, \x243, \x244, \x245, \x246) RETURNING id, name, subject, body, variables, category, directory_id, created_at, updated_at")
    .bind(&body.name).bind(&body.subject).bind(&body.body)
    .bind(&body.variables).bind(&body.category).bind(body.directory_id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(template)))
}

pub async fn update_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    let _existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM email_templates WHERE id = \x241")
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email template not found")))?;

    let updated = sqlx::query_as::<_, EmailTemplate>(
        "UPDATE email_templates SET name = COALESCE(\x241, name), subject = COALESCE(\x242, subject), body = COALESCE(\x243, body), variables = COALESCE(\x244, variables), category = COALESCE(\x245, category), directory_id = COALESCE(\x246, directory_id) WHERE id = \x247 RETURNING id, name, subject, body, variables, category, directory_id, created_at, updated_at")
    .bind(&body.name).bind(&body.subject).bind(&body.body)
    .bind(&body.variables).bind(&body.category).bind(body.directory_id)
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(updated)))
}

pub async fn delete_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM email_templates WHERE id = \x241")
    .bind(id)
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(String::from("Email template not found")));
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"deleted": true}))))
}

pub async fn list_campaigns(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let campaigns = sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at FROM email_campaigns ORDER BY created_at DESC")
    .fetch_all(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(campaigns)))
}

pub async fn get_campaign(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let campaign = sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at FROM email_campaigns WHERE id = \x241")
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email campaign not found")))?;
    Ok((StatusCode::OK, Json(campaign)))
}

pub async fn create_campaign(
    State(state): State<AppState>,
    Json(body): Json<CreateCampaignRequest>,
) -> ApiResult<impl IntoResponse> {
    let campaign = sqlx::query_as::<_, EmailCampaign>(
        "INSERT INTO email_campaigns (name, template_id, recipient_filter, status, scheduled_at, directory_id) VALUES (\x241, \x242, \x243, \x244, \x245, \x246) RETURNING id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at ")
    .bind(&body.name).bind(body.template_id).bind(&body.recipient_filter)
    .bind(&body.status).bind(body.scheduled_at).bind(body.directory_id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(campaign)))
}

pub async fn update_campaign(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCampaignRequest>,
) -> ApiResult<impl IntoResponse> {
    let _existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM email_campaigns WHERE id = \x241")
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email campaign not found")))?;

    let updated = sqlx::query_as::<_, EmailCampaign>(
        "UPDATE email_campaigns SET name = COALESCE(\x241, name), template_id = COALESCE(\x242, template_id), recipient_filter = COALESCE(\x243, recipient_filter), status = COALESCE(\x244, status), scheduled_at = COALESCE(\x245, scheduled_at), directory_id = COALESCE(\x246, directory_id) WHERE id = \x247 RETURNING id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at ")
    .bind(&body.name).bind(body.template_id).bind(&body.recipient_filter)
    .bind(&body.status).bind(body.scheduled_at).bind(body.directory_id)
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(updated)))
}

pub async fn delete_campaign(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM email_campaigns WHERE id = \x241")
    .bind(id)
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(String::from("Email campaign not found")));
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"deleted": true}))))
}

pub async fn send_campaign(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let campaign = sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at FROM email_campaigns WHERE id = \x241")
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email campaign not found")))?;

    if campaign.status.as_deref() == Some("sent") {
        return Err(AppError::BadRequest(String::from("Campaign has already been sent")));
    }

    if campaign.status.as_deref() == Some("sending") {
        return Err(AppError::BadRequest(String::from("Campaign is currently being sent")));
    }

    let updated = sqlx::query_as::<_, EmailCampaign>(
        "UPDATE email_campaigns SET status = 'sent', sent_count = COALESCE(sent_count, 0) + 1, sent_at = NOW() WHERE id = \x241 RETURNING id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at ")
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(updated)))
}