//! Loyalty Proxy — routes portal loyalty requests to IncentiveSwift.
//! Handles: PIN, Credits, Vouchers, Referrals, Rewards, Pledges
//!
//! These routes sit INSIDE the auth guard (need MD JWT).
//! They resolve the MD user -> IS account (by email) and generate an IS-compatible
//! JWT on-the-fly to proxy the request through.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use reqwest::Client;
use serde_json::{json, Value};
use uuid::Uuid;
use jsonwebtoken::{encode, EncodingKey, Header};

use crate::AppState;
use crate::auth::models::Claims;
use crate::error::{AppError, ApiResult};

const IS_BASE: &str = "http://127.0.0.1:8083/api/v1";

fn http() -> Client {
    Client::new()
}

/// Generate an IS-compatible JWT using jsonwebtoken
fn make_is_jwt(account_id: &str, email: &str, role: &str, secret: &str) -> Result<String, AppError> {
    use std::collections::HashMap;
    let now = chrono::Utc::now().timestamp() as usize;
    let mut claims = HashMap::new();
    claims.insert("sub", serde_json::Value::String(account_id.to_string()));
    claims.insert("email", serde_json::Value::String(email.to_string()));
    claims.insert("role", serde_json::Value::String(role.to_string()));
    claims.insert("iat", serde_json::Value::Number(now.into()));
    claims.insert("exp", serde_json::Value::Number((now + 300).into()));

    let header = Header::new(jsonwebtoken::Algorithm::HS256);
    encode(&header, &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(format!("JWT encode failed: {}", e)))
}

/// Look up the IS account_id by email from MD's user
async fn resolve_is_account(db: &sqlx::PgPool, is_db: &sqlx::PgPool, md_claims: &Claims) -> Result<(String, String), AppError> {
    let user_id = Uuid::parse_str(&md_claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Get email from MD users table
    let email: String = sqlx::query_scalar(
        "SELECT email FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .map_err(|_| AppError::Internal("DB lookup failed".into()))?
    .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    // Look up IS account_id by email
    let is_account: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM accounts WHERE email = $1 LIMIT 1"
    )
    .bind(&email)
    .fetch_optional(is_db)
    .await
    .map_err(|_| AppError::Internal("IS lookup failed".into()))?;

    let account_id = match is_account {
        Some(id) => id,
        None => md_claims.sub.clone(),
    };

    Ok((account_id, email))
}

/// Proxy a GET to IS
async fn proxy_get(path: &str, account_id: &str, email: &str, role: &str) -> Result<Value, AppError> {
    let secret = std::env::var("IS_JWT_SECRET")
        .unwrap_or_else(|_| "rr0NC13QNMpmvuopQjOZFqQKxtq1JosBr/i/mZ+QyrHwryQzaVzWKA1htAEBN9WI".to_string());
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", IS_BASE, path);

    let resp = http()
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("IS parse failed: {}", e)))
}

/// Proxy a POST to IS
async fn proxy_post(path: &str, body: &Value, account_id: &str, email: &str, role: &str) -> Result<Value, AppError> {
    let secret = std::env::var("IS_JWT_SECRET")
        .unwrap_or_else(|_| "rr0NC13QNMpmvuopQjOZFqQKxtq1JosBr/i/mZ+QyrHwryQzaVzWKA1htAEBN9WI".to_string());
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", IS_BASE, path);

    let resp = http()
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("IS parse failed: {}", e)))
}

/// Proxy a PUT to IS
async fn proxy_put(path: &str, body: &Value, account_id: &str, email: &str, role: &str) -> Result<Value, AppError> {
    let secret = std::env::var("IS_JWT_SECRET")
        .unwrap_or_else(|_| "rr0NC13QNMpmvuopQjOZFqQKxtq1JosBr/i/mZ+QyrHwryQzaVzWKA1htAEBN9WI".to_string());
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", IS_BASE, path);

    let resp = http()
        .put(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("IS parse failed: {}", e)))
}

/// Proxy a PATCH to IS
/// Proxy a DELETE to IS
async fn proxy_delete(path: &str, account_id: &str, email: &str, role: &str) -> Result<Value, AppError> {
    let secret = std::env::var("IS_JWT_SECRET")
        .unwrap_or_else(|_| "rr0NC13QNMpmvuopQjOZFqQKxtq1JosBr/i/mZ+QyrHwryQzaVzWKA1htAEBN9WI".to_string());
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", IS_BASE, path);

    let resp = http()
        .delete(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("IS parse failed: {}", e)))
}

/// Proxy a PATCH to IS
async fn proxy_patch(path: &str, body: &Value, account_id: &str, email: &str, role: &str) -> Result<Value, AppError> {
    let secret = std::env::var("IS_JWT_SECRET")
        .unwrap_or_else(|_| "rr0NC13QNMpmvuopQjOZFqQKxtq1JosBr/i/mZ+QyrHwryQzaVzWKA1htAEBN9WI".to_string());
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", IS_BASE, path);

    let resp = http()
        .patch(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("IS parse failed: {}", e)))
}

// ── PIN ──

pub async fn pin_status(
    Extension(_claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(json!({
        "message": "Use POST /loyalty/pin/generate to create a PIN",
        "available": true
    })))
}

pub async fn pin_generate(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/loyalty/generate-pin", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

pub async fn pin_verify(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/loyalty/verify-purchase", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Credits ──

pub async fn credits_balance(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/credits/balance", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

pub async fn credits_history(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/credits/history", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Vouchers ──

pub async fn vouchers_list(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/loyalty/vouchers", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

pub async fn voucher_redeem(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/loyalty/claim-voucher", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Referrals ──

pub async fn referrals_list(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/loyalty/referrals", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

pub async fn referral_create(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/loyalty/referrals/create", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Rewards ──

pub async fn rewards_list(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/loyalty/rewards", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

pub async fn reward_claim(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/loyalty/redeem-reward", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Pledges ──

pub async fn pledges_list(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(&format!("/business/pledges/{}", aid), &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

pub async fn pledge_create(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/business/pledge", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── QR Code Endpoint ──

/// GET /loyalty/qr — return the user's account_id as QR payload
/// Also returns the tenant's credit_rate for display ("1 credit per $X").
pub async fn get_loyalty_qr(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;

    // Fetch credit_rate from IS accounts table (tenants table)
    let credit_rate: i32 = sqlx::query_scalar(
        "SELECT credit_rate FROM accounts WHERE id = $1::uuid"
    )
    .bind(&account_id)
    .fetch_optional(&s.is_db)
    .await
    .ok()
    .flatten()
    .unwrap_or(10);

    Ok(Json(json!({
        "qr_id": account_id,
        "qr_data": account_id,
        "email": email,
        "credit_rate": credit_rate,
    })))
}

// ── Purchase Verify Proxy ──

/// POST /loyalty/purchase/verify — proxies to IS purchase verify with auto-credit
pub async fn purchase_verify_proxy(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/loyalty/purchase/verify", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Credit Rate & Purchase PIN Settings (ZaarHub Admin proxy to IS) ──

/// GET /loyalty/admin/credit-rate — fetch tenant's credit_rate from IS
pub async fn get_credit_rate(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(&format!("/admin/tenants/{}/credits-rate", account_id), &account_id, &email, &claims.role).await?;
    Ok(Json(result))
}

/// PATCH /loyalty/admin/credit-rate — update tenant's credit_rate via IS
pub async fn update_credit_rate(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_patch(&format!("/admin/tenants/{}/credits-rate", account_id), &body, &account_id, &email, &claims.role).await?;
    Ok(Json(result))
}

/// GET /loyalty/admin/purchase-pin — fetch tenant's purchase_pin from IS
pub async fn get_purchase_pin(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(&format!("/admin/tenants/{}/purchase-pin", account_id), &account_id, &email, &claims.role).await?;
    Ok(Json(result))
}

// Note: purchase_pin is auto-generated on signup. Admin view only (read-only).

// ── Offers Proxy ──

/// GET /loyalty/admin/offers — list offers for the authenticated tenant
pub async fn offers_list(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get("/admin/offers", &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// POST /loyalty/admin/offers — create a new offer
pub async fn offers_create(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_post("/admin/offers", &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// GET /loyalty/admin/offers/:id — get a single offer
pub async fn offers_get(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(&format!("/admin/offers/{}", id), &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// PUT /loyalty/admin/offers/:id — update an offer
pub async fn offers_update(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_put(&format!("/admin/offers/{}", id), &body, &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

/// DELETE /loyalty/admin/offers/:id — delete (deactivate) an offer
pub async fn offers_delete(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_delete(&format!("/admin/offers/{}", id), &aid, &email, &claims.role).await?;
    Ok(Json(result))
}

// ── Portal Dashboard ──

pub async fn portal_dashboard(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let role = &claims.role;

    // Get business info from MD DB
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let biz_info = sqlx::query_as::<_, (i64, Option<String>, Option<String>)>(
        r#"SELECT
            (SELECT COUNT(*) FROM claimed_businesses WHERE user_id = $1) as cnt,
            (SELECT pt.name FROM business_subscriptions bs
             JOIN plan_tiers pt ON pt.id = bs.tier_id
             WHERE bs.business_id IN (SELECT business_id FROM claimed_businesses WHERE user_id = $1)
             ORDER BY bs.created_at DESC LIMIT 1) as tier_name,
            (SELECT bs.status FROM business_subscriptions bs
             WHERE bs.business_id IN (SELECT business_id FROM claimed_businesses WHERE user_id = $1)
             ORDER BY bs.created_at DESC LIMIT 1) as sub_status"#
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or((0, None, None));

    // Fetch IS data
    let credits = proxy_get("/credits/balance", &account_id, &email, role).await.unwrap_or(json!({"balance": 0}));
    let vouchers = proxy_get("/loyalty/vouchers", &account_id, &email, role).await.unwrap_or(json!({"vouchers": []}));
    let referrals = proxy_get("/loyalty/referrals", &account_id, &email, role).await.unwrap_or(json!({"referrals": [], "code": null}));
    let rewards = proxy_get("/loyalty/rewards", &account_id, &email, role).await.unwrap_or(json!({"rewards": []}));

    Ok(Json(json!({
        "business_count": biz_info.0 as usize,
        "subscription_tier": biz_info.1,
        "subscription_status": biz_info.2,
        "total_credits": credits.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "active_vouchers": vouchers.get("vouchers").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
        "referrer_code": referrals.get("code").and_then(|v| v.as_str()),
        "referral_count": referrals.get("referrals").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
        "available_rewards": rewards.get("rewards").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
        "account_id": account_id,
    })))
}
