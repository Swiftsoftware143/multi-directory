//! Connected Services — account integration for CoreSwift CRM.
//!
//! A business owner connects their CoreSwift account so CoreSwift's integration
//! features appear in the listing editor.
//!
//! The "Connect IncentiveSwift" flow was RETIRED on David's decision (2026-09-23):
//! loyalty is native Multi-Directory code (ZaarCash) and CoreSwift is the ONLY
//! external integration. There is no IncentiveSwift API key to paste, no IS verify
//! call, no IS key row and no IS campaigns proxy in this app any more — the
//! `incentiveswift` service value is refused here and by the database CHECK
//! constraint (migration 110).

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// Request to connect a service.
#[derive(Debug, Deserialize)]
pub struct ConnectServiceRequest {
    pub service: String,
    /// Retained for wire compatibility with the shipped portals, which still send
    /// an (ignored) `api_key`. CoreSwift needs no key.
    #[serde(default)]
    pub api_key: String,
}

/// Request to verify a service.
#[derive(Debug, Deserialize)]
pub struct VerifyKeyRequest {
    pub service: String,
    #[serde(default)]
    pub api_key: String,
}

/// The one message every retired-IncentiveSwift path returns.
const INCENTIVESWIFT_RETIRED: &str = "The IncentiveSwift connection was retired. \
Loyalty is built into Multi-Directory (ZaarCash) and CoreSwift CRM is the only \
external integration this directory connects to.";

/// ── GET /api/v1/connected-services ──
/// Returns which services the signed-in user has connected. CoreSwift is the only one.
pub async fn list_connected_services(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let coreswift_connected = check_coreswift_connection_internal(&s, &claims)
        .await
        .unwrap_or(false);

    Ok(Json(json!({
        "coreswift": {
            "connected": coreswift_connected,
        }
    })))
}

/// ── POST /api/v1/connected-services/connect ──
/// Connects a service. CoreSwift is auto-provisioned per directory; this flips the flag.
pub async fn connect_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<ConnectServiceRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    match body.service.as_str() {
        "coreswift" => {
            // CoreSwift is auto-provisioned per directory; just toggle the flag.
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
        "incentiveswift" => Err(AppError::BadRequest(INCENTIVESWIFT_RETIRED.into())),
        _ => Err(AppError::BadRequest(format!(
            "Unknown service: {}",
            body.service
        ))),
    }
}

/// ── DELETE /api/v1/connected-services/:service ──
/// Disconnects a service by deactivating the stored connection.
pub async fn disconnect_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(service): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let svc = service.to_lowercase();

    match svc.as_str() {
        "coreswift" => {
            sqlx::query(
                "UPDATE connected_services SET is_active = false, updated_at = NOW() WHERE user_id = $1 AND service = 'coreswift'"
            )
            .bind(user_id)
            .execute(&s.db)
            .await
            .map_err(|_| AppError::Internal("DB error".into()))?;
        }
        "incentiveswift" => return Err(AppError::BadRequest(INCENTIVESWIFT_RETIRED.into())),
        _ => return Err(AppError::BadRequest(format!("Unknown service: {}", svc))),
    }

    Ok(Json(json!({
        "success": true,
        "service": svc,
        "message": format!("Disconnected from {}", svc)
    })))
}

/// ── POST /api/v1/connected-services/verify ──
/// Tests whether a service is connected for the caller.
///
/// The IncentiveSwift arm was retired with the connect flow (2026-09-23); only
/// CoreSwift remains.
pub async fn verify_service_key(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<VerifyKeyRequest>,
) -> ApiResult<impl IntoResponse> {
    match body.service.as_str() {
        "coreswift" => {
            let connected = check_coreswift_connection_internal(&s, &claims)
                .await
                .unwrap_or(false);
            Ok(Json(json!({
                "service": "coreswift",
                "valid": connected,
            })))
        }
        "incentiveswift" => Err(AppError::BadRequest(INCENTIVESWIFT_RETIRED.into())),
        _ => Err(AppError::BadRequest(format!(
            "Unknown service: {}",
            body.service
        ))),
    }
}

/// Internal helper: check if CoreSwift is connected for this user/directory.
async fn check_coreswift_connection_internal(
    s: &AppState,
    claims: &Claims,
) -> Result<bool, AppError> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

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
