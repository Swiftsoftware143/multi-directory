//! Referral System — refer users, earn Zaarcash.
//!
//! Stage 5: Any user can refer any other user (visitor→visitor, business→business,
//! business→visitor, visitor→business). Referral earns Zaarcash after manual
//! verification by an admin. Business-referring-business gets a bonus multiplier.
//!
//! Zaarcash is awarded via IncentiveSwift's grant-credits endpoint.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use tracing;

use crate::AppState;
use crate::auth::models::Claims;
use crate::auth::middleware::{is_admin, is_business_owner, is_visitor};
use crate::error::{AppError, ApiResult};

lazy_static::lazy_static! {
    static ref HTTP: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build reqwest client");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct GenerateReferralRequest {
    pub direction: Option<String>, // auto-detect from role if not provided
}

#[derive(Debug, Serialize)]
pub struct GenerateReferralResponse {
    pub referral_code: String,
    pub referral_link: String,
    pub direction: String,
}

#[derive(Debug, Deserialize)]
pub struct ClaimReferralRequest {
    pub referral_code: String,
    pub referee_email: Option<String>,
    pub referee_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminReferralAction {
    pub note: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Handler: Generate referral code
// ═══════════════════════════════════════════════════════════════════════════════

/// POST /api/v1/referrals/generate — generates a unique referral code for the caller
///
/// The direction is auto-detected from the caller's role:
///   - visitor → visitor_to_visitor
///   - business_owner → business_to_business
///   - admin → visitor_to_visitor (default)

fn extract_claims_from_headers(headers: &HeaderMap, jwt_secret: &str) -> Result<Claims, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    crate::auth::middleware::verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)
}

pub async fn generate_referral(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let role = &claims.role;

    // Determine referrer type and direction
    let direction_input = body.get("direction").and_then(|v| v.as_str());
    let (referrer_type, direction): (String, String) = match direction_input {
        Some(dir) => {
            let rt = match role.as_str() {
                "visitor" => "visitor",
                "business_owner" => "business",
                _ => "user",
            };
            (rt.to_string(), dir.to_string())
        }
        None => {
            let (rt, dir) = match role.as_str() {
                "visitor" => ("visitor", "visitor_to_visitor"),
                "business_owner" => ("business", "business_to_business"),
                _ => ("user", "visitor_to_visitor"),
            };
            (rt.to_string(), dir.to_string())
        }
    };

    let referrer_email = resolve_referrer_email(&s.db, role, user_id).await?;

    // Check if they already have a code
    let existing_code: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT referral_code FROM referrals WHERE referrer_id = $1::text::uuid AND referrer_type = $2 AND status NOT IN ('expired') LIMIT 1"
    )
    .bind(user_id.to_string())
    .bind(&referrer_type)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    if let Some(code) = existing_code {
        return Ok(Json(json!(GenerateReferralResponse {
            referral_code: code.clone(),
            referral_link: format!("zaarhub.com/join?ref={}", code),
            direction: direction.clone(),
        })));
    }

    // Generate unique code
    let code: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    // Create referral record
    let uid_str = user_id.to_string();
    sqlx::query(
        r#"INSERT INTO referrals
           (referrer_type, referrer_id, referrer_email, referee_type, referral_code, direction, status)
           VALUES ($1, $2, $3, 'visitor', $4, $5, 'pending')"#
    )
    .bind(&referrer_type)
    .bind(&uid_str)
    .bind(&referrer_email)
    .bind(&code)
    .bind(&direction)
    .execute(&s.db)
    .await?;

    // Return the generated response
    Ok(Json(json!(GenerateReferralResponse {
        referral_code: code.clone(),
        referral_link: format!("zaarhub.com/join?ref={}", code),
        direction,
    })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Handler: Claim referral
// ═══════════════════════════════════════════════════════════════════════════════

/// POST /api/v1/referrals/claim — public endpoint, called when a new user signs up
/// via a referral link. Creates a referral record with referee info, status = 'pending'.
pub async fn claim_referral(
    State(s): State<AppState>,
    Json(req): Json<ClaimReferralRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.referral_code.trim().is_empty() {
        return Err(AppError::Validation("referral_code is required".to_string()));
    }

    // Find the referral generator record
    let referral: Option<(Uuid, String, Uuid, String)> = sqlx::query_as(
        r#"SELECT id, referrer_type, referrer_id, direction
           FROM referrals
           WHERE referral_code = $1 AND status = 'pending'
             AND referee_id IS NULL
           LIMIT 1"#
    )
    .bind(&req.referral_code)
    .fetch_optional(&s.db)
    .await?;

    let (referral_id, referrer_type, referrer_id, direction) = match referral {
        Some(r) => r,
        None => {
            // Check if already claimed
            let claimed: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM referrals WHERE referral_code = $1 AND referee_id IS NOT NULL LIMIT 1"
            )
            .bind(&req.referral_code)
            .fetch_optional(&s.db)
            .await?
            .flatten();

            if claimed.is_some() {
                return Err(AppError::Duplicate("This referral code has already been used".to_string()));
            }

            return Err(AppError::NotFound("Invalid referral code".to_string()));
        }
    };

    // Update the referral with referee info
    sqlx::query(
        r#"UPDATE referrals
           SET referee_email = $1, referee_name = $2, updated_at = NOW()
           WHERE id = $3"#
    )
    .bind(&req.referee_email)
    .bind(&req.referee_name)
    .bind(referral_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "success": true,
        "message": "Welcome! Your referral is pending verification.",
        "referral_id": referral_id,
    })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Handler: Referral balance
// ═══════════════════════════════════════════════════════════════════════════════

/// GET /api/v1/referrals/balance — returns Zaarcash balance for the caller
pub async fn get_referral_balance(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let role = &claims.role;

    let email = resolve_referrer_email(&s.db, role, user_id).await?;

    // Query IncentiveSwift for balance
    let balance = query_is_balance(&s, &email).await?;

    // Also get total earned from referrals
    let total_earned: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(zaarcash_earned), 0) FROM referrals WHERE referrer_email = $1 AND status = 'paid'"
    )
    .bind(&email)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    Ok(Json(json!({
        "success": true,
        "zaarcash_balance": balance,
        "total_earned": total_earned,
        "email": email,
    })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Handler: Referral code info (public, no auth)
// ═══════════════════════════════════════════════════════════════════════════════

/// GET /api/v1/referrals/code/:code — get info about a referral code (public)
pub async fn get_referral_code_info(
    State(s): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let info: Option<(String, String)> = sqlx::query_as(
        "SELECT referrer_type, direction FROM referrals WHERE referral_code = $1 LIMIT 1"
    )
    .bind(&code)
    .fetch_optional(&s.db)
    .await?;

    match info {
        Some((referrer_type, direction)) => Ok(Json(json!({
            "success": true,
            "valid": true,
            "referrer_type": referrer_type,
            "direction": direction,
        }))),
        None => Ok(Json(json!({
            "success": true,
            "valid": false,
        }))),
    }
}

/// GET /api/v1/referrals/my-code — get the current user's referral code
pub async fn get_my_referral_code(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let role = &claims.role;

    let referrer_type = match role.as_str() {
        "visitor" => "visitor",
        "business_owner" => "business",
        _ => "user",
    };

    let code: Option<String> = sqlx::query_scalar(
        "SELECT referral_code FROM referrals WHERE referrer_id = $1 AND referrer_type = $2 AND status != 'expired' LIMIT 1"
    )
    .bind(user_id)
    .bind(referrer_type)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    match code {
        Some(c) => Ok(Json(json!({
            "success": true,
            "referral_code": c,
            "referral_link": format!("zaarhub.com/join?ref={}", c),
            "referrer_type": referrer_type,
        }))),
        None => Ok(Json(json!({
            "success": true,
            "referral_code": null,
            "referral_link": null,
            "referrer_type": referrer_type,
        }))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Admin Handlers
// ═══════════════════════════════════════════════════════════════════════════════

/// GET /api/v1/admin/referrals — list all referrals with optional status filter
pub async fn admin_list_referrals(
    State(s): State<AppState>,
    headers: HeaderMap,
    req: axum::extract::Query<AdminReferralFilter>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let filter_status = req.status.as_deref().unwrap_or("");

    let rows = if filter_status.is_empty() {
        sqlx::query_as::<_, ReferralRow>(
            r#"SELECT id, referrer_type, referrer_id, referrer_email, referee_type, referee_id,
                      referee_email, referee_name, referral_code, direction, status,
                      zaarcash_earned, verified_at, created_at, updated_at
               FROM referrals ORDER BY created_at DESC LIMIT 200"#
        )
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, ReferralRow>(
            r#"SELECT id, referrer_type, referrer_id, referrer_email, referee_type, referee_id,
                      referee_email, referee_name, referral_code, direction, status,
                      zaarcash_earned, verified_at, created_at, updated_at
               FROM referrals WHERE status = $1 ORDER BY created_at DESC LIMIT 200"#
        )
        .bind(filter_status)
        .fetch_all(&s.db)
        .await?
    };

    // Summary stats
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM referrals")
        .fetch_one(&s.db).await.unwrap_or(0);
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM referrals WHERE status = 'pending'")
        .fetch_one(&s.db).await.unwrap_or(0);
    let total_zaarcash: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(zaarcash_earned), 0) FROM referrals WHERE status = 'paid'")
        .fetch_one(&s.db).await.unwrap_or(0);

    Ok(Json(json!({
        "success": true,
        "referrals": rows,
        "stats": {
            "total": total,
            "pending": pending,
            "total_zaarcash_awarded": total_zaarcash,
        }
    })))
}

#[derive(Debug, Deserialize)]
pub struct AdminReferralFilter {
    pub status: Option<String>,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct ReferralRow {
    pub id: Uuid,
    pub referrer_type: String,
    pub referrer_id: Uuid,
    pub referrer_email: Option<String>,
    pub referee_type: String,
    pub referee_id: Option<Uuid>,
    pub referee_email: Option<String>,
    pub referee_name: Option<String>,
    pub referral_code: String,
    pub direction: String,
    pub status: String,
    pub zaarcash_earned: i32,
    pub verified_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// POST /api/v1/admin/referrals/:id/verify — verify a referral and grant Zaarcash
pub async fn admin_verify_referral(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(referral_id): Path<Uuid>,
    Json(req): Json<AdminReferralAction>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Get referral info
    let referral: Option<(String, String, Option<String>, String, i32)> = sqlx::query_as(
        r#"SELECT referrer_type, direction, referrer_email, status, zaarcash_earned
           FROM referrals WHERE id = $1"#
    )
    .bind(referral_id)
    .fetch_optional(&s.db)
    .await?;

    let (referrer_type, direction, referrer_email, status, already_earned) = referral
        .ok_or(AppError::NotFound("Referral not found".to_string()))?;

    if status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Referral is already '{}'. Only pending referrals can be verified.",
            status
        )));
    }

    if already_earned > 0 {
        return Err(AppError::BadRequest("Referral already has Zaarcash awarded".to_string()));
    }

    // Calculate Zaarcash based on direction
    let zaarcash = match direction.as_str() {
        "visitor_to_visitor" => 50,
        "business_to_business" => 200,  // 100 base × 2 multiplier
        "business_to_visitor" => 50,
        "visitor_to_business" => 100,
        _ => 50, // default
    };

    // Update referral record to 'verified' first
    sqlx::query(
        "UPDATE referrals SET status = 'verified', zaarcash_earned = $1, verified_at = NOW(), updated_at = NOW() WHERE id = $2"
    )
    .bind(zaarcash)
    .bind(referral_id)
    .execute(&s.db)
    .await?;

    // Grant Zaarcash via IncentiveSwift
    if let Some(ref email) = referrer_email {
        match grant_zaarcash_via_is(&s, email, zaarcash, &direction).await {
            Ok(_) => {
                // Mark as paid
                sqlx::query(
                    "UPDATE referrals SET status = 'paid', updated_at = NOW() WHERE id = $1"
                )
                .bind(referral_id)
                .execute(&s.db)
                .await?;

                tracing::info!("[referrals] Referral {} verified and {} Zaarcash granted to {}",
                    referral_id, zaarcash, email);
            }
            Err(e) => {
                tracing::warn!("[referrals] Referral {} verified but IS grant failed: {}. Keeping as 'verified'.", referral_id, e);
                // Keep as 'verified' — admin can retry
                return Ok(Json(json!({
                    "success": true,
                    "referral_id": referral_id,
                    "status": "verified",
                    "zaarcash_earned": zaarcash,
                    "incentiveswift_status": "pending_retry",
                    "message": format!("Referral verified ({} Zaarcash). IncentiveSwift grant pending: {}", zaarcash, e),
                })));
            }
        }
    }

    let note = req.note.as_deref().unwrap_or("Verified by admin");

    Ok(Json(json!({
        "success": true,
        "referral_id": referral_id,
        "status": "paid",
        "zaarcash_earned": zaarcash,
        "direction": direction,
        "referrer_type": referrer_type,
        "note": note,
        "message": format!("Referral verified and {} Zaarcash granted.", zaarcash),
    })))
}

/// POST /api/v1/admin/referrals/:id/reject — reject a referral
pub async fn admin_reject_referral(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(referral_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let status: String = sqlx::query_scalar(
        "SELECT status FROM referrals WHERE id = $1"
    )
    .bind(referral_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Referral not found".to_string()))?;

    if status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Only pending referrals can be rejected. Current status: {}",
            status
        )));
    }

    sqlx::query("UPDATE referrals SET status = 'expired', updated_at = NOW() WHERE id = $1")
        .bind(referral_id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({
        "success": true,
        "referral_id": referral_id,
        "status": "rejected",
        "message": "Referral rejected.",
    })))
}

/// POST /api/v1/admin/referrals/:id/retry-payment — retry grant if previous attempt failed
pub async fn admin_retry_referral_payment(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(referral_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let info: Option<(String, Option<String>, i32)> = sqlx::query_as(
        "SELECT status, referrer_email, zaarcash_earned FROM referrals WHERE id = $1"
    )
    .bind(referral_id)
    .fetch_optional(&s.db)
    .await?;

    let (status, referrer_email, zaarcash) = info
        .ok_or(AppError::NotFound("Referral not found".to_string()))?;

    if status != "verified" {
        return Err(AppError::BadRequest(format!(
            "Only 'verified' referrals can retry payment. Current status: {}",
            status
        )));
    }

    if zaarcash <= 0 {
        return Err(AppError::BadRequest("No Zaarcash to grant".to_string()));
    }

    let email = referrer_email
        .ok_or(AppError::BadRequest("No referrer email on record".to_string()))?;

    match grant_zaarcash_via_is(&s, &email, zaarcash, "retry").await {
        Ok(_) => {
            sqlx::query("UPDATE referrals SET status = 'paid', updated_at = NOW() WHERE id = $1")
                .bind(referral_id)
                .execute(&s.db)
                .await?;

            Ok(Json(json!({
                "success": true,
                "referral_id": referral_id,
                "status": "paid",
                "message": format!("Payment retry succeeded. {} Zaarcash granted.", zaarcash),
            })))
        }
        Err(e) => Err(AppError::Internal(format!("Payment retry failed: {}", e))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Generate a unique 8-character alphanumeric referral code
async fn generate_unique_code(db: &sqlx::PgPool) -> Result<String, AppError> {
    let mut rng = rand::thread_rng();
    for _ in 0..10 {
        let code: String = (0..8)
            .map(|_| {
                let charset = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
                charset[rng.gen_range(0..charset.len())] as char
            })
            .collect();

        let exists: bool = sqlx::query_scalar::<_, Option<bool>>(
            "SELECT EXISTS(SELECT 1 FROM referrals WHERE referral_code = $1)"
        )
        .bind(&code)
        .fetch_one(db)
        .await?
        .unwrap_or(false);

        if !exists {
            return Ok(code);
        }
    }

    Err(AppError::Internal("Failed to generate unique referral code".to_string()))
}

/// Resolve email for the current user
async fn resolve_referrer_email(
    db: &sqlx::PgPool,
    role: &str,
    user_id: Uuid,
) -> Result<String, AppError> {
    match role {
        "visitor" => {
            sqlx::query_scalar::<_, String>("SELECT email FROM visitor_accounts WHERE id = $1")
                .bind(user_id)
                .fetch_optional(db)
                .await?
                .ok_or(AppError::NotFound("Visitor account not found".to_string()))
        }
        "business_owner" | "admin" | "super_admin" => {
            sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(db)
                .await?
                .ok_or(AppError::NotFound("User account not found".to_string()))
        }
        _ => sqlx::query_scalar::<_, String>("SELECT email FROM visitor_accounts WHERE id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?
            .ok_or(AppError::NotFound("Account not found".to_string())),
    }
}

/// Grant Zaarcash via IncentiveSwift's external grant-credits API
async fn grant_zaarcash_via_is(
    s: &AppState,
    email: &str,
    amount: i32,
    reason: &str,
) -> Result<(), String> {
    // Get system API key from config or env
    let api_key = std::env::var("IS_SYSTEM_API_KEY")
        .unwrap_or_else(|_| {
            // Try to look up from provider_keys
            "system_external_key".to_string()
        });

    // Check if IncentiveSwift is available
    let is_url = std::env::var("INCENTIVESWIFT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8083".to_string());

    let payload = json!({
        "email": email,
        "amount": amount,
        "reason": reason,
        "program": "zaarhub",
    });

    let resp = HTTP
        .post(format!("{}/api/v1/loyalty/external/grant-credits", is_url))
        .header("X-API-Key", &api_key)
        .header("Content-Type", "application/json")
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("IS grant-credits request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
        return Err(format!("IS grant-credits returned {}: {}", status, body));
    }

    Ok(())
}

/// Query IncentiveSwift for credit balance
async fn query_is_balance(
    s: &AppState,
    email: &str,
) -> Result<i32, AppError> {
    let is_url = std::env::var("INCENTIVESWIFT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8083".to_string());
    let api_key = std::env::var("IS_SYSTEM_API_KEY")
        .unwrap_or_else(|_| "system_external_key".to_string());

    let resp = HTTP
        .get(format!("{}/api/v1/credits/balance?email={}", is_url, email))
        .header("X-API-Key", &api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match resp {
        Ok(r) => {
            if r.status().is_success() {
                let body: Value = r.json().await.unwrap_or(json!({"balance": 0}));
                Ok(body.get("balance").and_then(|v| v.as_i64()).unwrap_or(0) as i32)
            } else {
                Ok(0)
            }
        }
        _ => Ok(0),
    }
}
