//! Email template and campaign CRUD — with HTML/text body toggle and directory signatures.
//!
//! Templates store both body (HTML) and body_text (plain-text).
//! When body_text is null, the SMTP service auto-generates from the HTML.
//! Directory-level signatures are appended to outgoing emails automatically.

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
    pub body_text: Option<String>,
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
    pub body: String,           // HTML
    pub body_text: Option<String>, // plain-text fallback
    pub variables: Option<Vec<String>>,
    pub category: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTemplateRequest {
    pub name: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub body_text: Option<String>,
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

/// Directory signature fields exposed in the directory GET/PUT endpoints
#[derive(Debug, Serialize, Deserialize)]
pub struct EmailSignature {
    pub email_signature_html: Option<String>,
    pub email_signature_text: Option<String>,
}

// ==================== Template Handlers ====================

const TEMPLATE_COLS: &str = "id, name, subject, body, body_text, variables, category, directory_id, created_at, updated_at";

pub async fn list_templates(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let templates = sqlx::query_as::<_, EmailTemplate>(
        &format!("SELECT {TEMPLATE_COLS} FROM email_templates ORDER BY created_at DESC")
    )
    .fetch_all(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(templates)))
}

pub async fn get_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, EmailTemplate>(
        &format!("SELECT {TEMPLATE_COLS} FROM email_templates WHERE id = $1")
    )
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
        &format!("INSERT INTO email_templates (name, subject, body, body_text, variables, category, directory_id) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {TEMPLATE_COLS}")
    )
    .bind(&body.name).bind(&body.subject).bind(&body.body).bind(&body.body_text)
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
        "SELECT id FROM email_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email template not found")))?;

    // Build dynamic UPDATE — only include fields that were actually sent
    // Use COALESCE so unset fields keep their current value
    let updated = sqlx::query_as::<_, EmailTemplate>(
        &format!(
            "UPDATE email_templates SET \
             name       = COALESCE($1, name), \
             subject    = COALESCE($2, subject), \
             body       = COALESCE($3, body), \
             body_text  = COALESCE($4, body_text), \
             variables  = COALESCE($5, variables), \
             category   = COALESCE($6, category), \
             directory_id = COALESCE($7, directory_id) \
             WHERE id = $8 RETURNING {TEMPLATE_COLS}"
        )
    )
    .bind(&body.name).bind(&body.subject).bind(&body.body).bind(&body.body_text)
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
    let result = sqlx::query("DELETE FROM email_templates WHERE id = $1")
    .bind(id)
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(String::from("Email template not found")));
    }
    Ok((StatusCode::OK, Json(serde_json::json!({"deleted": true}))))
}

// ==================== Campaign Handlers ====================

const CAMP_COLS: &str = "id, name, template_id, recipient_filter, sent_count, opened_count, status, scheduled_at, sent_at, directory_id, created_at";

pub async fn list_campaigns(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let campaigns = sqlx::query_as::<_, EmailCampaign>(
        &format!("SELECT {CAMP_COLS} FROM email_campaigns ORDER BY created_at DESC")
    )
    .fetch_all(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(campaigns)))
}

pub async fn get_campaign(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let campaign = sqlx::query_as::<_, EmailCampaign>(
        &format!("SELECT {CAMP_COLS} FROM email_campaigns WHERE id = $1")
    )
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
        &format!("INSERT INTO email_campaigns (name, template_id, recipient_filter, status, scheduled_at, directory_id) VALUES ($1, $2, $3, $4, $5, $6) RETURNING {CAMP_COLS}")
    )
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
        "SELECT id FROM email_campaigns WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(String::from("Email campaign not found")))?;

    let updated = sqlx::query_as::<_, EmailCampaign>(
        &format!(
            "UPDATE email_campaigns SET \
             name = COALESCE($1, name), \
             template_id = COALESCE($2, template_id), \
             recipient_filter = COALESCE($3, recipient_filter), \
             status = COALESCE($4, status), \
             scheduled_at = COALESCE($5, scheduled_at), \
             directory_id = COALESCE($6, directory_id) \
             WHERE id = $7 RETURNING {CAMP_COLS}"
        )
    )
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
    let result = sqlx::query("DELETE FROM email_campaigns WHERE id = $1")
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
        &format!("SELECT {CAMP_COLS} FROM email_campaigns WHERE id = $1")
    )
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
        &format!("UPDATE email_campaigns SET status = 'sent', sent_count = COALESCE(sent_count, 0) + 1, sent_at = NOW() WHERE id = $1 RETURNING {CAMP_COLS}")
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::OK, Json(updated)))
}

// ==================== Signature Helper ====================

/// Fetch the email signature for a directory
pub async fn get_directory_signature(
    state: &AppState,
    dir_id: Uuid,
) -> EmailSignature {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT email_signature_html, email_signature_text FROM directories WHERE id = $1"
    )
    .bind(dir_id)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some((html, text))) => EmailSignature { email_signature_html: html, email_signature_text: text },
        _ => EmailSignature { email_signature_html: None, email_signature_text: None },
    }
}

/// Append directory signature to both HTML and text bodies
pub fn append_signature(
    html_body: &str,
    text_body: Option<&str>,
    signature: &EmailSignature,
) -> (String, Option<String>) {
    let sig_html = signature.email_signature_html.as_deref().unwrap_or("");
    let sig_text = signature.email_signature_text.as_deref().unwrap_or("");

    let result_html = if sig_html.is_empty() {
        html_body.to_string()
    } else {
        // Insert before </body> or append at the end
        if let Some(pos) = html_body.rfind("</body>") {
            let mut s = html_body.to_string();
            s.insert_str(pos, sig_html);
            s
        } else {
            format!("{html_body}\n{sig_html}")
        }
    };

    let result_text: Option<String> = match (text_body, sig_text) {
        (Some(tb), st) if !st.is_empty() => Some(format!("{tb}\n\n{st}")),
        (Some(tb), _) => Some(tb.to_string()),
        (None, st) if !st.is_empty() => Some(format!("{st}")),
        _ => None, // smtp service will generate from HTML
    };

    (result_html, result_text)
}
