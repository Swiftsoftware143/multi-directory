//! SSO Role Switcher — unified token system for role switching.
//!
//! Allows a user with accounts in multiple roles (visitor, business_owner, admin)
//! to switch between them seamlessly via a single endpoint.
//!
//! Accounts are linked by email in the `account_links` table.

use axum::{
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use chrono::Utc;

use crate::AppState;
use crate::auth::models::Claims;
use crate::auth::middleware::{create_token, verify_token, is_admin, is_business_owner, is_visitor};
use crate::error::{AppError, ApiResult};

#[derive(Debug, Deserialize)]
pub struct SwitchRoleRequest {
    pub target_role: String, // "visitor" | "business" | "admin"
}

#[derive(Debug, Serialize)]
pub struct SwitchRoleResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub role: String,
    pub email: String,
    pub switch_back_token: Option<String>,
    pub linked_accounts: Vec<LinkedAccountInfo>,
}

#[derive(Debug, Serialize)]
pub struct LinkedAccountInfo {
    pub role: String,
    pub available: bool,
    pub label: String,
}

/// POST /api/v1/auth/switch-role
///
/// Takes a valid JWT (any role), looks up linked accounts by email,
/// and returns a new JWT for the target role.
///
/// Body: { "target_role": "visitor" | "business" | "admin" }
pub async fn switch_role(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SwitchRoleRequest>,
) -> ApiResult<impl IntoResponse> {
    // Extract and verify JWT manually (route is outside auth_guard)
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let email = claims.aud.clone()
        .unwrap_or_else(|| "unknown@example.com".to_string());

    // The email claim stores the user's email — let's look it up properly
    let current_user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let current_role = &claims.role;

    // Resolve email from the appropriate table based on current role
    let email = resolve_email(&s.db, current_role, current_user_id).await?;

    // Validate target role
    let valid_roles = ["visitor", "business", "admin"];
    if !valid_roles.contains(&req.target_role.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid target_role '{}'. Valid: visitor, business, admin",
            req.target_role
        )));
    }

    // If target is the same as current, return the same token
    if req.target_role == *current_role {
        return Ok(Json(json!(SwitchRoleResponse {
            access_token: "same_session".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: 0,
            role: current_role.clone(),
            email: email.clone(),
            switch_back_token: None,
            linked_accounts: get_linked_account_info(&s.db, &email).await?,
        })));
    }

    // Look for linked accounts by email
    let target_id = find_target_account(&s.db, &email, &req.target_role).await?;

    let (target_id_str, target_email) = match target_id {
        Some(id) => (id, email.clone()),
        None => {
            return Err(AppError::NotFound(format!(
                "No {} account linked to this email. Please create a {} account first.",
                req.target_role, req.target_role
            )));
        }
    };

    // Create switch-back token (allows restoring the current role)
    let switch_back_token = create_role_token(
        &s.config.jwt_secret,
        &current_user_id.to_string(),
        current_role,
        &email,
        s.config.jwt_access_expiry,
    )?;

    // Create the new token for the target role
    let new_token = create_role_token(
        &s.config.jwt_secret,
        &target_id_str,
        &req.target_role,
        &target_email,
        s.config.jwt_access_expiry,
    )?;

    Ok(Json(json!(SwitchRoleResponse {
        access_token: new_token,
        token_type: "Bearer".to_string(),
        expires_in: s.config.jwt_access_expiry,
        role: req.target_role,
        email: target_email,
        switch_back_token: Some(switch_back_token),
        linked_accounts: get_linked_account_info(&s.db, &email).await?,
    })))
}

/// GET /api/v1/auth/linked-accounts — get info about linked accounts for the current user
pub async fn get_linked_accounts(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let claims = extract_claims_from_headers(&headers, &s.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;
    let role = &claims.role;

    let email = resolve_email(&s.db, role, user_id).await?;
    let linked_accounts = get_linked_account_info(&s.db, &email).await?;

    Ok(Json(json!({
        "success": true,
        "email": email,
        "current_role": role,
        "linked_accounts": linked_accounts,
    })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Internal Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Resolve email for the current user based on their role
async fn resolve_email(
    db: &sqlx::PgPool,
    role: &str,
    user_id: Uuid,
) -> Result<String, AppError> {
    match role {
        "visitor" => {
            sqlx::query_scalar::<_, String>(
                "SELECT email FROM visitor_accounts WHERE id = $1"
            )
            .bind(user_id)
            .fetch_optional(db)
            .await?
            .ok_or(AppError::NotFound("Visitor account not found".to_string()))
        }
        "business_owner" | "admin" => {
            sqlx::query_scalar::<_, String>(
                "SELECT email FROM users WHERE id = $1"
            )
            .bind(user_id)
            .fetch_optional(db)
            .await?
            .ok_or(AppError::NotFound("User account not found".to_string()))
        }
        _ => Err(AppError::BadRequest(format!("Unknown role: {}", role))),
    }
}

/// Find a target account by email and role
async fn find_target_account(
    db: &sqlx::PgPool,
    email: &str,
    target_role: &str,
) -> Result<Option<String>, AppError> {
    match target_role {
        "visitor" => {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM visitor_accounts WHERE email = $1"
            )
            .bind(email)
            .fetch_optional(db)
            .await?;
            Ok(id.map(|u| u.to_string()))
        }
        "business" => {
            // Business owners are users with claimed businesses
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT u.id FROM users u
                 JOIN claimed_businesses cb ON cb.user_id = u.id
                 WHERE u.email = $1 AND cb.is_active = true
                 LIMIT 1"
            )
            .bind(email)
            .fetch_optional(db)
            .await?;
            Ok(id.map(|u| u.to_string()))
        }
        "admin" => {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM users WHERE email = $1 AND (role = 'admin' OR role = 'super_admin')"
            )
            .bind(email)
            .fetch_optional(db)
            .await?;
            Ok(id.map(|u| u.to_string()))
        }
        _ => Ok(None),
    }
}

/// Create a JWT token for a given role
fn create_role_token(
    jwt_secret: &str,
    user_id: &str,
    role: &str,
    email: &str,
    expiry_secs: i64,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp() as usize;

    // Map role names to canonical claim role values
    let canonical_role = match role {
        "business" => "business_owner",
        "visitor" => "visitor",
        "admin" => "admin",
        other => other,
    };

    // Map role to tenant ID — for visitors we use a shared tenant
    let tid = match canonical_role {
        "visitor" => "00000000-0000-0000-0000-000000000001",
        "business_owner" => {
            // For business owners, try to find their tenant
            // Default to the email-based lookup
            "00000000-0000-0000-0000-000000000001"
        }
        "admin" => "00000000-0000-0000-0000-000000000000",
        _ => "00000000-0000-0000-0000-000000000001",
    };

    let claims = Claims {
        sub: user_id.to_string(),
        tid: tid.to_string(),
        role: canonical_role.to_string(),
        exp: now + expiry_secs as usize,
        iat: now,
        aud: Some(email.to_string()),
        iss: Some("multidirectory".to_string()),
    };

    create_token(&claims, jwt_secret)
        .map_err(|e| AppError::Internal(format!("Token creation failed: {}", e)))
}

/// Get info about all linked accounts for a given email
async fn get_linked_account_info(
    db: &sqlx::PgPool,
    email: &str,
) -> Result<Vec<LinkedAccountInfo>, AppError> {
    let mut accounts = Vec::new();

    // Check visitor account
    let has_visitor: bool = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT EXISTS(SELECT 1 FROM visitor_accounts WHERE email = $1)"
    )
    .bind(email)
    .fetch_one(db)
    .await?
    .unwrap_or(false);

    accounts.push(LinkedAccountInfo {
        role: "visitor".to_string(),
        available: has_visitor,
        label: if has_visitor { "Visitor Portal" } else { "Visitor Portal (not linked)" }.to_string(),
    });

    // Check business owner account
    let has_business: bool = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT EXISTS(SELECT 1 FROM users u
         JOIN claimed_businesses cb ON cb.user_id = u.id
         WHERE u.email = $1 AND cb.is_active = true)"
    )
    .bind(email)
    .fetch_one(db)
    .await?
    .unwrap_or(false);

    accounts.push(LinkedAccountInfo {
        role: "business".to_string(),
        available: has_business,
        label: if has_business { "Business Dashboard" } else { "Business Dashboard (not linked)" }.to_string(),
    });

    // Check admin account
    let has_admin: bool = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1 AND (role = 'admin' OR role = 'super_admin'))"
    )
    .bind(email)
    .fetch_one(db)
    .await?
    .unwrap_or(false);

    accounts.push(LinkedAccountInfo {
        role: "admin".to_string(),
        available: has_admin,
        label: if has_admin { "Admin Panel" } else { "Admin Panel (not linked)" }.to_string(),
    });

    Ok(accounts)
}

/// Extract claims from Authorization header manually (for routes outside auth_guard)
fn extract_claims_from_headers(
    headers: &HeaderMap,
    jwt_secret: &str,
) -> Result<Claims, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)
}
