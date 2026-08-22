//! Connected Services — API key integration for IncentiveSwift and CoreSwift.
//!
//! Business owners can connect their IncentiveSwift and CoreSwift accounts
//! via API keys. Once connected, integration features appear in the listing editor.
//!
//! Uses proxy_common for IS communication and direct DB for MD storage.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

use super::proxy_common::*;

/// Status of a connected service for the current user.
#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Response listing all connected services for the user.
#[derive(Debug, Serialize)]
pub struct ConnectedServicesResponse {
    pub incentiveswift: ServiceStatus,
    pub coreswift: ServiceStatus,
}

/// Request to connect a service with an API key.
#[derive(Debug, Deserialize)]
pub struct ConnectServiceRequest {
    pub service: String,
    pub api_key: String,
}

/// Request to verify a service key.
#[derive(Debug, Deserialize)]
pub struct VerifyKeyRequest {
    pub service: String,
    pub api_key: String,
}

/// ── GET /api/v1/connected-services ──
/// Returns which services the user has API keys for.
pub async fn list_connected_services(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // Look up connected services in MD
    let row = sqlx::query_as::<_, (bool, Option<chrono::NaiveDateTime>)>(
        r#"SELECT is_active, expires_at
           FROM connected_services
           WHERE user_id = $1 AND service = 'incentiveswift'
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("DB error".into()))?;

    let is_connected = row.map(|(active, _)| active).unwrap_or(false);
    let is_expires = row.and_then(|(_, exp)| exp.map(|e| e.to_string()));

    // Check CoreSwift connection — coreswift is auto-connected if tenant_id exists
    let coreswift_connected = check_coreswift_connection_internal(&s, &claims)
        .await
        .unwrap_or(false);

    Ok(Json(json!({
        "incentiveswift": {
            "connected": is_connected,
            "expires_at": is_expires,
        },
        "coreswift": {
            "connected": coreswift_connected,
        }
    })))
}

/// ── POST /api/v1/connected-services/connect ──
/// Creates/verifies an API key for the specified service.
pub async fn connect_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<ConnectServiceRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    match body.service.as_str() {
        "incentiveswift" => {
            // Verify the API key by calling IS's verify endpoint
            let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;

            // Test the key by calling a lightweight IS endpoint
            let is_url = is_base_url();
            let url = format!("{}/api-keys/verify", is_url);

            let resp = http()
                .post(&url)
                .json(&json!({ "api_key": body.api_key }))
                .send()
                .await
                .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

            if !resp.status().is_success() {
                return Err(AppError::BadRequest(
                    "Invalid API key — verification failed".into(),
                ));
            }

            let v: Value = resp.json().await.unwrap_or_default();
            let valid = v.get("valid").and_then(|v| v.as_bool()).unwrap_or(false);

            if !valid {
                return Err(AppError::BadRequest("Invalid API key".into()));
            }

            // Store the connection in MD
            sqlx::query(
                r#"INSERT INTO connected_services (user_id, service, api_key_encrypted, is_active, created_at)
                   VALUES ($1, 'incentiveswift', $2, true, NOW())
                   ON CONFLICT (user_id, service)
                   DO UPDATE SET api_key_encrypted = $2, is_active = true, updated_at = NOW()"#
            )
            .bind(user_id)
            .bind(&body.api_key)
            .execute(&s.db)
            .await
            .map_err(|_| AppError::Internal("Failed to store API key".into()))?;

            Ok(Json(json!({
                "success": true,
                "service": "incentiveswift",
                "message": "Connected to IncentiveSwift successfully"
            })))
        }
        "coreswift" => {
            // CoreSwift is auto-provisioned per directory; just toggle the flag
            sqlx::query(
                r#"INSERT INTO connected_services (user_id, service, is_active, created_at)
                   VALUES ($1, 'coreswift', true, NOW())
                   ON CONFLICT (user_id, service)
                   DO UPDATE SET is_active = true, updated_at = NOW()"#,
            )
            .bind(user_id)
            .execute(&s.db)
            .await
            .map_err(|_| AppError::Internal("Failed to store CoreSwift connection".into()))?;

            Ok(Json(json!({
                "success": true,
                "service": "coreswift",
                "message": "Connected to CoreSwift CRM successfully"
            })))
        }
        _ => Err(AppError::BadRequest(format!(
            "Unknown service: {}",
            body.service
        ))),
    }
}

/// ── DELETE /api/v1/connected-services/:service ──
/// Disconnects a service by revoking/deactivating the stored key.
pub async fn disconnect_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(service): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let svc = service.to_lowercase();

    match svc.as_str() {
        "incentiveswift" => {
            // If we have an API key, try to revoke it on IS side
            let key: Option<String> = sqlx::query_scalar(
                "SELECT api_key_encrypted FROM connected_services WHERE user_id = $1 AND service = 'incentiveswift' AND is_active = true"
            )
            .bind(user_id)
            .fetch_optional(&s.db)
            .await
            .map_err(|_| AppError::Internal("DB error".into()))?;

            if let Some(api_key) = key {
                let is_url = is_base_url();
                let url = format!("{}/api-keys/revoke", is_url);
                // Best-effort revocation
                let _ = http()
                    .post(&url)
                    .json(&json!({ "api_key": api_key }))
                    .send()
                    .await;
            }

            // Deactivate in MD
            sqlx::query(
                "UPDATE connected_services SET is_active = false, updated_at = NOW() WHERE user_id = $1 AND service = 'incentiveswift'"
            )
            .bind(user_id)
            .execute(&s.db)
            .await
            .map_err(|_| AppError::Internal("DB error".into()))?;
        }
        "coreswift" => {
            sqlx::query(
                "UPDATE connected_services SET is_active = false, updated_at = NOW() WHERE user_id = $1 AND service = 'coreswift'"
            )
            .bind(user_id)
            .execute(&s.db)
            .await
            .map_err(|_| AppError::Internal("DB error".into()))?;
        }
        _ => return Err(AppError::BadRequest(format!("Unknown service: {}", svc))),
    }

    Ok(Json(json!({
        "success": true,
        "service": svc,
        "message": format!("Disconnected from {}", svc)
    })))
}

/// ── POST /api/v1/connected-services/verify ──
/// Tests whether an API key is valid for the specified service.
pub async fn verify_service_key(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<VerifyKeyRequest>,
) -> ApiResult<impl IntoResponse> {
    match body.service.as_str() {
        "incentiveswift" => {
            let is_url = is_base_url();
            let url = format!("{}/api-keys/verify", is_url);

            let resp = http()
                .post(&url)
                .json(&json!({ "api_key": body.api_key }))
                .send()
                .await
                .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

            let v: Value = resp.json().await.unwrap_or_default();
            let valid = v.get("valid").and_then(|v| v.as_bool()).unwrap_or(false);

            Ok(Json(json!({
                "service": "incentiveswift",
                "valid": valid,
            })))
        }
        "coreswift" => {
            // For CoreSwift, check if there's a tenant connection
            let connected = check_coreswift_connection_internal(&s, &claims)
                .await
                .unwrap_or(false);
            Ok(Json(json!({
                "service": "coreswift",
                "valid": connected,
            })))
        }
        _ => Err(AppError::BadRequest(format!(
            "Unknown service: {}",
            body.service
        ))),
    }
}

/// ── GET /api/v1/connected-services/:service/campaigns ──
/// Fetches the user's campaigns from IncentiveSwift (requires connected key).
pub async fn list_service_campaigns(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(service): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    match service.to_lowercase().as_str() {
        "incentiveswift" => {
            // Verify the user has an active connection
            let connected: bool = sqlx::query_scalar(
                "SELECT is_active FROM connected_services WHERE user_id = $1 AND service = 'incentiveswift' LIMIT 1"
            )
            .bind(user_id)
            .fetch_optional(&s.db)
            .await
            .map_err(|_| AppError::Internal("DB error".into()))?
            .unwrap_or(false);

            if !connected {
                return Err(AppError::Unauthorized);
            }

            // Proxy the request to IS
            let (aid, email) = resolve_is_account(&s.db, &s.is_db, &claims).await?;
            let result = proxy_get("/campaigns", &aid, &email, &claims.role).await?;
            Ok(Json(result))
        }
        _ => Err(AppError::BadRequest(format!(
            "Unknown service: {}",
            service
        ))),
    }
}

/// Internal helper: check if CoreSwift is connected for this user/directory.
async fn check_coreswift_connection_internal(
    s: &AppState,
    claims: &Claims,
) -> Result<bool, AppError> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // Check if there's a coreswift entry in connected_services
    let active: Option<bool> = sqlx::query_scalar(
        "SELECT is_active FROM connected_services WHERE user_id = $1 AND service = 'coreswift' LIMIT 1"
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("DB error".into()))?;

    Ok(active.unwrap_or(false))
}

/// ── GET /api/v1/connected-services/coreswift/check ──
/// Checks whether CoreSwift is connected for the current user/directory.
pub async fn check_coreswift_connection(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let connected = check_coreswift_connection_internal(&s, &claims).await?;
    Ok(Json(json!({
        "service": "coreswift",
        "connected": connected,
    })))
}
