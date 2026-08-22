//! Shared proxy utilities for forwarding MultiDirectory API requests
//! to IncentiveSwift backend services (loyalty, IQS, badges, etc.).
//!
//! Used by loyalty_proxy and iqs_proxy modules.

use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::Value;

use crate::auth::models::Claims;
use crate::error::AppError;

/// Return the IncentiveSwift base URL from environment, with a sensible default.
pub(crate) fn is_base_url() -> String {
    std::env::var("INCENTIVESWIFT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8083/api/v1".to_string())
}

/// Create a short-lived reqwest HTTP client.
pub(crate) fn http() -> reqwest::Client {
    reqwest::Client::new()
}

/// Generate an IS-compatible JWT using jsonwebtoken.
pub(crate) fn make_is_jwt(
    account_id: &str,
    email: &str,
    role: &str,
    secret: &str,
) -> Result<String, AppError> {
    use std::collections::HashMap;
    let now = chrono::Utc::now().timestamp() as usize;
    let mut claims = HashMap::new();
    claims.insert("sub", serde_json::Value::String(account_id.to_string()));
    claims.insert("email", serde_json::Value::String(email.to_string()));
    claims.insert("role", serde_json::Value::String(role.to_string()));
    claims.insert("iat", serde_json::Value::Number(now.into()));
    claims.insert("exp", serde_json::Value::Number((now + 300).into()));

    let header = Header::new(jsonwebtoken::Algorithm::HS256);
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("JWT encode failed: {}", e)))
}

/// Look up the IS account_id by email from MD's user table.
pub(crate) async fn resolve_is_account(
    db: &sqlx::PgPool,
    is_db: &sqlx::PgPool,
    md_claims: &Claims,
) -> Result<(String, String), AppError> {
    let user_id = uuid::Uuid::parse_str(&md_claims.sub).map_err(|_| AppError::Unauthorized)?;

    // Get email from MD users table
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .map_err(|_| AppError::Internal("DB lookup failed".into()))?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    // Look up IS account_id by email
    let is_account: Option<String> =
        sqlx::query_scalar("SELECT id::text FROM accounts WHERE email = $1 LIMIT 1")
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

/// Resolve the IS JWT secret from env.
fn is_jwt_secret() -> String {
    std::env::var("IS_JWT_SECRET").unwrap_or_else(|_| {
        "rr0NC13QNMpmvuopQjOZFqQKxtq1JosBr/i/mZ+QyrHwryQzaVzWKA1htAEBN9WI".to_string()
    })
}

/// Proxy a GET request to IncentiveSwift.
pub(crate) async fn proxy_get(
    path: &str,
    account_id: &str,
    email: &str,
    role: &str,
) -> Result<Value, AppError> {
    let secret = is_jwt_secret();
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", is_base_url(), path);

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

/// Proxy a POST request to IncentiveSwift.
pub(crate) async fn proxy_post(
    path: &str,
    body: &Value,
    account_id: &str,
    email: &str,
    role: &str,
) -> Result<Value, AppError> {
    let secret = is_jwt_secret();
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", is_base_url(), path);

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

/// Proxy a PUT request to IncentiveSwift.
pub(crate) async fn proxy_put(
    path: &str,
    body: &Value,
    account_id: &str,
    email: &str,
    role: &str,
) -> Result<Value, AppError> {
    let secret = is_jwt_secret();
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", is_base_url(), path);

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

/// Proxy a DELETE request to IncentiveSwift.
pub(crate) async fn proxy_delete(
    path: &str,
    account_id: &str,
    email: &str,
    role: &str,
) -> Result<Value, AppError> {
    let secret = is_jwt_secret();
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", is_base_url(), path);

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

/// Proxy a PATCH request to IncentiveSwift.
pub(crate) async fn proxy_patch(
    path: &str,
    body: &Value,
    account_id: &str,
    email: &str,
    role: &str,
) -> Result<Value, AppError> {
    let secret = is_jwt_secret();
    let token = make_is_jwt(account_id, email, role, &secret)?;
    let url = format!("{}{}", is_base_url(), path);

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
