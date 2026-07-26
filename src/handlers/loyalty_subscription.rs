//! Loyalty Subscription — paid plan gating for loyalty features.
//! Proxies subscription status, plan listings, and Stripe checkout to IncentiveSwift.
//!
//! Routes sit inside the auth guard (need MD JWT).
//! Resolves the MD user -> IS account (by email) and generates IS JWT for proxying.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::AppState;
use crate::auth::models::Claims;
use crate::error::{AppError, ApiResult};

const IS_BASE: &str = "http://127.0.0.1:8083/api/v1";

/// Request body for initiating a loyalty subscription.
#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub plan_slug: String,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
}

/// Available loyalty plan info.
#[derive(Debug, Serialize)]
pub(crate) struct PlanInfo {
    pub slug: String,
    pub name: String,
    pub monthly_price: i32,
    pub monthly_zc_pool: i32,
    pub features: Vec<String>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn http() -> Client {
    Client::new()
}

/// Fetch public plans list from IS (no auth required).
async fn fetch_public_plans() -> Vec<PlanInfo> {
    let url = format!("{}/loyalty/plans", IS_BASE);
    match http().get(&url).send().await {
        Ok(resp) => match resp.json::<Value>().await {
            Ok(json) => json
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|v| PlanInfo {
                            slug: v["slug"].as_str().unwrap_or("").to_string(),
                            name: v["name"].as_str().unwrap_or("").to_string(),
                            monthly_price: v["monthly_price"].as_i64().unwrap_or(0) as i32,
                            monthly_zc_pool: v["monthly_zc_pool"].as_i64().unwrap_or(0) as i32,
                            features: v["features"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|f| f.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => vec![],
        },
        Err(_) => vec![],
    }
}

/// Look up the IS account_id by email from the MD user record.
async fn resolve_is_account(
    db: &sqlx::PgPool,
    is_db: &sqlx::PgPool,
    md_claims: &Claims,
) -> Result<(String, String), AppError> {
    let user_id = Uuid::parse_str(&md_claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Try visitor_accounts first, then fall back to users table
    let email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM visitor_accounts WHERE id = $1"
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .map_err(|_| AppError::Internal("DB lookup failed".into()))?;

    let email = match email {
        Some(e) => e,
        None => {
            sqlx::query_scalar(
                "SELECT email FROM users WHERE id = $1"
            )
            .bind(user_id)
            .fetch_optional(db)
            .await
            .map_err(|_| AppError::Internal("DB lookup failed".into()))?
            .ok_or_else(|| AppError::NotFound("Account not found".into()))?
        }
    };

    // Look up IS account_id by email
    let is_account: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM accounts WHERE email = $1 LIMIT 1"
    )
    .bind(&email)
    .fetch_optional(is_db)
    .await
    .map_err(|_| AppError::Internal("IS lookup failed".into()))?;

    let account_id = is_account.unwrap_or_else(|| md_claims.sub.clone());

    Ok((account_id, email))
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/v1/business/loyalty/status
///
/// Called by the business portal to check the user's loyalty subscription status.
/// Fetches available plans from IS and the user's current plan/enrollment status.
pub async fn loyalty_status(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;

    // Fetch available plans from IS (public endpoint, no auth needed)
    let plans: Vec<PlanInfo> = fetch_public_plans().await;

    // Check IS plan status (using JWT-proxied endpoint)
    let plan_result = super::loyalty_proxy::proxy_get(
        "/loyalty/plan/status",
        &account_id,
        &email,
        &claims.role,
    ).await;

    match plan_result {
        Ok(status_json) => Ok(Json(json!({
            "has_account": true,
            "is_subscribed": status_json.get("enrolled").and_then(|v| v.as_bool()).unwrap_or(false),
            "plan": status_json.get("plan").and_then(|v| v.as_str()),
            "plan_status": status_json.get("status").and_then(|v| v.as_str()).unwrap_or("inactive"),
            "zc_pool_remaining": status_json.get("zc_pool_remaining").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            "zc_pool_total": status_json.get("zc_pool_total").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            "pool_reset_date": status_json.get("pool_reset_date").and_then(|v| v.as_str()),
            "plans_available": plans,
        }))),
        Err(_) => Ok(Json(json!({
            "has_account": false,
            "is_subscribed": false,
            "plan": null,
            "plan_status": "inactive",
            "zc_pool_remaining": 0,
            "zc_pool_total": 0,
            "pool_reset_date": null,
            "plans_available": plans,
        }))),
    }
}

/// POST /api/v1/business/loyalty/subscribe
///
/// Initiates a Stripe checkout session for the requested loyalty plan.
/// Forwards the subscription request to IncentiveSwift with the user's email.
pub async fn loyalty_subscribe(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<SubscribeRequest>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;

    let body = json!({
        "plan_slug": req.plan_slug,
        "email": email,
        "success_url": req.success_url.unwrap_or_else(|| "/business-portal".to_string()),
        "cancel_url": req.cancel_url.unwrap_or_else(|| "/business-portal".to_string()),
    });

    let result = super::loyalty_proxy::proxy_post(
        "/loyalty/subscribe",
        &body,
        &account_id,
        &email,
        &claims.role,
    ).await?;

    Ok(Json(result))
}
