//! Lead Sharing Network handlers — BL25
//! Businesses share leads they can't fulfill with other businesses.
//! Creates a referral economy that no generic search engine can replicate.

use axum::{
    extract::{Path, State, Query},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::auth::middleware::verify_token;
use crate::error::{AppError, ApiResult};

// ── Auth helpers ──

fn extract_user_id(headers: &HeaderMap, state: &AppState) -> ApiResult<Uuid> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;
    let claims = verify_token(token, &state.config.jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;
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
struct SharedLeadRow {
    id: Uuid,
    title: String,
    description: String,
    category: Option<String>,
    location: Option<String>,
    estimated_value: Option<Decimal>,
    source: Option<String>,
    poster_business_id: Uuid,
    status: String,
    claimed_by: Option<Uuid>,
    claimed_at: Option<chrono::DateTime<chrono::Utc>>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct LeadQuery {
    pub category: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ShareLeadRequest {
    pub title: String,
    pub description: String,
    pub category: Option<String>,
    pub location: Option<String>,
    pub estimated_value: Option<Decimal>,
    pub source: Option<String>,
    pub expires_in_days: Option<i32>,
}

// ── API: POST /b2b/leads ──

pub async fn share_lead(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ShareLeadRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.title.trim().is_empty() {
        return Err(AppError::Validation("Title is required".into()));
    }
    if req.description.trim().is_empty() {
        return Err(AppError::Validation("Description is required".into()));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let expires_at = req
        .expires_in_days
        .map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO shared_leads (id, title, description, category, location, estimated_value, \
         source, poster_business_id, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.category)
    .bind(&req.location)
    .bind(req.estimated_value)
    .bind(&req.source)
    .bind(biz_id)
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "id": id,
        "status": "available",
        "poster_business_id": biz_id,
    })))
}

// ── API: GET /b2b/leads/available ──

pub async fn available_leads(
    State(state): State<AppState>,
    Query(params): Query<LeadQuery>,
) -> ApiResult<impl IntoResponse> {
    let per_page = params.per_page.unwrap_or(20).min(100);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let leads = sqlx::query_as::<_, SharedLeadRow>(
        "SELECT id, title, description, category, location, estimated_value, \
         source, poster_business_id, status, claimed_by, claimed_at, expires_at, created_at, updated_at \
         FROM shared_leads \
         WHERE status = 'available' AND (expires_at IS NULL OR expires_at > now()) \
         ORDER BY created_at DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM shared_leads WHERE status = 'available' AND (expires_at IS NULL OR expires_at > now())",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    Ok(Json(json!({
        "leads": leads,
        "total": total,
        "page": page,
        "per_page": per_page,
    })))
}

// ── API: POST /b2b/leads/:id/claim ──

pub async fn claim_lead(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(lead_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let lead = sqlx::query_as::<_, (Uuid, String, Option<Uuid>)>(
        "SELECT poster_business_id, status, claimed_by FROM shared_leads WHERE id = $1 FOR UPDATE",
    )
    .bind(lead_id)
    .fetch_optional(&state.db)
    .await?;

    let (poster_id, status, claimed_by) = match lead {
        Some(l) => l,
        None => return Err(AppError::NotFound("Lead not found".into())),
    };

    if poster_id == biz_id {
        return Err(AppError::Validation(
            "You cannot claim your own lead".into(),
        ));
    }
    if status != "available" {
        return Err(AppError::Validation(format!(
            "Lead is not available (status: {})",
            status
        )));
    }
    if claimed_by.is_some() {
        return Err(AppError::Validation(
            "Lead has already been claimed".into(),
        ));
    }

    sqlx::query(
        "UPDATE shared_leads SET status = 'claimed', claimed_by = $1, claimed_at = now(), updated_at = now() WHERE id = $2",
    )
    .bind(biz_id)
    .bind(lead_id)
    .execute(&state.db)
    .await?;

    sqlx::query(
        "INSERT INTO lead_share_transactions (lead_id, from_business_id, to_business_id) VALUES ($1, $2, $3)",
    )
    .bind(lead_id)
    .bind(poster_id)
    .bind(biz_id)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "lead_id": lead_id,
        "claimed_by": biz_id,
        "status": "claimed",
    })))
}

// ── API: GET /b2b/leads/my ──

pub async fn my_leads(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let posted = sqlx::query_as::<_, SharedLeadRow>(
        "SELECT id, title, description, category, location, estimated_value, \
         source, poster_business_id, status, claimed_by, claimed_at, expires_at, created_at, updated_at \
         FROM shared_leads WHERE poster_business_id = $1 ORDER BY created_at DESC",
    )
    .bind(biz_id)
    .fetch_all(&state.db)
    .await?;

    let claimed = sqlx::query_as::<_, SharedLeadRow>(
        "SELECT id, title, description, category, location, estimated_value, \
         source, poster_business_id, status, claimed_by, claimed_at, expires_at, created_at, updated_at \
         FROM shared_leads WHERE claimed_by = $1 ORDER BY claimed_at DESC NULLS LAST",
    )
    .bind(biz_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({
        "posted_leads": posted,
        "claimed_leads": claimed,
    })))
}
