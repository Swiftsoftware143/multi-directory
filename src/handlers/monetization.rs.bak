//! Monetization handlers — Plan Tiers, Business Subscriptions, and Ad Zones.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Data Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PlanTier {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub max_listings: Option<i32>,
    pub max_deals: Option<i32>,
    pub max_photos: Option<i32>,
    pub has_reviews: Option<bool>,
    pub has_analytics: Option<bool>,
    pub has_crm: Option<bool>,
    pub has_email: Option<bool>,
    pub has_call_tracking: Option<bool>,
    pub has_import_export: Option<bool>,
    pub has_api_access: Option<bool>,
    pub featured_listing: Option<bool>,
    pub description: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct BusinessSubscription {
    pub id: Uuid,
    pub business_id: Uuid,
    pub tier_id: Option<Uuid>,
    pub status: Option<String>,
    pub billing_cycle: Option<String>,
    pub price_paid: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub auto_renew: Option<bool>,
    pub stripe_subscription_id: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AdZone {
    pub id: Uuid,
    pub name: String,
    pub zone_key: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub current_advertiser_id: Option<Uuid>,
    pub current_ad_url: Option<String>,
    pub current_ad_image: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ── Plan Tiers ──────────────────────────────────────────────────────────────

/// GET /api/v1/tiers
pub async fn list_tiers(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let tiers = sqlx::query_as::<_, PlanTier>(
        "SELECT * FROM plan_tiers ORDER BY price_monthly ASC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(tiers)))
}

/// POST /api/v1/tiers
pub async fn create_tier(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let name = body.get("name").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("name is required".to_string()))?;
    let slug = body.get("slug").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("slug is required".to_string()))?;

    let tier = sqlx::query_as::<_, PlanTier>(
        r#"INSERT INTO plan_tiers (name, slug, price_monthly, price_yearly, max_listings, max_deals, max_photos,
            has_reviews, has_analytics, has_crm, has_email, has_call_tracking, has_import_export, has_api_access,
            featured_listing, description)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        RETURNING id, name, slug, price_monthly, price_yearly, max_listings, max_deals, max_photos,
            has_reviews, has_analytics, has_crm, has_email, has_call_tracking, has_import_export, has_api_access,
            featured_listing, description, created_at"#
    )
    .bind(name)
    .bind(slug)
    .bind(body.get("price_monthly").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(body.get("price_yearly").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(body.get("max_listings").and_then(|v| v.as_i64()).map(|v| v as i32))
    .bind(body.get("max_deals").and_then(|v| v.as_i64()).map(|v| v as i32))
    .bind(body.get("max_photos").and_then(|v| v.as_i64()).map(|v| v as i32))
    .bind(body.get("has_reviews").and_then(|v| v.as_bool()))
    .bind(body.get("has_analytics").and_then(|v| v.as_bool()))
    .bind(body.get("has_crm").and_then(|v| v.as_bool()))
    .bind(body.get("has_email").and_then(|v| v.as_bool()))
    .bind(body.get("has_call_tracking").and_then(|v| v.as_bool()))
    .bind(body.get("has_import_export").and_then(|v| v.as_bool()))
    .bind(body.get("has_api_access").and_then(|v| v.as_bool()))
    .bind(body.get("featured_listing").and_then(|v| v.as_bool()))
    .bind(body.get("description").and_then(|v| v.as_str()))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(tier))))
}

/// GET /api/v1/tiers/:id
pub async fn get_tier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let tier = sqlx::query_as::<_, PlanTier>(
        "SELECT * FROM plan_tiers WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Plan tier not found".to_string()))?;

    Ok(Json(json!(tier)))
}

/// PUT /api/v1/tiers/:id
pub async fn update_tier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, PlanTier>(
        "SELECT * FROM plan_tiers WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Plan tier not found".to_string()))?;

    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or(&existing.name);
    let slug = body.get("slug").and_then(|v| v.as_str()).unwrap_or(&existing.slug);

    let tier = sqlx::query_as::<_, PlanTier>(
        r#"UPDATE plan_tiers SET
            name = $1, slug = $2,
            price_monthly = $3, price_yearly = $4,
            max_listings = $5, max_deals = $6, max_photos = $7,
            has_reviews = $8, has_analytics = $9, has_crm = $10,
            has_email = $11, has_call_tracking = $12,
            has_import_export = $13, has_api_access = $14,
            featured_listing = $15, description = $16
        WHERE id = $17
        RETURNING id, name, slug, price_monthly, price_yearly, max_listings, max_deals, max_photos,
            has_reviews, has_analytics, has_crm, has_email, has_call_tracking, has_import_export, has_api_access,
            featured_listing, description, created_at"#
    )
    .bind(name)
    .bind(slug)
    .bind(body.get("price_monthly").and_then(|v| v.as_f64()).or(
        existing.price_monthly.as_ref().and_then(|d| d.to_string().parse::<f64>().ok())
    ).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(body.get("price_yearly").and_then(|v| v.as_f64()).or(
        existing.price_yearly.as_ref().and_then(|d| d.to_string().parse::<f64>().ok())
    ).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(body.get("max_listings").and_then(|v| v.as_i64()).map(|v| v as i32).or(existing.max_listings))
    .bind(body.get("max_deals").and_then(|v| v.as_i64()).map(|v| v as i32).or(existing.max_deals))
    .bind(body.get("max_photos").and_then(|v| v.as_i64()).map(|v| v as i32).or(existing.max_photos))
    .bind(body.get("has_reviews").and_then(|v| v.as_bool()).or(existing.has_reviews))
    .bind(body.get("has_analytics").and_then(|v| v.as_bool()).or(existing.has_analytics))
    .bind(body.get("has_crm").and_then(|v| v.as_bool()).or(existing.has_crm))
    .bind(body.get("has_email").and_then(|v| v.as_bool()).or(existing.has_email))
    .bind(body.get("has_call_tracking").and_then(|v| v.as_bool()).or(existing.has_call_tracking))
    .bind(body.get("has_import_export").and_then(|v| v.as_bool()).or(existing.has_import_export))
    .bind(body.get("has_api_access").and_then(|v| v.as_bool()).or(existing.has_api_access))
    .bind(body.get("featured_listing").and_then(|v| v.as_bool()).or(existing.featured_listing))
    .bind(body.get("description").and_then(|v| v.as_str()).or(existing.description.as_deref()))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(tier)))
}

/// DELETE /api/v1/tiers/:id
pub async fn delete_tier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM plan_tiers WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Plan tier not found".to_string()));
    }

    Ok(Json(json!({"message": "Plan tier deleted"})))
}

// ── Subscriptions ───────────────────────────────────────────────────────────

/// GET /api/v1/subscriptions
pub async fn list_subscriptions(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let subs = sqlx::query_as::<_, BusinessSubscription>(
        "SELECT * FROM business_subscriptions ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(subs)))
}

/// POST /api/v1/subscriptions
pub async fn create_subscription(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let business_id = body.get("business_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::Validation("business_id is required (UUID)".to_string()))?;

    let tier_id = body.get("tier_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok());

    let start_date_str = body.get("start_date").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("start_date is required (YYYY-MM-DD)".to_string()))?;
    let start_date = chrono::NaiveDate::parse_from_str(start_date_str, "%Y-%m-%d")
        .map_err(|_| AppError::Validation("Invalid start_date format, use YYYY-MM-DD".to_string()))?;

    let end_date_str = body.get("end_date").and_then(|v| v.as_str());
    let end_date = end_date_str.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

    let sub = sqlx::query_as::<_, BusinessSubscription>(
        r#"INSERT INTO business_subscriptions (business_id, tier_id, status, billing_cycle, price_paid, currency, start_date, end_date, auto_renew, stripe_subscription_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, business_id, tier_id, status, billing_cycle, price_paid, currency, start_date, end_date, auto_renew, stripe_subscription_id, created_at"#
    )
    .bind(business_id)
    .bind(tier_id)
    .bind(body.get("status").and_then(|v| v.as_str()))
    .bind(body.get("billing_cycle").and_then(|v| v.as_str()))
    .bind(body.get("price_paid").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(body.get("currency").and_then(|v| v.as_str()))
    .bind(start_date)
    .bind(end_date)
    .bind(body.get("auto_renew").and_then(|v| v.as_bool()))
    .bind(body.get("stripe_subscription_id").and_then(|v| v.as_str()))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(sub))))
}

/// GET /api/v1/subscriptions/:id
pub async fn get_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let sub = sqlx::query_as::<_, BusinessSubscription>(
        "SELECT * FROM business_subscriptions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Subscription not found".to_string()))?;

    Ok(Json(json!(sub)))
}

/// PUT /api/v1/subscriptions/:id
pub async fn update_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, BusinessSubscription>(
        "SELECT * FROM business_subscriptions WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Subscription not found".to_string()))?;

    let business_id = body.get("business_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .unwrap_or(existing.business_id);

    let tier_id = if body.get("tier_id").is_some() {
        body.get("tier_id").and_then(|v| v.as_str())
            .and_then(|v| Uuid::parse_str(v).ok())
    } else {
        existing.tier_id
    };

    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or(existing.status.as_deref().unwrap_or("active"));
    let billing_cycle = body.get("billing_cycle").and_then(|v| v.as_str()).unwrap_or(existing.billing_cycle.as_deref().unwrap_or("monthly"));

    let start_date = if let Some(s) = body.get("start_date").and_then(|v| v.as_str()) {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| AppError::Validation("Invalid start_date format".to_string()))?
    } else {
        existing.start_date.unwrap_or_else(|| chrono::Utc::now().date_naive())
    };

    let end_date = if body.get("end_date").is_some() {
        body.get("end_date").and_then(|v| v.as_str())
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
    } else {
        existing.end_date
    };

    let price_paid = if body.get("price_paid").is_some() {
        body.get("price_paid").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default())
    } else {
        existing.price_paid
    };

    let sub = sqlx::query_as::<_, BusinessSubscription>(
        r#"UPDATE business_subscriptions SET
            business_id = $1, tier_id = $2, status = $3, billing_cycle = $4,
            price_paid = $5, currency = $6, start_date = $7, end_date = $8,
            auto_renew = $9, stripe_subscription_id = $10
        WHERE id = $11
        RETURNING id, business_id, tier_id, status, billing_cycle, price_paid, currency, start_date, end_date, auto_renew, stripe_subscription_id, created_at"#
    )
    .bind(business_id)
    .bind(tier_id)
    .bind(status)
    .bind(billing_cycle)
    .bind(price_paid)
    .bind(body.get("currency").and_then(|v| v.as_str()).or(existing.currency.as_deref()))
    .bind(start_date)
    .bind(end_date)
    .bind(body.get("auto_renew").and_then(|v| v.as_bool()).or(existing.auto_renew))
    .bind(body.get("stripe_subscription_id").and_then(|v| v.as_str()).or(existing.stripe_subscription_id.as_deref()))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(sub)))
}

/// DELETE /api/v1/subscriptions/:id
pub async fn delete_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM business_subscriptions WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Subscription not found".to_string()));
    }

    Ok(Json(json!({"message": "Subscription deleted"})))
}

/// GET /api/v1/businesses/:id/subscription
pub async fn business_subscription(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let sub = sqlx::query_as::<_, BusinessSubscription>(
        "SELECT * FROM business_subscriptions WHERE business_id = \x241 ORDER BY created_at DESC LIMIT 1 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("No subscription found for this business".to_string()))?;

    Ok(Json(json!(sub)))
}

// ── Ad Zones ────────────────────────────────────────────────────────────────

/// GET /api/v1/ad-zones
pub async fn list_ad_zones(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let zones = sqlx::query_as::<_, AdZone>(
        "SELECT * FROM ad_zones ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(zones)))
}

/// POST /api/v1/ad-zones
pub async fn create_ad_zone(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let name = body.get("name").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("name is required".to_string()))?;
    let zone_key = body.get("zone_key").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("zone_key is required".to_string()))?;

    let directory_id = body.get("directory_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok());

    let zone = sqlx::query_as::<_, AdZone>(
        r#"INSERT INTO ad_zones (name, zone_key, width, height, price_monthly, directory_id, status, current_advertiser_id, current_ad_url, current_ad_image)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, name, zone_key, width, height, price_monthly, directory_id, status, current_advertiser_id, current_ad_url, current_ad_image, created_at"#
    )
    .bind(name)
    .bind(zone_key)
    .bind(body.get("width").and_then(|v| v.as_i64()).map(|v| v as i32))
    .bind(body.get("height").and_then(|v| v.as_i64()).map(|v| v as i32))
    .bind(body.get("price_monthly").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(directory_id)
    .bind(body.get("status").and_then(|v| v.as_str()))
    .bind(body.get("current_advertiser_id").and_then(|v| v.as_str()).and_then(|v| Uuid::parse_str(v).ok()))
    .bind(body.get("current_ad_url").and_then(|v| v.as_str()))
    .bind(body.get("current_ad_image").and_then(|v| v.as_str()))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(zone))))
}

/// GET /api/v1/ad-zones/:id
pub async fn get_ad_zone(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let zone = sqlx::query_as::<_, AdZone>(
        "SELECT * FROM ad_zones WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Ad zone not found".to_string()))?;

    Ok(Json(json!(zone)))
}

/// PUT /api/v1/ad-zones/:id
pub async fn update_ad_zone(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, AdZone>(
        "SELECT * FROM ad_zones WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Ad zone not found".to_string()))?;

    let zone = sqlx::query_as::<_, AdZone>(
        r#"UPDATE ad_zones SET
            name = $1, zone_key = $2, width = $3, height = $4,
            price_monthly = $5, directory_id = $6, status = $7,
            current_advertiser_id = $8, current_ad_url = $9, current_ad_image = $10
        WHERE id = $11
        RETURNING id, name, zone_key, width, height, price_monthly, directory_id, status, current_advertiser_id, current_ad_url, current_ad_image, created_at"#
    )
    .bind(body.get("name").and_then(|v| v.as_str()).unwrap_or(&existing.name))
    .bind(body.get("zone_key").and_then(|v| v.as_str()).unwrap_or(&existing.zone_key))
    .bind(body.get("width").and_then(|v| v.as_i64()).map(|v| v as i32).or(existing.width))
    .bind(body.get("height").and_then(|v| v.as_i64()).map(|v| v as i32).or(existing.height))
    .bind(body.get("price_monthly").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()).or(existing.price_monthly))
    .bind(body.get("directory_id").and_then(|v| v.as_str()).and_then(|v| Uuid::parse_str(v).ok()).or(existing.directory_id))
    .bind(body.get("status").and_then(|v| v.as_str()).or(existing.status.as_deref()))
    .bind(body.get("current_advertiser_id").and_then(|v| v.as_str()).and_then(|v| Uuid::parse_str(v).ok()).or(existing.current_advertiser_id))
    .bind(body.get("current_ad_url").and_then(|v| v.as_str()).or(existing.current_ad_url.as_deref()))
    .bind(body.get("current_ad_image").and_then(|v| v.as_str()).or(existing.current_ad_image.as_deref()))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(zone)))
}

/// DELETE /api/v1/ad-zones/:id
pub async fn delete_ad_zone(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM ad_zones WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Ad zone not found".to_string()));
    }

    Ok(Json(json!({"message": "Ad zone deleted"})))
}

/// GET /api/v1/directories/:slug/ad-zones
pub async fn directory_ad_zones(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let zones = sqlx::query_as::<_, AdZone>(
        "SELECT * FROM ad_zones WHERE directory_id = \x241 ORDER BY created_at DESC "
    )
    .bind(dir.0)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(zones)))
}
