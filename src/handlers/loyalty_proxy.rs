//! Loyalty Proxy — routes portal loyalty requests to IncentiveSwift.
//! Handles: PIN, Credits, Vouchers, Referrals, Rewards, Pledges
//!
//! These routes sit INSIDE the auth guard (need MD JWT).
//! They resolve the MD user -> IS account (by email) and generate an IS-compatible
//! JWT on-the-fly to proxy the request through.
//!
//! Shared proxy utilities (make_is_jwt, resolve_is_account, proxy_get, etc.)
//! live in super::proxy_common.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::AppState;
use crate::auth::models::Claims;
use crate::error::{AppError, ApiResult};

use super::proxy_common::*;

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
        "SELECT credit_rate FROM accounts WHERE id = $1::uuid",
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
    let result = proxy_get(
        &format!("/admin/tenants/{}/credits-rate", account_id),
        &account_id,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// PATCH /loyalty/admin/credit-rate — update tenant's credit_rate via IS
pub async fn update_credit_rate(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_patch(
        &format!("/admin/tenants/{}/credits-rate", account_id),
        &body,
        &account_id,
        &email,
        &claims.role,
    )
    .await?;
    Ok(Json(result))
}

/// GET /loyalty/admin/purchase-pin — fetch tenant's purchase_pin from IS
pub async fn get_purchase_pin(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (account_id, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
    let result = proxy_get(
        &format!("/admin/tenants/{}/purchase-pin", account_id),
        &account_id,
        &email,
        &claims.role,
    )
    .await?;
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

// ── Loyalty Enrollment ─────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct EnrollRequest {
    pub directory_slug: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct EnrollResponse {
    pub success: bool,
    pub contact_id: Option<String>,
    pub member_id: Option<String>,
    pub loyalty_program_id: Option<String>,
    pub loyalty_program_name: Option<String>,
    pub already_existed: bool,
    pub message: String,
}

/// POST /loyalty/enroll
/// Opt-in enrollment into the IncentiveSwift loyalty program.
/// Called from portal CTAs (visitor, business, supplier dashboards).
/// Looks up the authenticated user's email + member type from MD visitor_accounts
/// and fires register_member_in_is() to IS.
pub async fn enroll(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<EnrollRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // Look up visitor account details from MD
    #[derive(sqlx::FromRow)]
    struct VisitorRow {
        email: String,
        name: Option<String>,
        phone: Option<String>,
        business_type: Option<String>,
        directory_slug: Option<String>,
    }

    let visitor = sqlx::query_as::<_, VisitorRow>(
        r#"SELECT
            email,
            name,
            phone,
            business_type,
            COALESCE(
                (SELECT slug FROM tenants WHERE id = visitor_accounts.directory_id),
                'zaarhub'
            ) as directory_slug
           FROM visitor_accounts
           WHERE id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("DB lookup failed".into()))?
    .ok_or_else(|| {
        AppError::NotFound(
            "Visitor account not found. Complete signup + survey first.".into(),
        )
    })?;

    let directory_slug = body
        .directory_slug
        .unwrap_or(visitor.directory_slug.unwrap_or_else(|| "zaarhub".to_string()));

    // Determine member_type from business_type
    let member_type: &str = match visitor.business_type.as_deref() {
        Some("supplier") | Some("farm") | Some("wholesaler") | Some("distributor") => "supplier",
        Some("business") | Some("service") => "business_owner",
        _ => "visitor",
    };

    // Split name into first/last
    let first_name = visitor.name.clone();
    let last_name = first_name.as_ref().and_then(|n| {
        let parts: Vec<&str> = n.splitn(2, ' ').collect();
        parts.get(1).map(|s| s.to_string())
    });

    // Call IS register-member via fire-and-forget, but await so we can return status
    crate::handlers::tag_sync::register_member_in_is(
        visitor.email.clone(),
        first_name,
        last_name,
        visitor.phone,
        member_type,
        visitor.business_type,
        Some(directory_slug),
        None, // tags not needed for enrollment
    )
    .await;

    Ok(Json(json!({
        "success": true,
        "contact_id": null,
        "member_id": null,
        "loyalty_program_id": null,
        "loyalty_program_name": member_type_to_program(member_type),
        "already_existed": false,
        "message": format!("Enrolled as {} in the loyalty program", member_type),
    })))
}

fn member_type_to_program(member_type: &str) -> &str {
    match member_type {
        "supplier" => "ZaarHub B2B Loop",
        _ => "ZaarHub Local Pass",
    }
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
             ORDER BY bs.created_at DESC LIMIT 1) as sub_status"#,
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or((0, None, None));

    // Fetch IS data
    let credits = proxy_get("/credits/balance", &account_id, &email, role)
        .await
        .unwrap_or(json!({"balance": 0}));
    let vouchers = proxy_get("/loyalty/vouchers", &account_id, &email, role)
        .await
        .unwrap_or(json!({"vouchers": []}));
    let referrals = proxy_get("/loyalty/referrals", &account_id, &email, role)
        .await
        .unwrap_or(json!({"referrals": [], "code": null}));
    let rewards = proxy_get("/loyalty/rewards", &account_id, &email, role)
        .await
        .unwrap_or(json!({"rewards": []}));

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
