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
    pub plan_sales_page_url: Option<String>,
    pub payment_provider: Option<String>,
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
    pub external_payment_ref: Option<String>,
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
    pub external_payment_ref: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DirectoryTier {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub tier_slug: String,
    pub tier_name: String,
    pub is_active: Option<bool>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub stripe_subscription_id: Option<String>,
    pub stripe_customer_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub plan_tier_id: Option<Uuid>,
    pub external_plan_id: Option<String>,
    pub external_checkout_url: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SponsoredListing {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub business_id: Uuid,
    pub slot_position: Option<i32>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub is_active: Option<bool>,
    pub price_paid: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub stripe_payment_intent_id: Option<String>,
    pub external_payment_ref: Option<String>,
    pub featured: Option<bool>,
    pub badge_text: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
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
            featured_listing, description, plan_sales_page_url, payment_provider)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        RETURNING id, name, slug, price_monthly, price_yearly, max_listings, max_deals, max_photos,
            has_reviews, has_analytics, has_crm, has_email, has_call_tracking, has_import_export, has_api_access,
            featured_listing, description, plan_sales_page_url, payment_provider, created_at"#
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
    .bind(body.get("plan_sales_page_url").and_then(|v| v.as_str()))
    .bind(body.get("payment_provider").and_then(|v| v.as_str()))
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
            featured_listing = $15, description = $16,
            plan_sales_page_url = $17,
            payment_provider = $18
        WHERE id = $19
        RETURNING id, name, slug, price_monthly, price_yearly, max_listings, max_deals, max_photos,
            has_reviews, has_analytics, has_crm, has_email, has_call_tracking, has_import_export, has_api_access,
            featured_listing, description, plan_sales_page_url, payment_provider, created_at"#
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
    .bind(body.get("plan_sales_page_url").and_then(|v| v.as_str()).or(existing.plan_sales_page_url.as_deref()))
    .bind(body.get("payment_provider").and_then(|v| v.as_str()).or(existing.payment_provider.as_deref()))
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
        r#"INSERT INTO business_subscriptions (business_id, tier_id, status, billing_cycle, price_paid, currency, start_date, end_date, auto_renew, stripe_subscription_id, external_payment_ref)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, business_id, tier_id, status, billing_cycle, price_paid, currency, start_date, end_date, auto_renew, stripe_subscription_id, external_payment_ref, created_at"#
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
    .bind(body.get("external_payment_ref").and_then(|v| v.as_str()))
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
            auto_renew = $9, stripe_subscription_id = $10,
            external_payment_ref = $11
        WHERE id = $12
        RETURNING id, business_id, tier_id, status, billing_cycle, price_paid, currency, start_date, end_date, auto_renew, stripe_subscription_id, external_payment_ref, created_at"#
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
    .bind(body.get("external_payment_ref").and_then(|v| v.as_str()).or(existing.external_payment_ref.as_deref()))
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
        r#"INSERT INTO ad_zones (name, zone_key, width, height, price_monthly, directory_id, status, current_advertiser_id, current_ad_url, current_ad_image, external_payment_ref)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, name, zone_key, width, height, price_monthly, directory_id, status, current_advertiser_id, current_ad_url, current_ad_image, external_payment_ref, created_at"#
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
    .bind(body.get("external_payment_ref").and_then(|v| v.as_str()))
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
            current_advertiser_id = $8, current_ad_url = $9, current_ad_image = $10,
            external_payment_ref = $11
        WHERE id = $12
        RETURNING id, name, zone_key, width, height, price_monthly, directory_id, status, current_advertiser_id, current_ad_url, current_ad_image, external_payment_ref, created_at"#
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
    .bind(body.get("external_payment_ref").and_then(|v| v.as_str()).or(existing.external_payment_ref.as_deref()))
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


// ── Directory Tiers ─────────────────────────────────────────────────────────

pub async fn list_directory_tiers(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let tiers = sqlx::query_as::<_, DirectoryTier>(
        "SELECT * FROM directory_tiers ORDER BY created_at DESC"
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(json!(tiers)))
}

pub async fn create_directory_tier(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let directory_id = body.get("directory_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::Validation("directory_id is required (UUID)".into()))?;

    let tier_slug = body.get("tier_slug").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("tier_slug is required".into()))?;
    let tier_name = body.get("tier_name").and_then(|v| v.as_str()).unwrap_or("Free");

    let expires_at = body.get("expires_at").and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let dt = sqlx::query_as::<_, DirectoryTier>(
        r#"
INSERT INTO directory_tiers (directory_id, tier_slug, tier_name, is_active, expires_at, stripe_subscription_id, stripe_customer_id, metadata, plan_tier_id, external_plan_id, external_checkout_url)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
ON CONFLICT (directory_id) DO UPDATE SET
    tier_slug = EXCLUDED.tier_slug,
    tier_name = EXCLUDED.tier_name,
    is_active = EXCLUDED.is_active,
    expires_at = EXCLUDED.expires_at,
    stripe_subscription_id = EXCLUDED.stripe_subscription_id,
    stripe_customer_id = EXCLUDED.stripe_customer_id,
    metadata = EXCLUDED.metadata,
    plan_tier_id = EXCLUDED.plan_tier_id,
    external_plan_id = EXCLUDED.external_plan_id,
    external_checkout_url = EXCLUDED.external_checkout_url,
    updated_at = NOW()
RETURNING id, directory_id, tier_slug, tier_name, is_active, started_at, expires_at,
    stripe_subscription_id, stripe_customer_id, metadata, plan_tier_id, external_plan_id, external_checkout_url, created_at, updated_at
"#
    )
    .bind(directory_id)
    .bind(tier_slug)
    .bind(tier_name)
    .bind(body.get("is_active").and_then(|v| v.as_bool()).unwrap_or(true))
    .bind(expires_at)
    .bind(body.get("stripe_subscription_id").and_then(|v| v.as_str()))
    .bind(body.get("stripe_customer_id").and_then(|v| v.as_str()))
    .bind(body.get("metadata").cloned().unwrap_or(serde_json::Value::Object(serde_json::Map::new())))
    .bind(body.get("plan_tier_id").and_then(|v| v.as_str()).and_then(|v| Uuid::parse_str(v).ok()))
    .bind(body.get("external_plan_id").and_then(|v| v.as_str()))
    .bind(body.get("external_checkout_url").and_then(|v| v.as_str()))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(dt))))
}

pub async fn get_directory_tier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dt = sqlx::query_as::<_, DirectoryTier>(
        "SELECT * FROM directory_tiers WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory tier not found".into()))?;
    Ok(Json(json!(dt)))
}

pub async fn update_directory_tier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, DirectoryTier>(
        "SELECT * FROM directory_tiers WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory tier not found".into()))?;

    let expires_at = body.get("expires_at").and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or(existing.expires_at);

    let dt = sqlx::query_as::<_, DirectoryTier>(
        r#"
UPDATE directory_tiers SET
    tier_slug = $1, tier_name = $2, is_active = $3,
    expires_at = $4, stripe_subscription_id = $5,
    stripe_customer_id = $6, metadata = $7,
    plan_tier_id = $8, external_plan_id = $9, external_checkout_url = $10,
    updated_at = NOW()
WHERE id = $11
RETURNING id, directory_id, tier_slug, tier_name, is_active, started_at, expires_at,
    stripe_subscription_id, stripe_customer_id, metadata, plan_tier_id, external_plan_id, external_checkout_url, created_at, updated_at
"#
    )
    .bind(body.get("tier_slug").and_then(|v| v.as_str()).unwrap_or(&existing.tier_slug))
    .bind(body.get("tier_name").and_then(|v| v.as_str()).unwrap_or(&existing.tier_name))
    .bind(body.get("is_active").and_then(|v| v.as_bool()).unwrap_or(existing.is_active.unwrap_or(true)))
    .bind(expires_at)
    .bind(body.get("stripe_subscription_id").and_then(|v| v.as_str()).or(existing.stripe_subscription_id.as_deref()))
    .bind(body.get("stripe_customer_id").and_then(|v| v.as_str()).or(existing.stripe_customer_id.as_deref()))
    .bind(body.get("metadata").cloned().or(existing.metadata))
    .bind(body.get("plan_tier_id").and_then(|v| v.as_str()).and_then(|v| Uuid::parse_str(v).ok()).or(existing.plan_tier_id))
    .bind(body.get("external_plan_id").and_then(|v| v.as_str()).or(existing.external_plan_id.as_deref()))
    .bind(body.get("external_checkout_url").and_then(|v| v.as_str()).or(existing.external_checkout_url.as_deref()))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(dt)))
}

pub async fn delete_directory_tier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM directory_tiers WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Directory tier not found".into()));
    }

    Ok(Json(json!({"message": "Directory tier deleted"})))
}

pub async fn directory_tier_by_slug(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let dt = sqlx::query_as::<_, DirectoryTier>(
        "SELECT * FROM directory_tiers WHERE directory_id = $1"
    )
    .bind(dir.0)
    .fetch_optional(&s.db)
    .await?;

    Ok(Json(json!(dt)))
}

// ── Sponsored Listings ──────────────────────────────────────────────────────

pub async fn list_sponsored_listings(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let listings = sqlx::query_as::<_, SponsoredListing>(
        "SELECT * FROM sponsored_listings ORDER BY slot_position ASC, created_at DESC"
    )
    .fetch_all(&s.db)
    .await?;
    Ok(Json(json!(listings)))
}

pub async fn create_sponsored_listing(
    State(s): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let directory_id = body.get("directory_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::Validation("directory_id is required (UUID)".into()))?;
    let business_id = body.get("business_id").and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::Validation("business_id is required (UUID)".into()))?;

    let start_date_str = body.get("start_date").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("start_date is required (YYYY-MM-DD)".into()))?;
    let start_date = chrono::NaiveDate::parse_from_str(start_date_str, "%Y-%m-%d")
        .map_err(|_| AppError::Validation("Invalid start_date".into()))?;

    let end_date_str = body.get("end_date").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Validation("end_date is required (YYYY-MM-DD)".into()))?;
    let end_date = chrono::NaiveDate::parse_from_str(end_date_str, "%Y-%m-%d")
        .map_err(|_| AppError::Validation("Invalid end_date".into()))?;

    let listing = sqlx::query_as::<_, SponsoredListing>(
        r#"
INSERT INTO sponsored_listings (directory_id, business_id, slot_position, start_date, end_date, is_active, price_paid, currency, stripe_payment_intent_id, external_payment_ref, featured, badge_text, metadata)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
RETURNING id, directory_id, business_id, slot_position, start_date, end_date, is_active, price_paid, currency, stripe_payment_intent_id, external_payment_ref, featured, badge_text, metadata, created_at, updated_at
"#
    )
    .bind(directory_id)
    .bind(business_id)
    .bind(body.get("slot_position").and_then(|v| v.as_i64()).map(|v| v as i32).unwrap_or(1))
    .bind(start_date)
    .bind(end_date)
    .bind(body.get("is_active").and_then(|v| v.as_bool()).unwrap_or(true))
    .bind(body.get("price_paid").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()))
    .bind(body.get("currency").and_then(|v| v.as_str()))
    .bind(body.get("stripe_payment_intent_id").and_then(|v| v.as_str()))
    .bind(body.get("external_payment_ref").and_then(|v| v.as_str()))
    .bind(body.get("featured").and_then(|v| v.as_bool()).unwrap_or(false))
    .bind(body.get("badge_text").and_then(|v| v.as_str()))
    .bind(body.get("metadata").cloned().unwrap_or(serde_json::Value::Object(serde_json::Map::new())))
    .bind(body.get("plan_tier_id").and_then(|v| v.as_str()).and_then(|v| Uuid::parse_str(v).ok()))
    .bind(body.get("external_plan_id").and_then(|v| v.as_str()))
    .bind(body.get("external_checkout_url").and_then(|v| v.as_str()))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(listing))))
}

pub async fn get_sponsored_listing(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let listing = sqlx::query_as::<_, SponsoredListing>(
        "SELECT * FROM sponsored_listings WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Sponsored listing not found".into()))?;
    Ok(Json(json!(listing)))
}

pub async fn update_sponsored_listing(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, SponsoredListing>(
        "SELECT * FROM sponsored_listings WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Sponsored listing not found".into()))?;

    let start_date = if let Some(s) = body.get("start_date").and_then(|v| v.as_str()) {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    } else {
        existing.start_date
    };

    let end_date = if let Some(s) = body.get("end_date").and_then(|v| v.as_str()) {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    } else {
        existing.end_date
    };

    let listing = sqlx::query_as::<_, SponsoredListing>(
        r#"
UPDATE sponsored_listings SET
    slot_position = $1, start_date = $2, end_date = $3,
    is_active = $4, price_paid = $5, currency = $6,
    stripe_payment_intent_id = $7, external_payment_ref = $8, featured = $9, badge_text = $10,
    metadata = $11, updated_at = NOW()
WHERE id = $12
RETURNING id, directory_id, business_id, slot_position, start_date, end_date, is_active, price_paid, currency, stripe_payment_intent_id, external_payment_ref, featured, badge_text, metadata, created_at, updated_at
"#
    )
    .bind(body.get("slot_position").and_then(|v| v.as_i64()).map(|v| v as i32).or(existing.slot_position))
    .bind(start_date)
    .bind(end_date)
    .bind(body.get("is_active").and_then(|v| v.as_bool()).or(existing.is_active))
    .bind(body.get("price_paid").and_then(|v| v.as_f64()).map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default()).or(existing.price_paid))
    .bind(body.get("currency").and_then(|v| v.as_str()).or(existing.currency.as_deref()))
    .bind(body.get("stripe_payment_intent_id").and_then(|v| v.as_str()).or(existing.stripe_payment_intent_id.as_deref()))
    .bind(body.get("external_payment_ref").and_then(|v| v.as_str()).or(existing.external_payment_ref.as_deref()))
    .bind(body.get("featured").and_then(|v| v.as_bool()).or(existing.featured))
    .bind(body.get("badge_text").and_then(|v| v.as_str()).or(existing.badge_text.as_deref()))
    .bind(body.get("metadata").cloned().or(existing.metadata))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(listing)))
}

pub async fn delete_sponsored_listing(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM sponsored_listings WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Sponsored listing not found".into()));
    }

    Ok(Json(json!({"message": "Sponsored listing deleted"})))
}

pub async fn directory_sponsored_listings(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let listings = sqlx::query_as::<_, SponsoredListing>(
        "SELECT * FROM sponsored_listings WHERE directory_id = $1 AND is_active = true ORDER BY slot_position ASC"
    )
    .bind(dir.0)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(listings)))
}


// ?????? Monetization Dashboard ??????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????????

/// GET /api/v1/monetization
pub async fn monetization_dashboard(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    use rust_decimal::Decimal;

    let tier_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM plan_tiers"
    )
    .fetch_one(&s.db)
    .await?;

    let subscription_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM business_subscriptions"
    )
    .fetch_one(&s.db)
    .await?;

    let active_subscriptions: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM business_subscriptions WHERE status = 'active'"
    )
    .fetch_one(&s.db)
    .await?;

    let ad_zone_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ad_zones"
    )
    .fetch_one(&s.db)
    .await?;

    let directory_tier_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM directory_tiers"
    )
    .fetch_one(&s.db)
    .await?;

    let sponsored_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sponsored_listings"
    )
    .fetch_one(&s.db)
    .await?;

    let active_sponsored: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sponsored_listings WHERE is_active = true AND end_date >= CURRENT_DATE"
    )
    .fetch_one(&s.db)
    .await?;

    let total_revenue_row: Option<(Option<Decimal>,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(price_paid), 0) FROM business_subscriptions WHERE price_paid IS NOT NULL"
    )
    .fetch_optional(&s.db)
    .await?;
    let total_subscription_revenue = total_revenue_row.and_then(|r| r.0);

    let sponsored_revenue_row: Option<(Option<Decimal>,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(price_paid), 0) FROM sponsored_listings WHERE price_paid IS NOT NULL"
    )
    .fetch_optional(&s.db)
    .await?;
    let total_sponsored_revenue = sponsored_revenue_row.and_then(|r| r.0);

    Ok(Json(json!({
        "plan_tiers": tier_count.0,
        "subscriptions": {
            "total": subscription_count.0,
            "active": active_subscriptions.0,
        },
        "ad_zones": ad_zone_count.0,
        "directory_tiers": directory_tier_count.0,
        "sponsored_listings": {
            "total": sponsored_count.0,
            "active": active_sponsored.0,
        },
        "revenue": {
            "subscriptions": total_subscription_revenue,
            "sponsored": total_sponsored_revenue,
        },
        "status": "ok"
    })))
}

/// GET /api/v1/subscriptions/plans — list available plan tiers with feature access
pub async fn list_plans(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let plans = sqlx::query_as::<_, (Uuid, String, rust_decimal::Decimal, rust_decimal::Decimal, Option<String>, Option<serde_json::Value>, Option<i32>)>(
        "SELECT id, name, price_monthly, price_yearly, description, feature_access, max_listings FROM plan_tiers ORDER BY price_monthly ASC"
    )
    .fetch_all(&s.db)
    .await?;

    let result: Vec<serde_json::Value> = plans.into_iter().map(|p| json!({
        "id": p.0, "name": p.1, "price_monthly": p.2, "price_yearly": p.3,
        "description": p.4, "features": p.5, "max_listings": p.6
    })).collect();

    Ok(Json(json!({"plans": result})))
}

/// POST /api/v1/subscriptions/upgrade — upgrade a business subscription (self-serve)
#[derive(Debug, Deserialize)]
pub struct UpgradeRequest {
    pub business_id: Uuid,
    pub plan_id: Uuid,
    pub billing_cycle: Option<String>,
}

pub async fn upgrade_subscription(
    State(s): State<AppState>,
    Json(req): Json<UpgradeRequest>,
) -> ApiResult<impl IntoResponse> {
    let plan = sqlx::query_as::<_, (String, rust_decimal::Decimal, rust_decimal::Decimal, Option<serde_json::Value>)>(
        "SELECT name, price_monthly, price_yearly, feature_access FROM plan_tiers WHERE id = $1"
    )
    .bind(req.plan_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

    let price: f64 = if req.billing_cycle.as_deref() == Some("yearly") { plan.2.try_into().unwrap_or(0.0) } else { plan.1.try_into().unwrap_or(0.0) };

    sqlx::query(
        "INSERT INTO business_subscriptions (id, business_id, plan_name, price, currency, billing_cycle, status, start_date, auto_renew)          VALUES ($1, $2, $3, $4, 'USD', $5, 'active', NOW(), true)          ON CONFLICT (business_id) DO UPDATE SET plan_name = $3, price = $4, status = 'active', updated_at = NOW()"
    )
    .bind(Uuid::new_v4())
    .bind(req.business_id)
    .bind(&plan.0)
    .bind(price)
    .bind(req.billing_cycle.as_deref().unwrap_or("monthly"))
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "upgraded", "plan": plan.0, "price": price.to_string(), "features": plan.3})))
}

/// POST /api/v1/subscriptions/downgrade — downgrade or cancel
#[derive(Debug, Deserialize)]
pub struct DowngradeRequest {
    pub business_id: Uuid,
    pub plan_id: Option<Uuid>,
}

pub async fn downgrade_subscription(
    State(s): State<AppState>,
    Json(req): Json<DowngradeRequest>,
) -> ApiResult<impl IntoResponse> {
    if let Some(plan_id) = req.plan_id {
        let plan = sqlx::query_as::<_, (String, f64)>(
            "SELECT name, price_monthly FROM plan_tiers WHERE id = $1"
        )
        .bind(plan_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Plan not found".into()))?;

        sqlx::query(
            "UPDATE business_subscriptions SET plan_name = $1, price = $2, status = 'active', updated_at = NOW() WHERE business_id = $3"
        )
        .bind(&plan.0)
        .bind(plan.1)
        .bind(req.business_id)
        .execute(&s.db)
        .await?;

        Ok(Json(json!({"status": "downgraded", "plan": plan.0})))
    } else {
        sqlx::query(
            "UPDATE business_subscriptions SET status = 'cancelled', updated_at = NOW() WHERE business_id = $1"
        )
        .bind(req.business_id)
        .execute(&s.db)
        .await?;

        Ok(Json(json!({"status": "cancelled"})))
    }
}

/// GET /api/v1/subscriptions/features — check feature access for a business
pub async fn check_feature_access(
    State(s): State<AppState>,
    Query(qs): Query<FeatureCheckQuery>,
) -> ApiResult<impl IntoResponse> {
    let features = sqlx::query_as::<_, (Option<String>, Option<serde_json::Value>, Option<i32>)>(
        r#"SELECT bs.plan_name, pt.feature_access, pt.max_listings
           FROM business_subscriptions bs
           LEFT JOIN plan_tiers pt ON LOWER(pt.name) = LOWER(bs.plan_name)
           WHERE bs.business_id = $1 AND bs.status = 'active'"#
    )
    .bind(qs.business_id)
    .fetch_optional(&s.db)
    .await?;

    match features {
        Some((plan_name, feature_access, max_cats)) => {
            Ok(Json(json!({
                "plan": plan_name,
                "features": feature_access.unwrap_or(serde_json::json!({})),
                "max_listings": max_cats.unwrap_or(1)
            })))
        }
        None => Ok(Json(json!({
            "plan": "Listed",
            "features": {
                "deals": false, "community_posts": false, "blogging": false,
                "b2b_access": false, "multi_category": false, "custom_branding": false
            },
            "max_listings": 1
        })))
    }
}

#[derive(Debug, Deserialize)]
pub struct FeatureCheckQuery {
    pub business_id: Uuid,
}

/// POST /api/v1/businesses/:id/categories — manage multi-category assignment
#[derive(Debug, Deserialize)]
pub struct UpdateCategoriesRequest {
    pub category_ids: Vec<Uuid>,
    pub primary_category_id: Option<Uuid>,
}

pub async fn update_business_categories(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCategoriesRequest>,
) -> ApiResult<impl IntoResponse> {
    // Check max categories from subscription
    let sub_info = sqlx::query_as::<_, (Option<i32>,)>(
        "SELECT pt.max_listings FROM business_subscriptions bs          LEFT JOIN plan_tiers pt ON LOWER(pt.name) = LOWER(bs.plan_name)          WHERE bs.business_id = $1 AND bs.status = 'active'"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or((Some(1),));

    let max_cats = sub_info.0.unwrap_or(1) as usize;
    if req.category_ids.len() > max_cats {
        return Err(AppError::BadRequest(format!("Your plan allows max {} categories. Upgrade to add more.", max_cats)));
    }

    // Remove existing
    sqlx::query("DELETE FROM business_categories WHERE business_id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    // Insert new
    for (i, cat_id) in req.category_ids.iter().enumerate() {
        let is_primary = Some(*cat_id) == req.primary_category_id || (req.primary_category_id.is_none() && i == 0);
        sqlx::query(
            "INSERT INTO business_categories (business_id, category_id, is_primary) VALUES ($1, $2, $3)"
        )
        .bind(id)
        .bind(cat_id)
        .bind(is_primary)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({"status": "updated", "categories": req.category_ids.len()})))
}

/// GET /api/v1/businesses/:id/categories — list all categories for a business
pub async fn list_business_categories(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let cats = sqlx::query_as::<_, (Uuid, String, bool)>(
        r#"SELECT bc.category_id, dc.name, bc.is_primary
           FROM business_categories bc
           LEFT JOIN directory_categories dc ON dc.id = bc.category_id
           WHERE bc.business_id = $1
           ORDER BY bc.is_primary DESC, dc.name ASC"#
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    let result: Vec<serde_json::Value> = cats.into_iter().map(|c| json!({
        "id": c.0, "name": c.1, "is_primary": c.2
    })).collect();

    Ok(Json(result))
}
