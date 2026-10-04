//! Direct messaging between visitors and businesses

use axum::{
    extract::{Path, State},
    http::HeaderMap,
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
    pub name: Option<String>,  // visitor name (guest mode)
    pub email: Option<String>, // visitor email (guest mode)
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
        "SELECT email FROM users WHERE id = $1::uuid AND is_active = true",
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
        "SELECT EXISTS(SELECT 1 FROM businesses WHERE id = $1 AND is_active = true)",
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
            "SELECT name, email FROM users WHERE id = $1::uuid AND is_active = true",
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
           RETURNING id"#,
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
           FROM business_messages WHERE id = $1"#,
    )
    .bind(msg_id)
    .fetch_one(db)
    .await?;

    // Fire-and-forget forward to CoreSwift CRM if configured
    let sender_name_clone = sender_name.clone().unwrap_or_default();
    let sender_email_clone = sender_email.clone();
    let subject_clone = body.subject.clone();
    let message_clone = body.message.clone();
    tokio::spawn(async move {
        let _ = forward_to_coreswift(
            &sender_name_clone,
            sender_email_clone.as_deref(),
            subject_clone.as_deref(),
            &message_clone,
            "md_business",
            &business_id.to_string(),
        )
        .await;
    });

    Ok(Json(msg))
}

/// Forward a message to CoreSwift CRM webhook (fire-and-forget).
/// Does NOT block or error on failure — CoreSwift being down is non-fatal.
async fn forward_to_coreswift(
    sender_name: &str,
    sender_email: Option<&str>,
    subject: Option<&str>,
    body: &str,
    source: &str,
    source_id: &str,
) {
    let payload = serde_json::json!({
        "sender_name": sender_name,
        "sender_email": sender_email,
        "sender_phone": null,
        "subject": subject,
        "body": body,
        "source": source,
        "source_id": source_id,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let urls = ["http://localhost:8084/api/messages/webhook"];

    for url in &urls {
        let _ = client.post(*url).json(&payload).send().await;
    }
}

/// GET /api/v1/messages/:business_id — list messages for a business (owner only)
pub async fn list_messages(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(business_id): Path<Uuid>,
) -> Result<Json<Vec<MessageResponse>>, AppError> {
    let db = &s.db;

    // Customer messages are private to the business. The claims are verified from the header
    // rather than taken from Extension, so the check holds no matter which router group this
    // route lands in. (The old check let ANY tenant admin through via `role == "admin"`.)
    let claims =
        crate::handlers::tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    crate::handlers::tenant_scope::assert_business_admin_or_claimant(db, &claims, business_id)
        .await?;

    let messages = sqlx::query_as::<_, MessageResponse>(
        r#"SELECT id, business_id, sender_name, sender_email, subject, message, is_read, created_at
           FROM business_messages
           WHERE business_id = $1
           ORDER BY created_at DESC"#,
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

    let biz_id: Option<Uuid> =
        sqlx::query_scalar("SELECT business_id FROM business_messages WHERE id = $1")
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
    headers: HeaderMap,
    Path(business_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = &s.db;

    // The unread count is derived from the same private message set — same guard.
    let claims =
        crate::handlers::tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    crate::handlers::tenant_scope::assert_business_admin_or_claimant(db, &claims, business_id)
        .await?;

    let count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM business_messages WHERE business_id = $1 AND is_read = false",
    )
    .bind(business_id)
    .fetch_one(db)
    .await?;

    Ok(Json(serde_json::json!({"unread": count})))
}

#[derive(Debug, Deserialize)]
pub struct BroadcastQuoteRequest {
    pub business_id: Uuid,
    pub name: Option<String>,
    pub email: Option<String>,
    pub subject: Option<String>,
    pub message: String,
    #[serde(default)]
    pub max: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BroadcastTarget {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct BroadcastQuoteResponse {
    pub sent: usize,
    pub businesses: Vec<BroadcastTarget>,
}

/// POST /api/v1/quotes/broadcast — Thumbtack-style "get multiple quotes for one job".
///
/// A visitor's single quote request is routed to the OTHER pros **matched by category and area**
/// for the anchor listing: same directory (market) and same category, preferring the same city,
/// ranked by rating/review volume. Built ON the existing messaging threads (no parallel inbox, no
/// new table) — every matched business receives the request as a normal `business_messages` row,
/// so it lands in that owner's portal Customer Messages inbox exactly like a direct enquiry.
///
/// The anchor listing is NOT re-messaged here; the caller sends the direct request first through
/// `POST /messages/:business_id`, then fans out the same text to the matched peers.
pub async fn broadcast_quote(
    State(s): State<AppState>,
    claims: Option<Extension<Claims>>,
    Json(body): Json<BroadcastQuoteRequest>,
) -> Result<Json<BroadcastQuoteResponse>, AppError> {
    let db = &s.db;

    if body.message.trim().is_empty() {
        return Err(AppError::BadRequest("message is required".into()));
    }

    // Anchor listing: which market (directory) and category are we matching on?
    let anchor: Option<(Uuid, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "SELECT directory_id, category_id, city FROM businesses WHERE id = $1 AND is_active = true",
    )
    .bind(body.business_id)
    .fetch_optional(db)
    .await?;

    let (directory_id, category_id, city) = match anchor {
        Some(a) => a,
        None => return Err(AppError::NotFound("Business not found".into())),
    };

    // Bounded fan-out — a lead never floods an entire market.
    let limit = body.max.unwrap_or(4).clamp(1, 6);

    // No category → no meaningful "matched by category and area" set; honest zero.
    let category_id = match category_id {
        Some(c) => c,
        None => {
            return Ok(Json(BroadcastQuoteResponse {
                sent: 0,
                businesses: vec![],
            }))
        }
    };

    let targets = sqlx::query_as::<_, BroadcastTarget>(
        r#"SELECT id, name FROM businesses
           WHERE is_active = true
             AND id <> $1
             AND directory_id = $2
             AND category_id = $3
           ORDER BY (city IS NOT DISTINCT FROM $4) DESC,
                    rating DESC NULLS LAST,
                    review_count DESC NULLS LAST,
                    name ASC
           LIMIT $5"#,
    )
    .bind(body.business_id)
    .bind(directory_id)
    .bind(category_id)
    .bind(city.clone())
    .bind(limit)
    .fetch_all(db)
    .await?;

    // Sender identity: the visitor token wins; guests may still send using the form fields.
    let (sender_name, sender_email) = if let Some(Extension(c)) = claims {
        let user_info = sqlx::query_as::<_, (String, String)>(
            "SELECT name, email FROM users WHERE id = $1::uuid AND is_active = true",
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

    let mut sent = 0usize;
    for t in &targets {
        let res = sqlx::query(
            r#"INSERT INTO business_messages (business_id, sender_name, sender_email, subject, message)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(t.id)
        .bind(&sender_name)
        .bind(&sender_email)
        .bind(&body.subject)
        .bind(&body.message)
        .execute(db)
        .await;
        match res {
            Ok(_) => sent += 1,
            // One bad target must not sink the whole fan-out.
            Err(e) => eprintln!("broadcast_quote: insert for {} failed: {}", t.id, e),
        }
    }

    Ok(Json(BroadcastQuoteResponse {
        sent,
        businesses: targets,
    }))
}
