//! Direct messaging between visitors and businesses

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::AppError;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub name: Option<String>,   // visitor name (guest mode)
    pub email: Option<String>,  // visitor email (guest mode)
    pub subject: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MessageResponse {
    pub id: Uuid,
    pub business_id: Uuid,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub subject: Option<String>,
    pub message: String,
    pub is_read: bool,
    pub created_at: DateTime<Utc>,
}

/// Helper: fetch the email for a user by their UUID sub claim.
async fn get_user_email(db: &PgPool, user_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT email FROM users WHERE id = $1::uuid AND is_active = true"
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// Helper: verify that `sub` (user id) owns the business via claimed_businesses.
/// Returns true if the user's email matches a claim on the business.
async fn is_owner_of(db: &PgPool, user_id: &str, business_id: Uuid) -> Result<bool, AppError> {
    let user_email = get_user_email(db, user_id).await;
    match user_email {
        Some(email) => {
            let row: (bool,) = sqlx::query_as(
                "SELECT EXISTS(SELECT 1 FROM claimed_businesses WHERE business_id = $1 AND owner_email = $2 AND is_active = true)"
            )
            .bind(business_id)
            .bind(&email)
            .fetch_one(db)
            .await?;
            Ok(row.0)
        }
        None => Ok(false),
    }
}

/// POST /api/v1/messages/:business_id — send a message to a business
/// Works both for logged-in visitors and anonymous guests.
pub async fn send_message(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    claims: Option<Extension<Claims>>,
    Json(body): Json<SendMessageRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    let db = &s.db;

    // Validate business exists and is active
    let biz_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM businesses WHERE id = $1 AND is_active = true)"
    )
    .bind(business_id)
    .fetch_one(db)
    .await?;
    let biz_exists = biz_exists.0;

    if !biz_exists {
        return Err(AppError::NotFound("Business not found".into()));
    }

    // Get sender info from auth if available, else use form fields
    let (sender_name, sender_email) = if let Some(Extension(c)) = claims {
        let user_info = sqlx::query_as::<_, (String, String)>(
            "SELECT name, email FROM users WHERE id = $1::uuid AND is_active = true"
        )
        .bind(&c.sub)
        .fetch_optional(db)
        .await?;

        match user_info {
            Some((name, email)) => (Some(name), Some(email)),
            None => (body.name.clone(), body.email.clone()),
        }
    } else {
        (body.name.clone(), body.email.clone())
    };

    let msg_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO business_messages (business_id, sender_name, sender_email, subject, message)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#
    )
    .bind(business_id)
    .bind(&sender_name)
    .bind(&sender_email)
    .bind(&body.subject)
    .bind(&body.message)
    .fetch_one(db)
    .await?;

    let msg = sqlx::query_as::<_, MessageResponse>(
        r#"SELECT id, business_id, sender_name, sender_email, subject, message, is_read, created_at
           FROM business_messages WHERE id = $1"#
    )
    .bind(msg_id)
    .fetch_one(db)
    .await?;

    Ok(Json(msg))
}

/// GET /api/v1/messages/:business_id — list messages for a business (owner only)
pub async fn list_messages(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Vec<MessageResponse>>, AppError> {
    let db = &s.db;

    if !is_owner_of(db, &claims.sub, business_id).await? && claims.role != "admin" {
        return Err(AppError::Forbidden("Not authorized to view messages".into()));
    }

    let messages = sqlx::query_as::<_, MessageResponse>(
        r#"SELECT id, business_id, sender_name, sender_email, subject, message, is_read, created_at
           FROM business_messages
           WHERE business_id = $1
           ORDER BY created_at DESC"#
    )
    .bind(business_id)
    .fetch_all(db)
    .await?;

    Ok(Json(messages))
}

/// PATCH /api/v1/messages/:id/read — mark message as read
pub async fn mark_read(
    State(s): State<AppState>,
    Path(msg_id): Path<Uuid>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = &s.db;

    let biz_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT business_id FROM business_messages WHERE id = $1"
    )
    .bind(msg_id)
    .fetch_optional(db)
    .await?;

    let biz_id = biz_id.ok_or_else(|| AppError::NotFound("Message not found".into()))?;

    if !is_owner_of(db, &claims.sub, biz_id).await? && claims.role != "admin" {
        return Err(AppError::Forbidden("Not authorized".into()));
    }

    sqlx::query("UPDATE business_messages SET is_read = true WHERE id = $1")
        .bind(msg_id)
        .execute(db)
        .await?;

    Ok(Json(serde_json::json!({"status": "ok"})))
}

/// GET /api/v1/messages/:business_id/unread — unread count for business owner
pub async fn unread_count(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = &s.db;

    if !is_owner_of(db, &claims.sub, business_id).await? && claims.role != "admin" {
        return Err(AppError::Forbidden("Not authorized".into()));
    }

    let count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM business_messages WHERE business_id = $1 AND is_read = false"
    )
    .bind(business_id)
    .fetch_one(db)
    .await?;

    Ok(Json(serde_json::json!({"unread": count})))
}
