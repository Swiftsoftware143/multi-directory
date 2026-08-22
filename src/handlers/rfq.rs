//! RFQ Marketplace handlers — BL24
//! Businesses post what they need. Suppliers browse and bid.
//! Creates a B2B lead exchange that Google's algorithm cannot replicate.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::middleware::verify_token;
use crate::error::{ApiResult, AppError};
use crate::AppState;

// ── Auth helpers ──

fn extract_user_id(headers: &HeaderMap, state: &AppState) -> ApiResult<Uuid> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;
    let claims =
        verify_token(token, &state.config.jwt_secret).map_err(|_| AppError::Unauthorized)?;
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

async fn resolve_business_id(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Uuid> {
    let biz_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT cb.business_id
           FROM claimed_businesses cb
           WHERE cb.visitor_account_id = $1
           ORDER BY cb.created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    if let Some(bid) = biz_id {
        return Ok(bid);
    }

    let biz_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT cb.business_id
           FROM claimed_businesses cb
           WHERE cb.user_id = $1
           ORDER BY cb.created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    if let Some(bid) = biz_id {
        return Ok(bid);
    }

    let email = sqlx::query_scalar::<_, String>("SELECT email FROM visitor_accounts WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await?;

    if let Some(ref em) = email {
        let biz_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM businesses WHERE email = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(em)
        .fetch_optional(db)
        .await?;

        if let Some(bid) = biz_id {
            return Ok(bid);
        }
    }

    Err(AppError::NotFound(
        "No business linked to your account. Claim a business first.".into(),
    ))
}

// ── Types ──

#[derive(Debug, FromRow, Serialize)]
struct RfqRow {
    id: Uuid,
    title: String,
    description: String,
    category: Option<String>,
    quantity: Option<String>,
    budget_min: Option<Decimal>,
    budget_max: Option<Decimal>,
    deadline: Option<NaiveDate>,
    delivery_location: Option<String>,
    poster_business_id: Uuid,
    status: String,
    urgency: Option<String>,
    is_public: Option<bool>,
    awarded_to: Option<Uuid>,
    awarded_bid_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    view_count: Option<i32>,
    bid_count: Option<i64>,
}

#[derive(Debug, FromRow, Serialize)]
struct RfqBidRow {
    id: Uuid,
    rfq_id: Uuid,
    bidder_business_id: Uuid,
    amount: Decimal,
    details: String,
    delivery_timeline: Option<String>,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow, Serialize)]
struct RfqMessageRow {
    id: Uuid,
    rfq_id: Uuid,
    sender_business_id: Uuid,
    message: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RfqQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    pub urgency: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRfqRequest {
    pub title: String,
    pub description: String,
    pub category: Option<String>,
    pub quantity: Option<String>,
    pub budget_min: Option<Decimal>,
    pub budget_max: Option<Decimal>,
    pub deadline: Option<String>,
    pub delivery_location: Option<String>,
    pub urgency: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRfqRequest {
    pub status: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BidRequest {
    pub amount: Decimal,
    pub details: String,
    pub delivery_timeline: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub message: String,
}

// ── API: GET /b2b/rfqs/stats ──

pub async fn rfq_stats(State(state): State<AppState>) -> ApiResult<impl IntoResponse> {
    let open_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rfqs WHERE status = 'open' AND is_public = true")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    let awarded_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rfqs WHERE status = 'awarded' AND is_public = true",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let categories: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT category FROM rfqs WHERE status = 'open' AND is_public = true AND category IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    Ok(Json(json!({
        "open_rfqs": open_count,
        "awarded_rfqs": awarded_count,
        "categories": categories,
    })))
}

// ── API: GET /b2b/rfqs ──

pub async fn list_rfqs(
    State(state): State<AppState>,
    Query(params): Query<RfqQuery>,
) -> ApiResult<impl IntoResponse> {
    let per_page = params.per_page.unwrap_or(20).min(100);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let rfqs = sqlx::query_as::<_, RfqRow>(
        "SELECT r.id, r.title, r.description, r.category, r.quantity, r.budget_min, r.budget_max, \
         r.deadline, r.delivery_location, r.poster_business_id, r.status, r.urgency, r.is_public, \
         r.awarded_to, r.awarded_bid_id, r.created_at, r.updated_at, \
         COALESCE(r.view_count, 0) as view_count, \
         COALESCE((SELECT COUNT(*) FROM rfq_bids WHERE rfq_id = r.id), 0) as bid_count \
         FROM rfqs r \
         WHERE r.status = 'open' AND r.is_public = true \
         ORDER BY r.created_at DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rfqs WHERE status = 'open' AND is_public = true")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

    Ok(Json(json!({
        "rfqs": rfqs,
        "total": total,
        "page": page,
        "per_page": per_page,
    })))
}

// ── API: GET /b2b/rfqs/my ──

pub async fn my_rfqs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let rfqs = sqlx::query_as::<_, RfqRow>(
        "SELECT r.id, r.title, r.description, r.category, r.quantity, r.budget_min, r.budget_max, \
         r.deadline, r.delivery_location, r.poster_business_id, r.status, r.urgency, r.is_public, \
         r.awarded_to, r.awarded_bid_id, r.created_at, r.updated_at, \
         COALESCE(r.view_count, 0) as view_count, \
         COALESCE((SELECT COUNT(*) FROM rfq_bids WHERE rfq_id = r.id), 0) as bid_count \
         FROM rfqs r WHERE r.poster_business_id = $1 ORDER BY r.created_at DESC",
    )
    .bind(biz_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({ "rfqs": rfqs })))
}

// ── API: GET /b2b/rfqs/:id ──

pub async fn get_rfq(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let _ = sqlx::query("UPDATE rfqs SET view_count = COALESCE(view_count, 0) + 1, updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;

    let rfq = sqlx::query_as::<_, RfqRow>(
        "SELECT r.id, r.title, r.description, r.category, r.quantity, r.budget_min, r.budget_max, \
         r.deadline, r.delivery_location, r.poster_business_id, r.status, r.urgency, r.is_public, \
         r.awarded_to, r.awarded_bid_id, r.created_at, r.updated_at, \
         COALESCE(r.view_count, 0) as view_count, \
         COALESCE((SELECT COUNT(*) FROM rfq_bids WHERE rfq_id = r.id), 0) as bid_count \
         FROM rfqs r WHERE r.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;

    match rfq {
        Some(r) => Ok(Json(json!({ "rfq": r }))),
        None => Err(AppError::NotFound("RFQ not found".into())),
    }
}

// ── API: POST /b2b/rfqs ──

pub async fn create_rfq(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRfqRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.title.trim().is_empty() {
        return Err(AppError::Validation("Title is required".into()));
    }
    if req.description.trim().is_empty() {
        return Err(AppError::Validation("Description is required".into()));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let deadline = req
        .deadline
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());

    let id = Uuid::new_v4();
    let urgency = req.urgency.unwrap_or_else(|| "standard".to_string());

    sqlx::query(
        "INSERT INTO rfqs (id, title, description, category, quantity, budget_min, budget_max, \
         deadline, delivery_location, poster_business_id, urgency) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.category)
    .bind(&req.quantity)
    .bind(req.budget_min)
    .bind(req.budget_max)
    .bind(deadline)
    .bind(&req.delivery_location)
    .bind(biz_id)
    .bind(&urgency)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "id": id,
        "status": "open",
        "poster_business_id": biz_id,
    })))
}

// ── API: PATCH /b2b/rfqs/:id ──

pub async fn update_rfq(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRfqRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let existing = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT poster_business_id, status FROM rfqs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;

    let (poster_id, current_status) = match existing {
        Some(r) => r,
        None => return Err(AppError::NotFound("RFQ not found".into())),
    };

    if poster_id != biz_id {
        return Err(AppError::Forbidden(
            "Only the RFQ owner can update it".into(),
        ));
    }

    if let Some(ref new_status) = req.status {
        let valid = ["open", "closed", "awarded"];
        if !valid.contains(&new_status.as_str()) {
            return Err(AppError::Validation(format!(
                "Invalid status. Must be one of: {}",
                valid.join(", ")
            )));
        }
        if new_status == "closed" && current_status != "awarded" {
            sqlx::query("UPDATE rfqs SET status = $1, updated_at = now() WHERE id = $2")
                .bind(new_status)
                .bind(id)
                .execute(&state.db)
                .await?;
            return Ok(Json(json!({ "id": id, "status": "closed" })));
        }
    }

    if let Some(ref new_title) = req.title {
        if new_title.trim().is_empty() {
            return Err(AppError::Validation("Title cannot be empty".into()));
        }
        sqlx::query("UPDATE rfqs SET title = $1, updated_at = now() WHERE id = $2")
            .bind(new_title)
            .bind(id)
            .execute(&state.db)
            .await?;
    }

    if let Some(ref new_desc) = req.description {
        sqlx::query("UPDATE rfqs SET description = $1, updated_at = now() WHERE id = $2")
            .bind(new_desc)
            .bind(id)
            .execute(&state.db)
            .await?;
    }

    Ok(Json(json!({
        "id": id,
        "status": req.status.unwrap_or(current_status),
    })))
}

// ── API: POST /b2b/rfqs/:id/bids ──

pub async fn submit_bid(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<Uuid>,
    Json(req): Json<BidRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.amount <= Decimal::ZERO {
        return Err(AppError::Validation(
            "Bid amount must be greater than zero".into(),
        ));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let bidder_id = resolve_business_id(&state.db, user_id).await?;

    let rfq = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT poster_business_id, status FROM rfqs WHERE id = $1",
    )
    .bind(rfq_id)
    .fetch_optional(&state.db)
    .await?;

    match rfq {
        Some((poster, status)) => {
            if poster == bidder_id {
                return Err(AppError::Validation(
                    "You cannot bid on your own RFQ".into(),
                ));
            }
            if status != "open" {
                return Err(AppError::Validation(format!(
                    "RFQ is not open (current status: {})",
                    status
                )));
            }
        }
        None => return Err(AppError::NotFound("RFQ not found".into())),
    }

    let bid_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rfq_bids (id, rfq_id, bidder_business_id, amount, details, delivery_timeline) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(bid_id)
    .bind(rfq_id)
    .bind(bidder_id)
    .bind(req.amount)
    .bind(&req.details)
    .bind(&req.delivery_timeline)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "id": bid_id,
        "rfq_id": rfq_id,
        "bidder_business_id": bidder_id,
        "status": "submitted",
    })))
}

// ── API: GET /b2b/rfqs/:id/bids ──

pub async fn list_bids(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let poster: Option<Uuid> =
        sqlx::query_scalar("SELECT poster_business_id FROM rfqs WHERE id = $1")
            .bind(rfq_id)
            .fetch_optional(&state.db)
            .await?;

    match poster {
        Some(p) if p == biz_id => {}
        Some(_) => return Err(AppError::Forbidden("Only RFQ owner can view bids".into())),
        None => return Err(AppError::NotFound("RFQ not found".into())),
    }

    let bids = sqlx::query_as::<_, RfqBidRow>(
        "SELECT bd.id, bd.rfq_id, bd.bidder_business_id, bd.amount, bd.details, \
         bd.delivery_timeline, bd.status, bd.created_at, bd.updated_at \
         FROM rfq_bids bd \
         WHERE bd.rfq_id = $1 ORDER BY bd.amount ASC, bd.created_at ASC",
    )
    .bind(rfq_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({ "bids": bids })))
}

// ── API: PATCH /b2b/rfqs/:id/bids/:bid_id/accept ──

pub async fn accept_bid(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((rfq_id, bid_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let rfq = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT poster_business_id, status FROM rfqs WHERE id = $1",
    )
    .bind(rfq_id)
    .fetch_optional(&state.db)
    .await?;

    match rfq {
        Some((poster, status)) => {
            if poster != biz_id {
                return Err(AppError::Forbidden("Only RFQ owner can accept bids".into()));
            }
            if status != "open" {
                return Err(AppError::Validation(format!("RFQ is already {}", status)));
            }
        }
        None => return Err(AppError::NotFound("RFQ not found".into())),
    }

    let bidder: Option<Uuid> =
        sqlx::query_scalar("SELECT bidder_business_id FROM rfq_bids WHERE id = $1 AND rfq_id = $2")
            .bind(bid_id)
            .bind(rfq_id)
            .fetch_optional(&state.db)
            .await?;

    let bidder_id = match bidder {
        Some(b) => b,
        None => return Err(AppError::NotFound("Bid not found".into())),
    };

    sqlx::query(
        "UPDATE rfqs SET status = 'awarded', awarded_to = $1, awarded_bid_id = $2, updated_at = now() WHERE id = $3",
    )
    .bind(bidder_id)
    .bind(bid_id)
    .bind(rfq_id)
    .execute(&state.db)
    .await?;

    sqlx::query("UPDATE rfq_bids SET status = 'accepted', updated_at = now() WHERE id = $1")
        .bind(bid_id)
        .execute(&state.db)
        .await?;

    Ok(Json(json!({
        "rfq_id": rfq_id,
        "accepted_bid_id": bid_id,
        "awarded_to": bidder_id,
        "status": "awarded",
    })))
}

// ── API: PATCH /b2b/rfqs/:id/bids/:bid_id/reject ──

pub async fn reject_bid(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((rfq_id, bid_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let poster: Option<Uuid> =
        sqlx::query_scalar("SELECT poster_business_id FROM rfqs WHERE id = $1")
            .bind(rfq_id)
            .fetch_optional(&state.db)
            .await?;

    match poster {
        Some(p) if p == biz_id => {}
        Some(_) => return Err(AppError::Forbidden("Only RFQ owner can reject bids".into())),
        None => return Err(AppError::NotFound("RFQ not found".into())),
    }

    let result = sqlx::query(
        "UPDATE rfq_bids SET status = 'rejected', updated_at = now() WHERE id = $1 AND rfq_id = $2",
    )
    .bind(bid_id)
    .bind(rfq_id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Bid not found".into()));
    }

    Ok(Json(json!({
        "rfq_id": rfq_id,
        "bid_id": bid_id,
        "status": "rejected",
    })))
}

// ── API: GET /b2b/rfqs/:id/messages ──

pub async fn get_rfq_messages(
    State(state): State<AppState>,
    Path(rfq_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let messages = sqlx::query_as::<_, RfqMessageRow>(
        "SELECT m.id, m.rfq_id, m.sender_business_id, m.message, m.created_at \
         FROM rfq_messages m WHERE m.rfq_id = $1 ORDER BY m.created_at ASC",
    )
    .bind(rfq_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({ "messages": messages })))
}

// ── API: POST /b2b/rfqs/:id/messages ──

pub async fn post_rfq_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rfq_id): Path<Uuid>,
    Json(req): Json<MessageRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.message.trim().is_empty() {
        return Err(AppError::Validation("Message cannot be empty".into()));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let sender_id = resolve_business_id(&state.db, user_id).await?;

    let exists: bool = sqlx::query_scalar("SELECT COUNT(*) FROM rfqs WHERE id = $1")
        .bind(rfq_id)
        .fetch_one(&state.db)
        .await
        .map(|c: i64| c > 0)
        .unwrap_or(false);

    if !exists {
        return Err(AppError::NotFound("RFQ not found".into()));
    }

    let msg_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rfq_messages (id, rfq_id, sender_business_id, message) VALUES ($1, $2, $3, $4)",
    )
    .bind(msg_id)
    .bind(rfq_id)
    .bind(sender_id)
    .bind(&req.message)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "id": msg_id,
        "rfq_id": rfq_id,
        "sender_business_id": sender_id,
        "status": "sent",
    })))
}
