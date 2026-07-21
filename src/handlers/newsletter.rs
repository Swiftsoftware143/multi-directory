//! Newsletter queue, subscriber management, SMTP settings, and email sending.
//!
//! All features are per-directory. Each directory owner configures their own SMTP
//! and manages their own subscribers. Admin level is only for David's personal use.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::{IntoResponse, Html, Json},
    Json as JsonBody,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use lettre::{
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, Tokio1Executor,
    Message, AsyncTransport,
};

use crate::AppState;
use crate::error::{ApiResult, AppError};

// ── Newsletter Queue ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Newsletter {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub title: String,
    pub intro_text: Option<String>,
    pub include_blog: Option<bool>,
    pub include_deals: Option<bool>,
    pub manual_sections: Option<serde_json::Value>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub status: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNewsletterRequest {
    pub directory_id: Uuid,
    pub title: String,
    pub intro_text: Option<String>,
    pub include_blog: Option<bool>,
    pub include_deals: Option<bool>,
    pub manual_sections: Option<serde_json::Value>,
    pub scheduled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNewsletterRequest {
    pub title: Option<String>,
    pub intro_text: Option<String>,
    pub include_blog: Option<bool>,
    pub include_deals: Option<bool>,
    pub manual_sections: Option<serde_json::Value>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub status: Option<String>,
}

// ── SMTP Settings ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DirectoryEmailSettings {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub smtp_host: String,
    pub smtp_port: i32,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_encryption: String,
    pub from_name: String,
    pub from_email: String,
    pub reply_to: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertEmailSettingsRequest {
    pub smtp_host: String,
    pub smtp_port: Option<i32>,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_encryption: Option<String>,
    pub from_name: String,
    pub from_email: String,
    pub reply_to: Option<String>,
}

// ── Subscribers ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct NewsletterSubscriber {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub status: Option<String>,
    pub subscribed_at: Option<DateTime<Utc>>,
    pub unsubscribed_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct AddSubscriberRequest {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportSubscribersRequest {
    pub subscribers: Vec<AddSubscriberRequest>,
}

#[derive(Debug, Serialize)]
pub struct NewsletterRender {
    pub title: String,
    pub html: String,
    pub text: String,
    pub json_sections: Vec<serde_json::Value>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateParams {
    pub format: Option<String>,
}

// ── Newsletter Queue CRUD ──

pub async fn list_newsletters(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let dir_id = params.get("directory_id")
        .ok_or_else(|| AppError::BadRequest("directory_id is required".into()))?;
    let uuid: Uuid = dir_id.parse().map_err(|_| AppError::BadRequest("invalid uuid".into()))?;

    let newsletters = sqlx::query_as::<_, Newsletter>(
        "SELECT * FROM newsletter_queue WHERE directory_id = $1 ORDER BY created_at DESC"
    )
    .bind(uuid)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(newsletters))
}

pub async fn get_newsletter(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let n = sqlx::query_as::<_, Newsletter>(
        "SELECT * FROM newsletter_queue WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("newsletter not found".into()))?;

    Ok(Json(n))
}

pub async fn create_newsletter(
    State(s): State<AppState>,
    JsonBody(req): JsonBody<CreateNewsletterRequest>,
) -> ApiResult<impl IntoResponse> {
    let n = sqlx::query_as::<_, Newsletter>(
        r#"INSERT INTO newsletter_queue (directory_id, title, intro_text, include_blog, include_deals, manual_sections, scheduled_at)
           VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7)
           RETURNING *"#
    )
    .bind(req.directory_id)
    .bind(&req.title)
    .bind(req.intro_text)
    .bind(req.include_blog.unwrap_or(true))
    .bind(req.include_deals.unwrap_or(true))
    .bind(req.manual_sections.unwrap_or(serde_json::json!([])))
    .bind(req.scheduled_at)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(n)))
}

pub async fn update_newsletter(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    JsonBody(req): JsonBody<UpdateNewsletterRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Newsletter>(
        "SELECT * FROM newsletter_queue WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("newsletter not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let intro_text = req.intro_text.or(existing.intro_text);
    let include_blog = req.include_blog.or(existing.include_blog);
    let include_deals = req.include_deals.or(existing.include_deals);
    let manual_sections = req.manual_sections.or(existing.manual_sections);
    let scheduled_at = req.scheduled_at.or(existing.scheduled_at);
    let status = req.status.or(existing.status);

    let n = sqlx::query_as::<_, Newsletter>(
        r#"UPDATE newsletter_queue
           SET title = $1, intro_text = $2, include_blog = $3, include_deals = $4,
               manual_sections = $5::jsonb, scheduled_at = $6, status = $7, updated_at = NOW()
           WHERE id = $8
           RETURNING *"#
    )
    .bind(&title)
    .bind(intro_text)
    .bind(include_blog)
    .bind(include_deals)
    .bind(manual_sections)
    .bind(scheduled_at)
    .bind(status)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(n))
}

pub async fn delete_newsletter(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let r = sqlx::query("DELETE FROM newsletter_queue WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("newsletter not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── SMTP Settings ──

pub async fn get_email_settings(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    let settings = sqlx::query_as::<_, DirectoryEmailSettings>(
        "SELECT * FROM directory_email_settings WHERE directory_id = $1"
    )
    .bind(dir.0)
    .fetch_optional(&s.db)
    .await?;

    match settings {
        Some(s) => {
            let mut resp = serde_json::to_value(&s).unwrap();
            if let Some(obj) = resp.as_object_mut() {
                obj.insert("smtp_password".into(), serde_json::json!("********"));
            }
            Ok(Json(resp).into_response())
        }
        None => Ok((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "no email settings configured"}))).into_response()),
    }
}

pub async fn upsert_email_settings(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    JsonBody(req): JsonBody<UpsertEmailSettingsRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    let settings = sqlx::query_as::<_, DirectoryEmailSettings>(
        r#"INSERT INTO directory_email_settings (directory_id, smtp_host, smtp_port, smtp_username, smtp_password, smtp_encryption, from_name, from_email, reply_to)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (directory_id)
           DO UPDATE SET smtp_host = $2, smtp_port = $3, smtp_username = $4, smtp_password = $5,
                         smtp_encryption = $6, from_name = $7, from_email = $8, reply_to = $9, updated_at = NOW()
           RETURNING *"#
    )
    .bind(dir.0)
    .bind(&req.smtp_host)
    .bind(req.smtp_port.unwrap_or(587))
    .bind(&req.smtp_username)
    .bind(&req.smtp_password)
    .bind(req.smtp_encryption.unwrap_or_else(|| "tls".to_string()))
    .bind(&req.from_name)
    .bind(&req.from_email)
    .bind(req.reply_to)
    .fetch_one(&s.db)
    .await?;

    let mut resp = serde_json::to_value(&settings).unwrap();
    if let Some(obj) = resp.as_object_mut() {
        obj.insert("smtp_password".into(), serde_json::json!("********"));
    }
    Ok(Json(resp))
}

pub async fn delete_email_settings(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    sqlx::query("DELETE FROM directory_email_settings WHERE directory_id = $1")
        .bind(dir.0)
        .execute(&s.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Subscriber Management ──

pub async fn list_subscribers(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    let status_filter = params.get("status").map(|s| s.as_str()).unwrap_or("active");
    let page: i64 = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let per_page: i64 = params.get("per_page").and_then(|p| p.parse().ok()).unwrap_or(50);
    let offset = (page - 1) * per_page;

    let subscribers = sqlx::query_as::<_, NewsletterSubscriber>(
        "SELECT * FROM newsletter_subscribers WHERE directory_id = $1 AND status = $2 ORDER BY subscribed_at DESC LIMIT $3 OFFSET $4"
    )
    .bind(dir.0)
    .bind(status_filter)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM newsletter_subscribers WHERE directory_id = $1 AND status = $2"
    )
    .bind(dir.0)
    .bind(status_filter)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(serde_json::json!({
        "subscribers": subscribers,
        "total": total.0,
        "page": page,
        "per_page": per_page,
    })))
}

pub async fn add_subscriber(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    JsonBody(req): JsonBody<AddSubscriberRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    let sub = sqlx::query_as::<_, NewsletterSubscriber>(
        r#"INSERT INTO newsletter_subscribers (directory_id, email, name)
           VALUES ($1, $2, $3)
           ON CONFLICT (directory_id, email)
           DO UPDATE SET status = 'active', name = COALESCE($3, newsletter_subscribers.name), unsubscribed_at = NULL, subscribed_at = NOW()
           RETURNING *"#
    )
    .bind(dir.0)
    .bind(&req.email)
    .bind(&req.name)
    .fetch_one(&s.db)
    .await?;

    // Push to CoreSwift CRM with city tag (fire-and-forget, log on failure)
    let db = s.db.clone();
    let dir_id = dir.0;
    let email = req.email.clone();
    let name = req.name.clone();
    tokio::spawn(async move {
        match crate::coreswift::push_newsletter_signup(&db, dir_id, &email, name.as_deref()).await {
            Ok(contact_id) => tracing::info!("[newsletter] CoreSwift push OK for {email}, contact={contact_id}"),
            Err(e) => tracing::warn!("[newsletter] CoreSwift push failed for {email}: {e}"),
        }
    });

    // Fire cross-platform tag sync (fire-and-forget) — Subscriber tag for IncentiveSwift
    let ts_db = s.db.clone();
    let ts_email = sub.email.clone();
    let ts_name = sub.name.clone();
    let ts_dir_id = dir.0;
    let ts_dir_slug = slug.clone();
    tokio::spawn(async move {
        // Map directory slug to ZaarHub newsletter tag (e.g., palm-coast → pc-zh-newsletter)
        let newsletter_tag = match ts_dir_slug.as_str() {
            "apopka" => "ap-zh-newsletter",
            "boca-raton" => "br-zh-newsletter",
            "hollywood" => "hw-zh-newsletter",
            "lake-nona" => "ln-zh-newsletter",
            "palm-bay" => "pb-zh-newsletter",
            "palm-coast" => "pc-zh-newsletter",
            "pompano-beach" => "pp-zh-newsletter",
            "st-cloud" => "sc-zh-newsletter",
            "st-petersburg" => "sp-zh-newsletter",
            "winter-garden" => "wg-zh-newsletter",
            _ => &ts_dir_slug,
        }.to_string();

        let tags = vec!["Subscriber".to_string(), newsletter_tag];

        // Resolve tenant_id from directory so CoreSwift tag-sync doesn't 422
        let resolved_tenant = crate::coreswift::resolve_config(&ts_db, ts_dir_id).await.ok()
            .map(|(tid, _, _, _)| tid.to_string());

        crate::handlers::tag_sync::fire_tag_sync(
            &ts_db,
            ts_email,
            ts_name,
            None,
            None, // phone
            tags,
            None, // city_list
            Some("subscribers".to_string()),
            Some(ts_dir_slug),
            Some("newsletter_signup".to_string()),
            resolved_tenant, // tenant_id (resolved)
            None, // coreswift_list_id
        );
    });

    Ok((StatusCode::CREATED, Json(sub)))
}

pub async fn import_subscribers(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    JsonBody(req): JsonBody<ImportSubscribersRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    let mut added = 0u64;

    for sub in &req.subscribers {
        let r = sqlx::query(
            r#"INSERT INTO newsletter_subscribers (directory_id, email, name)
               VALUES ($1, $2, $3)
               ON CONFLICT (directory_id, email)
               DO UPDATE SET status = 'active', name = COALESCE($3, newsletter_subscribers.name), unsubscribed_at = NULL, subscribed_at = NOW()
               WHERE newsletter_subscribers.status = 'unsubscribed'"#
        )
        .bind(dir.0)
        .bind(&sub.email)
        .bind(&sub.name)
        .execute(&s.db)
        .await.map_err(|e| AppError::Internal(e.to_string()))?;

        if r.rows_affected() > 0 {
            added += 1;
        }
    }

    Ok(Json(serde_json::json!({"added": added, "total": req.subscribers.len()})))
}

pub async fn unsubscribe_subscriber(
    State(s): State<AppState>,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))?;

    let r = sqlx::query(
        "UPDATE newsletter_subscribers SET status = 'unsubscribed', unsubscribed_at = NOW() WHERE id = $1 AND directory_id = $2 AND status = 'active'"
    )
    .bind(id)
    .bind(dir.0)
    .execute(&s.db)
    .await?;

    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("subscriber not found or already unsubscribed".into()));
    }

    Ok(Json(serde_json::json!({"status": "unsubscribed"})))
}

// ── Content Generation & HTML/Text Rendering ──

pub async fn generate_newsletter_content(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<GenerateParams>,
) -> ApiResult<impl IntoResponse> {
    let n = sqlx::query_as::<_, Newsletter>(
        "SELECT * FROM newsletter_queue WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("newsletter not found".into()))?;

    let (render, _) = build_newsletter_render(&s, &n).await?;

    match params.format.as_deref().unwrap_or("json") {
        "html" => Ok(Html(render.html).into_response()),
        "text" => Ok(render.text.into_response()),
        "both" => Ok(Json(render).into_response()),
        _ => Ok(Json(serde_json::json!({
            "title": render.title,
            "sections": render.json_sections,
            "generated_at": render.generated_at
        })).into_response()),
    }
}

// ── Send Newsletter via SMTP ──

pub async fn send_newsletter(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let n = sqlx::query_as::<_, Newsletter>(
        "SELECT * FROM newsletter_queue WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("newsletter not found".into()))?;

    if n.status.as_deref() == Some("sent") {
        return Err(AppError::BadRequest("newsletter already sent".into()));
    }

    let smtp = sqlx::query_as::<_, DirectoryEmailSettings>(
        "SELECT * FROM directory_email_settings WHERE directory_id = $1"
    )
    .bind(n.directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("no SMTP settings configured for this directory".into()))?;

    let subscribers = sqlx::query_as::<_, NewsletterSubscriber>(
        "SELECT * FROM newsletter_subscribers WHERE directory_id = $1 AND status = 'active'"
    )
    .bind(n.directory_id)
    .fetch_all(&s.db)
    .await?;

    if subscribers.is_empty() {
        return Err(AppError::BadRequest("no active subscribers".into()));
    }

    let (render, dir_name) = build_newsletter_render(&s, &n).await?;

    // SMTP connection
    let from_name_final = if smtp.from_name.is_empty() { dir_name.clone() } else { smtp.from_name.clone() };
    let from_email_final = if smtp.from_email.is_empty() { format!("noreply@{}", smtp.smtp_host) } else { smtp.from_email.clone() };
    let from_addr = format!("{} <{}>", from_name_final, from_email_final);

    let creds = Credentials::new(smtp.smtp_username.clone(), smtp.smtp_password.clone());

    let mailer = match smtp.smtp_encryption.as_str() {
        "ssl" => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.smtp_host)
            .map_err(|e| AppError::Internal(format!("SMTP relay: {}", e)))?
            .port(smtp.smtp_port as u16)
            .credentials(creds)
            .build(),
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.smtp_host)
            .map_err(|e| AppError::Internal(format!("SMTP relay: {}", e)))?
            .port(smtp.smtp_port as u16)
            .credentials(creds)
            .build(),
        _ => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.smtp_host)
            .port(smtp.smtp_port as u16)
            .credentials(creds)
            .build(),
    };

    // Fetch dir slug for unsubscribe link
    let dir_slug: Option<String> = sqlx::query_as::<_, (String,)>("SELECT slug FROM directories WHERE id = $1")
        .bind(n.directory_id).fetch_optional(&s.db).await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map(|r| r.0);

    let base_url = format!("https://{}.{}", dir_slug.as_deref().unwrap_or("directory"), s.config.base_domain);

    let mut sent = 0u64;
    let mut failed = 0u64;

    for sub in &subscribers {
        let name = sub.name.as_deref().unwrap_or("Subscriber");
        let personal_html = render.html.replace("{{SUBSCRIBER_NAME}}", name);
        let personal_text = render.text.replace("{{SUBSCRIBER_NAME}}", name);
        let unsub_url = format!("{}/unsubscribe?id={}", base_url, sub.id);
        let final_html = personal_html.replace("{{UNSUBSCRIBE_URL}}", &unsub_url);
        let final_text = personal_text.replace("{{UNSUBSCRIBE_URL}}", &unsub_url);

        let email = match Message::builder()
            .from(from_addr.parse().map_err(|_| AppError::Internal("invalid from address".into()))?)
            .to(sub.email.parse().map_err(|_| AppError::Internal("invalid to address".into()))?)
            .subject(&n.title)
            .multipart(
                lettre::message::MultiPart::alternative_plain_html(
                    final_text,
                    final_html,
                )
            ) {
            Ok(e) => e,
            Err(_) => { failed += 1; continue; }
        };

        match mailer.send(email).await {
            Ok(_) => sent += 1,
            Err(_) => failed += 1,
        }
    }

    sqlx::query("UPDATE newsletter_queue SET status = 'sent', sent_at = NOW(), updated_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    Ok(Json(serde_json::json!({
        "status": "sent",
        "sent_count": sent,
        "failed_count": failed,
        "total": subscribers.len()
    })))
}

// ── Internal Render Helper ──

async fn build_newsletter_render(
    s: &AppState,
    n: &Newsletter,
) -> Result<(NewsletterRender, String), AppError> {
    let dir_info: Option<(String, String)> = sqlx::query_as(
        "SELECT name, slug FROM directories WHERE id = $1"
    )
    .bind(n.directory_id)
    .fetch_optional(&s.db)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .map(|r: (String, String)| r);

    let dir_name = dir_info.as_ref().map(|d| d.0.as_str()).unwrap_or("Your Directory").to_string();
    let dir_slug = dir_info.as_ref().map(|d| d.1.as_str()).unwrap_or("directory");
    let dir_url = format!("https://{}.{}", dir_slug, s.config.base_domain);

    let mut sections: Vec<serde_json::Value> = Vec::new();
    let mut html_parts: Vec<String> = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    // Header
    html_parts.push(format!(
        r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#0f172a;padding:24px 0">
           <tr><td align="center" style="font-family:Georgia,serif;font-size:28px;font-weight:700;color:#14b8a6">{}</td></tr>
           </table>"#,
        html_escape(&n.title)
    ));
    text_parts.push(format!("=== {} ===\n", &n.title));

    // Intro
    if let Some(ref intro) = n.intro_text {
        sections.push(serde_json::json!({"type":"intro","content":intro}));
        html_parts.push(format!(
            r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="padding:20px 24px;font-family:Helvetica,Arial,sans-serif;font-size:16px;line-height:1.6;color:#334155"><tr><td>{}</td></tr></table>"#,
            html_escape(intro)
        ));
        text_parts.push(intro.to_string());
    }

    // Directory bar
    html_parts.push(format!(
        r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f8fafc;padding:12px 24px;font-family:Helvetica,Arial,sans-serif;font-size:13px;color:#64748b"><tr><td>📍 {} | <a href="{}" style="color:#0d9488">{}</a></td></tr></table>"#,
        html_escape(&dir_name), html_escape(&dir_url), html_escape(&dir_url)
    ));
    text_parts.push(format!("📍 {} | {}", dir_name, dir_url));

    // Blog posts
    let blog_included = n.include_blog.unwrap_or(true);
    if blog_included {
        let posts: Vec<(Uuid, String, String, Option<String>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT id, title, slug, excerpt, created_at FROM blog_posts WHERE directory_id = $1 AND published = true ORDER BY created_at DESC LIMIT 5"
        )
        .bind(n.directory_id)
        .fetch_all(&s.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        if !posts.is_empty() {
            let mut blog_items: Vec<serde_json::Value> = Vec::new();
            let mut blog_html = String::from(
                r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="padding:16px 24px"><tr><td>
                   <h2 style="font-family:Helvetica,Arial,sans-serif;font-size:20px;color:#0f172a;margin:0 0 12px 0">📝 Latest Blog Posts</h2>"#
            );
            text_parts.push("\n📝 LATEST BLOG POSTS\n".to_string());

            for (id, title, slug, excerpt, created) in &posts {
                let post_url = format!("{}/blog/{}", dir_url, slug);
                let date_str = created.map(|d| d.format("%B %d, %Y").to_string()).unwrap_or_default();
                let excerpt_text = excerpt.as_deref().unwrap_or("");

                blog_items.push(serde_json::json!({"id":id,"title":title,"slug":slug,"excerpt":excerpt,"date":created}));
                blog_html.push_str(&format!(
                    r#"<div style="padding:12px 0;border-bottom:1px solid #e2e8f0">
                       <a href="{}" style="font-family:Helvetica,Arial,sans-serif;font-size:16px;font-weight:600;color:#0d9488;text-decoration:none">{}</a>
                       <div style="font-size:12px;color:#94a3b8;margin:2px 0 4px">{}</div>
                       <div style="font-family:Helvetica,Arial,sans-serif;font-size:14px;color:#475569;line-height:1.5">{}</div></div>"#,
                    html_escape(&post_url), html_escape(title), date_str, html_escape(excerpt_text)
                ));
                text_parts.push(format!("\n  📰 {} ({})", title, date_str));
                if !excerpt_text.is_empty() {
                    text_parts.push(format!("     {}", excerpt_text));
                }
                text_parts.push(format!("     Read more: {}", post_url));
            }
            blog_html.push_str("</td></tr></table>");
            html_parts.push(blog_html);
            sections.push(serde_json::json!({"type":"blog_posts","items":blog_items}));
        }
    }

    // Deals
    let deals_included = n.include_deals.unwrap_or(true);
    if deals_included {
        let deals: Vec<(Uuid, String, Option<String>, Option<String>, Option<i32>, String, bool)> = sqlx::query_as(
            "SELECT id, title, description, deal_price, discount_percent, status, featured FROM deals WHERE directory_id = $1 AND status = 'active' ORDER BY featured DESC, created_at DESC LIMIT 5"
        )
        .bind(n.directory_id)
        .fetch_all(&s.db)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        if !deals.is_empty() {
            let mut deal_items: Vec<serde_json::Value> = Vec::new();
            let mut deal_html = String::from(
                r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="padding:16px 24px;background:#f0fdfa"><tr><td>
                   <h2 style="font-family:Helvetica,Arial,sans-serif;font-size:20px;color:#0f172a;margin:0 0 12px 0">🔥 Featured Deals</h2>"#
            );
            text_parts.push("\n🔥 FEATURED DEALS\n".to_string());

            for (id, title, desc, price, discount, _status, featured) in &deals {
                let badge = if *featured { r#"<span style="background:#f59e0b;color:#fff;font-size:11px;font-weight:700;padding:2px 8px;border-radius:4px;margin-left:8px">FEATURED</span>"# } else { "" };
                let discount_str = discount.map(|d| format!("{}% OFF", d)).unwrap_or_default();

                deal_items.push(serde_json::json!({"id":id,"title":title,"description":desc,"deal_price":price,"discount_percent":discount,"featured":featured}));
                deal_html.push_str(&format!(
                    r#"<div style="padding:12px 0;border-bottom:1px solid #ccfbf1">
                       <div style="font-family:Helvetica,Arial,sans-serif;font-size:16px;font-weight:600;color:#0f172a">{}{}</div>"#,
                    html_escape(title), badge
                ));
                if let Some(p) = price {
                    deal_html.push_str(&format!(
                        r#"<div style="font-family:Helvetica,Arial,sans-serif;font-size:18px;font-weight:700;color:#0d9488">${}<span style="font-size:13px;font-weight:400;color:#64748b;margin-left:8px">{}</span></div>"#,
                        html_escape(p), discount_str
                    ));
                }
                if let Some(d) = desc {
                    deal_html.push_str(&format!(r#"<div style="font-family:Helvetica,Arial,sans-serif;font-size:14px;color:#475569;line-height:1.5">{}</div>"#, html_escape(d)));
                }
                deal_html.push_str("</div>");
                text_parts.push(format!("\n  💰 {}", title));
                if let Some(p) = price { text_parts.push(format!("     Price: ${}", p)); }
                if !discount_str.is_empty() { text_parts.push(format!("     Discount: {}", discount_str)); }
                if let Some(d) = desc { text_parts.push(format!("     {}", d)); }
                if *featured { text_parts.push("     ⭐ FEATURED".to_string()); }
            }
            deal_html.push_str("</td></tr></table>");
            html_parts.push(deal_html);
            sections.push(serde_json::json!({"type":"deals","items":deal_items}));
        }
    }

    // Manual sections (paid ad spots)
    if let Some(ref ms) = n.manual_sections {
        if let Some(arr) = ms.as_array() {
            for section in arr {
                sections.push(serde_json::json!({"type":"manual","content":section}));
                let heading = section.get("heading").and_then(|h| h.as_str()).unwrap_or("Sponsored");
                let body = section.get("body").and_then(|b| b.as_str()).unwrap_or("");
                let link = section.get("link").and_then(|l| l.as_str());

                let mut man_html = format!(
                    r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="padding:16px 24px;background:#fffbeb"><tr><td>
                       <h3 style="font-family:Helvetica,Arial,sans-serif;font-size:15px;color:#92400e;margin:0 0 6px 0">📢 {}</h3>
                       <div style="font-family:Helvetica,Arial,sans-serif;font-size:14px;color:#78350f;line-height:1.5">{}</div>"#,
                    html_escape(heading), html_escape(body)
                );
                if let Some(l) = link {
                    man_html.push_str(&format!(
                        r#"<div style="margin-top:8px"><a href="{}" style="display:inline-block;background:#f59e0b;color:#fff;font-weight:600;padding:8px 16px;border-radius:6px;text-decoration:none;font-size:13px">Learn More →</a></div>"#,
                        html_escape(l)
                    ));
                }
                man_html.push_str("</td></tr></table>");
                html_parts.push(man_html);
                text_parts.push(format!("\n📢 {}: {}", heading, body));
                if let Some(l) = link { text_parts.push(format!("   Learn more: {}", l)); }
            }
        }
    }

    // Footer with subscriber placeholders
    sections.push(serde_json::json!({"type":"footer","content":"You are receiving this because you subscribed."}));
    html_parts.push(format!(
        r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f1f5f9;padding:20px 24px"><tr><td align="center" style="font-family:Helvetica,Arial,sans-serif;font-size:12px;color:#94a3b8;line-height:1.6">
           <p style="margin:0">Hi {{SUBSCRIBER_NAME}}, you're receiving this because you subscribed to {}.<br>
           <a href="{{{{UNSUBSCRIBE_URL}}}}" style="color:#0d9488">Unsubscribe</a> at any time.<br>
           © 2026 <a href="{}" style="color:#0d9488">{}</a>. All rights reserved.</p></td></tr></table>"#,
        html_escape(&dir_name), html_escape(&dir_url), html_escape(&dir_name)
    ));
    text_parts.push(format!("\n---\nHi {{SUBSCRIBER_NAME}},\nYou're receiving this because you subscribed to {}.\nUnsubscribe: {{{{UNSUBSCRIBE_URL}}}}\n© 2026 {} — All rights reserved.", dir_name, dir_url));

    let html_body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<style type="text/css">
body{{margin:0;padding:0;background-color:#e2e8f0}}
.container{{max-width:600px;margin:0 auto;background:#ffffff}}
a{{color:#0d9488}}
@media only screen and (max-width:480px){{.container{{width:100%!important}}}}
</style>
</head>
<body style="margin:0;padding:16px 0;background:#e2e8f0">
<div class="container" style="max-width:600px;margin:0 auto;background:#ffffff;border-radius:8px;overflow:hidden">
{}
</div>
</body>
</html>"#,
        html_parts.join("\n")
    );
    let text_body = text_parts.join("\n");

    Ok((NewsletterRender {
        title: n.title.clone(),
        html: html_body,
        text: text_body,
        json_sections: sections,
        generated_at: Utc::now(),
    }, dir_name))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&#39;")
}
