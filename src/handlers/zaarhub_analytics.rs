/// Analytics & Dashboard handlers for ZaarHub deal performance
use axum::{extract::{Query, State}, Json};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiResult;

/// Default pagination helpers
fn default_page(p: Option<i32>) -> i32 { p.unwrap_or(1).max(1) }
fn default_per_page(pp: Option<i32>) -> i32 { pp.unwrap_or(20).min(100).max(1) }
fn offset(page: i32, per_page: i32) -> i32 { (page - 1) * per_page }

#[derive(Deserialize)]
pub struct DateRangeQuery {
    pub start: Option<String>,
    pub end: Option<String>,
    pub page: Option<i32>,
    pub per_page: Option<i32>,
}

#[derive(Deserialize)]
pub struct CityStatsQuery {
    pub city_slug: Option<String>,
}

/// GET /api/v1/zaarhub/analytics/overview — overall performance summary
pub async fn overview(
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    // Total metrics
    let cities_row = sqlx::query("SELECT COUNT(*) AS cnt FROM city_pages WHERE is_active = true")
        .fetch_one(&state.db).await?;
    let listings_row = sqlx::query("SELECT COUNT(*) AS cnt FROM business_listings")
        .fetch_one(&state.db).await?;
    let offers_row = sqlx::query("SELECT COUNT(*) AS cnt FROM claim_offers WHERE is_active = true")
        .fetch_one(&state.db).await?;
    let claims_row = sqlx::query("SELECT COUNT(*) AS cnt FROM offer_claims")
        .fetch_one(&state.db).await?;
    let redeemed_row = sqlx::query("SELECT COUNT(*) AS cnt FROM offer_claims WHERE redeemed = true")
        .fetch_one(&state.db).await?;

    // Claims today
    let today_claims_row = sqlx::query(
        "SELECT COUNT(*) AS cnt FROM offer_claims WHERE claimed_at::date = CURRENT_DATE"
    ).fetch_one(&state.db).await?;

    // Weekly claims trend
    let weekly = sqlx::query(
        "SELECT claimed_at::date AS day, COUNT(*) AS cnt \
         FROM offer_claims \
         WHERE claimed_at >= CURRENT_DATE - INTERVAL '7 days' \
         GROUP BY day ORDER BY day"
    ).fetch_all(&state.db).await?;

    let weekly_trend: Vec<Value> = weekly.iter().map(|r| {
        let day: NaiveDate = r.try_get("day").unwrap_or(Utc::now().date_naive());
        let cnt: i64 = r.try_get("cnt").unwrap_or(0);
        json!({ "date": day.to_string(), "claims": cnt })
    }).collect();

    Ok(Json(json!({
        "totals": {
            "cities": cities_row.try_get::<i64,_>("cnt").unwrap_or(0),
            "listings": listings_row.try_get::<i64,_>("cnt").unwrap_or(0),
            "active_offers": offers_row.try_get::<i64,_>("cnt").unwrap_or(0),
            "total_claims": claims_row.try_get::<i64,_>("cnt").unwrap_or(0),
            "redeemed": redeemed_row.try_get::<i64,_>("cnt").unwrap_or(0),
            "redemption_rate": if claims_row.try_get::<i64,_>("cnt").unwrap_or(1) > 0 {
                format!("{:.1}%",
                    (redeemed_row.try_get::<i64,_>("cnt").unwrap_or(0) as f64
                     / claims_row.try_get::<i64,_>("cnt").unwrap_or(1) as f64) * 100.0)
            } else { "0%".to_string() },
            "claims_today": today_claims_row.try_get::<i64,_>("cnt").unwrap_or(0),
        },
        "weekly_trend": weekly_trend,
    })))
}

/// GET /api/v1/zaarhub/analytics/cities — per-city performance
pub async fn city_performance(
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT cp.city_slug, cp.city_name, \
                COUNT(DISTINCT bl.id) AS listing_count, \
                COUNT(DISTINCT co.id) AS offer_count, \
                COUNT(DISTINCT oc.id) AS claim_count, \
                COUNT(DISTINCT CASE WHEN oc.redeemed = true THEN oc.id END) AS redeemed_count \
         FROM city_pages cp \
         LEFT JOIN business_listings bl ON bl.city_page_id = cp.id \
         LEFT JOIN claim_offers co ON co.listing_id = bl.id AND co.is_active = true \
         LEFT JOIN offer_claims oc ON oc.offer_id = co.id \
         WHERE cp.is_active = true \
         GROUP BY cp.id, cp.city_slug, cp.city_name \
         ORDER BY claim_count DESC"
    ).fetch_all(&state.db).await?;

    let cities: Vec<Value> = rows.iter().map(|r| {
        let claims: i64 = r.try_get("claim_count").unwrap_or(0);
        let redeemed: i64 = r.try_get("redeemed_count").unwrap_or(0);
        json!({
            "city_slug": r.try_get::<String,_>("city_slug").unwrap_or_default(),
            "city_name": r.try_get::<String,_>("city_name").unwrap_or_default(),
            "listing_count": r.try_get::<i64,_>("listing_count").unwrap_or(0),
            "offer_count": r.try_get::<i64,_>("offer_count").unwrap_or(0),
            "claim_count": claims,
            "redeemed_count": redeemed,
            "redemption_rate": if claims > 0 {
                format!("{:.1}%", (redeemed as f64 / claims as f64) * 100.0)
            } else { "0%".to_string() },
        })
    }).collect();

    Ok(Json(json!({ "cities": cities })))
}

/// GET /api/v1/zaarhub/analytics/offers — top performing offers
pub async fn top_offers(
    State(state): State<AppState>,
    Query(q): Query<DateRangeQuery>,
) -> ApiResult<Json<Value>> {
    let page = default_page(q.page);
    let per_page = default_per_page(q.per_page);

    let total_row = sqlx::query(
        "SELECT COUNT(*) AS cnt FROM claim_offers co \
         JOIN business_listings bl ON co.listing_id = bl.id \
         WHERE co.is_active = true"
    ).fetch_one(&state.db).await?;

    let rows = sqlx::query(
        "SELECT co.id, co.offer_title, co.offer_type, co.discount_value, \
                bl.business_name, bl.category, \
                co.max_claims, co.current_claims, \
                COUNT(oc.id) AS total_claims, \
                COUNT(CASE WHEN oc.redeemed = true THEN 1 END) AS total_redeemed \
         FROM claim_offers co \
         JOIN business_listings bl ON co.listing_id = bl.id \
         LEFT JOIN offer_claims oc ON oc.offer_id = co.id \
         WHERE co.is_active = true \
         GROUP BY co.id, co.offer_title, co.offer_type, co.discount_value, bl.business_name, bl.category \
         ORDER BY total_claims DESC, total_redeemed DESC \
         LIMIT $1 OFFSET $2"
    ).bind(per_page as i64).bind(offset(page, per_page) as i64)
     .fetch_all(&state.db).await?;

    let total: i64 = total_row.try_get("cnt").unwrap_or(0);

    let offers: Vec<Value> = rows.iter().map(|r| {
        let claims: i64 = r.try_get("total_claims").unwrap_or(0);
        let redeemed: i64 = r.try_get("total_redeemed").unwrap_or(0);
        json!({
            "id": r.try_get::<Uuid,_>("id").map(|v| v.to_string()).unwrap_or_default(),
            "offer_title": r.try_get::<String,_>("offer_title").unwrap_or_default(),
            "offer_type": r.try_get::<String,_>("offer_type").unwrap_or_default(),
            "discount_value": r.try_get::<Option<String>,_>("discount_value").unwrap_or_default(),
            "business_name": r.try_get::<String,_>("business_name").unwrap_or_default(),
            "category": r.try_get::<Option<String>,_>("category").unwrap_or_default(),
            "max_claims": r.try_get::<Option<i32>,_>("max_claims").unwrap_or_default(),
            "current_claims": r.try_get::<i32,_>("current_claims").unwrap_or(0),
            "total_claims": claims,
            "total_redeemed": redeemed,
            "redemption_rate": if claims > 0 {
                format!("{:.1}%", (redeemed as f64 / claims as f64) * 100.0)
            } else { "0%".to_string() },
        })
    }).collect();

    let total_pages = if total == 0 { 0 } else { ((total as f64) / (per_page as f64)).ceil() as i32 };

    Ok(Json(json!({
        "offers": offers,
        "pagination": { "page": page, "per_page": per_page, "total": total, "total_pages": total_pages }
    })))
}

/// GET /api/v1/zaarhub/analytics/claims — recent claim activity
pub async fn recent_claims(
    State(state): State<AppState>,
    Query(q): Query<DateRangeQuery>,
) -> ApiResult<Json<Value>> {
    let page = default_page(q.page);
    let per_page = default_per_page(q.per_page);

    let total_row = sqlx::query("SELECT COUNT(*) AS cnt FROM offer_claims")
        .fetch_one(&state.db).await?;

    let rows = sqlx::query(
        "SELECT oc.id, oc.visitor_id, oc.email, oc.phone, \
                oc.promo_code_revealed, oc.claimed_at, oc.redeemed, \
                co.offer_title, co.offer_type, \
                bl.business_name \
         FROM offer_claims oc \
         JOIN claim_offers co ON oc.offer_id = co.id \
         JOIN business_listings bl ON co.listing_id = bl.id \
         ORDER BY oc.claimed_at DESC \
         LIMIT $1 OFFSET $2"
    ).bind(per_page as i64).bind(offset(page, per_page) as i64)
     .fetch_all(&state.db).await?;

    let total: i64 = total_row.try_get("cnt").unwrap_or(0);

    let claims: Vec<Value> = rows.iter().map(|r| {
        json!({
            "id": r.try_get::<Uuid,_>("id").map(|v| v.to_string()).unwrap_or_default(),
            "visitor_id": r.try_get::<String,_>("visitor_id").unwrap_or_default(),
            "email": r.try_get::<Option<String>,_>("email").unwrap_or_default(),
            "phone": r.try_get::<Option<String>,_>("phone").unwrap_or_default(),
            "promo_code": r.try_get::<String,_>("promo_code_revealed").unwrap_or_default(),
            "claimed_at": r.try_get::<chrono::NaiveDateTime,_>("claimed_at").unwrap_or_default(),
            "redeemed": r.try_get::<bool,_>("redeemed").unwrap_or(false),
            "offer_title": r.try_get::<String,_>("offer_title").unwrap_or_default(),
            "offer_type": r.try_get::<String,_>("offer_type").unwrap_or_default(),
            "business_name": r.try_get::<String,_>("business_name").unwrap_or_default(),
        })
    }).collect();

    let total_pages = if total == 0 { 0 } else { ((total as f64) / (per_page as f64)).ceil() as i32 };

    Ok(Json(json!({
        "claims": claims,
        "pagination": { "page": page, "per_page": per_page, "total": total, "total_pages": total_pages }
    })))
}

/// GET /api/v1/zaarhub/analytics/categories — category performance
pub async fn category_breakdown(
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT bl.category, \
                COUNT(DISTINCT bl.id) AS listing_count, \
                COUNT(DISTINCT co.id) AS offer_count, \
                COUNT(DISTINCT oc.id) AS claim_count \
         FROM business_listings bl \
         LEFT JOIN claim_offers co ON co.listing_id = bl.id AND co.is_active = true \
         LEFT JOIN offer_claims oc ON oc.offer_id = co.id \
         WHERE bl.category IS NOT NULL \
         GROUP BY bl.category \
         ORDER BY claim_count DESC, listing_count DESC \
         LIMIT 30"
    ).fetch_all(&state.db).await?;

    let categories: Vec<Value> = rows.iter().map(|r| {
        json!({
            "category": r.try_get::<String,_>("category").unwrap_or_default(),
            "listing_count": r.try_get::<i64,_>("listing_count").unwrap_or(0),
            "offer_count": r.try_get::<i64,_>("offer_count").unwrap_or(0),
            "claim_count": r.try_get::<i64,_>("claim_count").unwrap_or(0),
        })
    }).collect();

    Ok(Json(json!({ "categories": categories })))
}
