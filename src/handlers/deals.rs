//! Deal CRUD handlers for Multi-Directory API.

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
pub struct Deal {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub original_price: Option<String>,
    pub deal_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    pub terms: Option<String>,
    pub redemption_limit: Option<i32>,
    pub redemption_count: Option<i32>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Uuid,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub deal_type: Option<String>,
    pub coupon_code: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDealRequest {
    pub title: String,
    pub description: Option<String>,
    pub original_price: Option<String>,
    pub deal_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    pub terms: Option<String>,
    pub redemption_limit: Option<i32>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Uuid,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub deal_type: Option<String>,
    pub coupon_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDealRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub original_price: Option<String>,
    pub deal_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
    pub terms: Option<String>,
    pub redemption_limit: Option<i32>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub deal_type: Option<String>,
    pub coupon_code: Option<String>,
}

/// GET /api/v1/deals — list all deals
pub async fn list_deals(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let deals = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(deals))
}

/// GET /api/v1/deals/featured — featured deals across all directories
pub async fn list_featured_deals(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let deals = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals WHERE featured = true AND status = 'active' ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(deals))
}

/// POST /api/v1/deals — create a deal
pub async fn create_deal(
    State(s): State<AppState>,
    Json(req): Json<CreateDealRequest>,
) -> ApiResult<impl IntoResponse> {
    let deal = sqlx::query_as::<_, Deal>(
        "INSERT INTO deals (title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413, \x2414, \x2415, \x2416, \x2417) RETURNING id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at "
    )
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.original_price)
    .bind(&req.deal_price)
    .bind(req.discount_percent)
    .bind(req.currency.as_deref().unwrap_or("USD"))
    .bind(&req.image_url)
    .bind(&req.terms)
    .bind(req.redemption_limit)
    .bind(req.status.as_deref().unwrap_or("active"))
    .bind(req.directory_id)
    .bind(req.business_id)
    .bind(req.start_date)
    .bind(req.end_date)
    .bind(req.featured.unwrap_or(false))
    .bind(req.deal_type.as_deref().unwrap_or("coupon"))
    .bind(&req.coupon_code)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(deal)))
}

/// GET /api/v1/deals/:id — get single deal
pub async fn get_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let deal = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Deal not found".into()))?;

    Ok(Json(deal))
}

/// PUT /api/v1/deals/:id — update deal
pub async fn update_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDealRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Deal not found".into()))?;

    let title = req.title.unwrap_or(existing.title);
    let description = req.description.or(existing.description);
    let original_price = req.original_price.or(existing.original_price);
    let deal_price = req.deal_price.or(existing.deal_price);
    let discount_percent = req.discount_percent.or(existing.discount_percent);
    let currency = req.currency.or(existing.currency);
    let image_url = req.image_url.or(existing.image_url);
    let terms = req.terms.or(existing.terms);
    let redemption_limit = req.redemption_limit.or(existing.redemption_limit);
    let status = req.status.or(existing.status);
    let directory_id = req.directory_id.or(existing.directory_id);
    let business_id = req.business_id.unwrap_or(existing.business_id);
    let start_date = req.start_date.or(existing.start_date);
    let end_date = req.end_date.or(existing.end_date);
    let featured = req.featured.or(existing.featured);
    let deal_type = req.deal_type.or(existing.deal_type);
    let coupon_code = req.coupon_code.or(existing.coupon_code);

    let deal = sqlx::query_as::<_, Deal>(
        "UPDATE deals SET title = \x241, description = \x242, original_price = \x243, deal_price = \x244, discount_percent = \x245, currency = \x246, image_url = \x247, terms = \x248, redemption_limit = \x249, status = \x2410, directory_id = \x2411, business_id = \x2412, start_date = \x2413, end_date = \x2414, featured = \x2415, deal_type = \x2416, coupon_code = \x2417, updated_at = NOW() WHERE id = \x2418 RETURNING id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at "
    )
    .bind(&title)
    .bind(&description)
    .bind(&original_price)
    .bind(&deal_price)
    .bind(discount_percent)
    .bind(&currency)
    .bind(&image_url)
    .bind(&terms)
    .bind(redemption_limit)
    .bind(&status)
    .bind(directory_id)
    .bind(business_id)
    .bind(start_date)
    .bind(end_date)
    .bind(featured)
    .bind(&deal_type)
    .bind(&coupon_code)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(deal))
}

/// DELETE /api/v1/deals/:id — delete deal
pub async fn delete_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM deals WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Deal not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/deals/:id/claim — increment redemption_count
pub async fn claim_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let deal = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Deal not found".into()))?;

    if let Some(limit) = deal.redemption_limit {
        let count = deal.redemption_count.unwrap_or(0);
        if count >= limit {
            return Err(AppError::BadRequest("Redemption limit reached for this deal".into()));
        }
    }

    let updated = sqlx::query_as::<_, Deal>(
        "UPDATE deals SET redemption_count = COALESCE(redemption_count, 0) + 1, updated_at = NOW() WHERE id = \x241 RETURNING id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at "
    )
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(updated))
}

/// GET /api/v1/directories/:slug/deals — deals for a directory
pub async fn list_directory_deals(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let deals = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals WHERE directory_id = \x241 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(deals))
}

/// GET /api/v1/directories/:slug/businesses/:business_id/deals — deals for a business
pub async fn list_business_deals(
    State(s): State<AppState>,
    Path((slug, business_id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let deals = sqlx::query_as::<_, Deal>(
        "SELECT id, title, description, original_price, deal_price, discount_percent, currency, image_url, terms, redemption_limit, redemption_count, status, directory_id, business_id, start_date, end_date, featured, deal_type, coupon_code, created_at, updated_at FROM deals WHERE directory_id = \x241 AND business_id = \x242 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(deals))
}
