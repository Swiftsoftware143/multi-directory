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
use crate::security::provider_key_crypto as keycrypto;
use crate::AppState;

use super::proxy_common::*;

/// IncentiveSwift's answer to `POST /api-keys/verify`.
struct IsKeyCheck {
    /// The HTTP call itself succeeded (2xx).
    http_ok: bool,
    /// IncentiveSwift says the key is live.
    valid: bool,
    /// The IncentiveSwift account the key belongs to. EMPTY when the key is not valid or when
    /// IncentiveSwift did not report an owner (older build) — callers must treat empty as
    /// "ownership unknown", never as a match.
    account_id: String,
}

/// Ask IncentiveSwift whether an API key is live, and which account it belongs to.
///
/// This is the ONE place Multi-Directory reads IncentiveSwift's verify answer, so
/// `connect_service` and `verify_service_key` cannot drift apart about who owns a key.
async fn verify_is_api_key(api_key: &str) -> Result<IsKeyCheck, AppError> {
    let url = format!("{}/api-keys/verify", is_base_url());

    let resp = http()
        .post(&url)
        .json(&json!({ "api_key": api_key }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("IS request failed: {}", e)))?;

    let http_ok = resp.status().is_success();
    let v: Value = resp.json().await.unwrap_or_default();

    Ok(IsKeyCheck {
        http_ok,
        valid: v.get("valid").and_then(|v| v.as_bool()).unwrap_or(false),
        account_id: v
            .get("account_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

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
    let row = sqlx::query_as::<_, (bool, Option<chrono::DateTime<chrono::Utc>>)>(
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
            // Who is THIS directory's IncentiveSwift account? Resolved from the signed-in
            // user's email. `None` means no IncentiveSwift account is registered with that
            // address, so there is nothing a pasted key could legitimately belong to.
            let (email, directory_is_account) =
                resolve_is_account_owner(&s.db, &s.is_db, &claims).await?;

            // Is the pasted key a LIVE IncentiveSwift key, and whose is it?
            let check = verify_is_api_key(&body.api_key).await?;

            if !check.http_ok {
                return Err(AppError::BadRequest(
                    "Invalid API key — verification failed".into(),
                ));
            }

            if !check.valid {
                return Err(AppError::BadRequest("Invalid API key".into()));
            }

            // ── BIND the key to this directory's IncentiveSwift account ──────────────
            // `valid` only proves "this string is SOMEBODY'S IncentiveSwift key": an
            // IncentiveSwift API key is a SHARED customer credential — IS's own API-Keys
            // screen hands it out to be pasted into other surfaces — so without the
            // comparison below ANY valid key, a stranger's included, could arm this
            // directory's connection. The connection has to be a LINK, not a liveness gate:
            // the key must have been issued by the same IncentiveSwift account this
            // directory shows campaigns from (list_service_campaigns proxies with this
            // directory's own IS account, resolved by the same email).
            if check.account_id.is_empty() {
                return Err(AppError::BadRequest(
                    "IncentiveSwift did not say which account this API key belongs to, so it cannot be linked to this directory.".into(),
                ));
            }

            match directory_is_account.as_deref() {
                Some(owner) if owner == check.account_id => {}
                Some(_) => {
                    return Err(AppError::BadRequest(format!(
                        "This API key belongs to a different IncentiveSwift account. Paste a key issued by the IncentiveSwift account registered to {} — that is the account this directory shows campaigns from.",
                        email
                    )))
                }
                None => {
                    return Err(AppError::BadRequest(format!(
                        "No IncentiveSwift account is registered with {}. Sign in to Multi-Directory with the email your IncentiveSwift account uses (or create that IncentiveSwift account first), then paste its API key.",
                        email
                    )))
                }
            }

            // The IncentiveSwift API key is a CUSTOMER credential (it can revoke campaigns and touch
            // the account's data), so it is encrypted BEFORE it reaches the database through the
            // single choke point in src/security/provider_key_crypto.rs: enc:v1 + AES-256 under
            // PROVIDER_KEY_ENC_SECRET, which lives only in the process environment. Fail-closed — a
            // missing master key errors here instead of storing the value the user typed, and
            // migration 096's CHECK constraint refuses a plaintext write at the database.
            let stored_api_key = keycrypto::encrypt_for_storage(&s.db, &body.api_key).await?;

            // Store the connection in MD
            sqlx::query(
                r#"INSERT INTO connected_services (user_id, service, api_key_encrypted, is_active, created_at)
                   VALUES ($1, 'incentiveswift', $2, true, NOW())
                   ON CONFLICT (user_id, service)
                   DO UPDATE SET api_key_encrypted = $2, is_active = true, updated_at = NOW()"#
            )
            .bind(user_id)
            .bind(&stored_api_key)
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
            // NO remote revoke here, deliberately — and no read of the stored key either.
            //
            // connected_services.api_key_encrypted holds a SHARED customer credential:
            // IncentiveSwift's own API-Keys screen hands the key out so the customer can paste
            // it into several surfaces, so one consumer disconnecting must NEVER deactivate a
            // key the others depend on. IncentiveSwift has no /api-keys/revoke route and is not
            // going to get one (t_09e43d1a), so the old best-effort POST there could only ever
            // 404 while putting the DECRYPTED key on the wire for nothing. Both are gone.
            //
            // Disconnect therefore only flips Multi-Directory's own flag: the campaigns view
            // disappears here, and the key keeps working everywhere else the customer pasted
            // it. The stored ciphertext is left untouched — it stays the record of which key was
            // linked, encrypted at rest, and is no longer read by any code path.
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
///
/// For IncentiveSwift "valid for this service" means the key is live AND belongs to this
/// directory's IncentiveSwift account: a key that is merely somebody's key would be refused
/// by `connect_service`, so answering a bare `valid: true` here would send the UI down a path
/// the connect endpoint then rejects. `key_valid`/`owned` are reported separately so the caller
/// can tell "not a key at all" from "a valid key that is not ours".
pub async fn verify_service_key(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<VerifyKeyRequest>,
) -> ApiResult<impl IntoResponse> {
    match body.service.as_str() {
        "incentiveswift" => {
            let (_email, directory_is_account) =
                resolve_is_account_owner(&s.db, &s.is_db, &claims).await?;

            let check = verify_is_api_key(&body.api_key).await?;

            let owned = match (directory_is_account.as_deref(), check.account_id.as_str()) {
                (Some(owner), key_owner) if !key_owner.is_empty() => owner == key_owner,
                _ => false,
            };

            Ok(Json(json!({
                "service": "incentiveswift",
                "valid": check.valid && owned,
                "key_valid": check.valid,
                "owned": owned,
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
