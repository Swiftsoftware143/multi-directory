//! Integration Center — canonical spoke endpoints for the native CoreSwift integration.
//!
//! Fleet standard: /opt/swift/docs/integration-center-standard-2026-09-20.md
//!   * R2 — every app has a NATIVE CoreSwift integration and the data flows
//!     DOWNWARD into CoreSwift. MultiDirectory captures the lead; CoreSwift is the
//!     hub and the single home for it.
//!
//! Canonical paths (identical in every spoke):
//!   GET  /api/v1/integrations/coreswift/status  -> {"connected": bool, "base_url": "…"}
//!   GET  /api/v1/integrations/coreswift/lists   -> proxy hub GET  /api/external/lists
//!   POST /api/v1/integrations/coreswift/push    -> proxy hub POST /api/external/contacts
//!                                                  (the manual fallback; the app also
//!                                                  fires it from its own capture events)
//!
//! Credentials are BYOK and tenant-level: they live in `provider_keys` for provider
//! `coreswift` (written through the app's existing BYOK surface, `POST
//! /api/v1/provider-keys`). Nothing here reads a key from the environment, and a GET
//! never returns the raw key — only `mask_key()` output.
//!
//! Every hop below delegates to [`crate::coreswift`]: one resolver, one HTTP client,
//! no second CoreSwift client.

use axum::{
    extract::{Extension, Path, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::coreswift::{hub_lists, push_lead_to_coreswift, resolve_lead_conn, LeadPayload};
use crate::error::{ApiResult, AppError};
use crate::AppState;

use super::provider_keys_handler::mask_key;

/// GET /api/v1/integrations/coreswift/status
pub async fn coreswift_status(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    match resolve_lead_conn(&s.db, Some(tenant_id), None).await {
        Ok(Some(conn)) => Ok(Json(json!({
            "provider": "coreswift",
            "connected": true,
            "base_url": conn.base_url,
            "key_preview": mask_key(&conn.api_key),
            "lists": {
                "users": conn.users_list_id,
                "businesses": conn.businesses_list_id,
                "suppliers": conn.suppliers_list_id,
            }
        }))),
        Ok(None) => Ok(Json(json!({
            "provider": "coreswift",
            "connected": false,
            "base_url": Value::Null,
            "message": "No CoreSwift key stored for this tenant — capture continues locally.",
        }))),
        Err(e) => Err(AppError::Internal(e)),
    }
}

/// GET /api/v1/integrations/coreswift/lists — proxy the hub's list picker.
pub async fn coreswift_lists(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let conn = resolve_lead_conn(&s.db, Some(tenant_id), None)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| {
            AppError::BadRequest(
                "CoreSwift is not connected — store a csk_ key from CoreSwift's Integration Center first."
                    .to_string(),
            )
        })?;

    let lists = hub_lists(&conn).await.map_err(AppError::Internal)?;
    Ok(Json(json!({ "success": true, "data": lists })))
}

/// Body for the manual push / the shared capture helper's HTTP shape.
#[derive(Debug, Deserialize, Default)]
pub struct LeadPushRequest {
    pub directory_id: Option<Uuid>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub title: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub address_line1: Option<String>,
    pub notes: Option<String>,
    pub list_id: Option<Uuid>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

impl From<LeadPushRequest> for LeadPayload {
    fn from(r: LeadPushRequest) -> Self {
        LeadPayload {
            email: r.email,
            phone: r.phone,
            name: r.name,
            first_name: r.first_name,
            last_name: r.last_name,
            company: r.company,
            title: r.title,
            city: r.city,
            state: r.state,
            postal_code: r.postal_code,
            address_line1: r.address_line1,
            notes: r.notes,
            list_id: r.list_id,
            tags: r.tags,
            fields: r.fields,
        }
    }
}

/// POST /api/v1/integrations/coreswift/push — the manual fallback ("push now").
pub async fn coreswift_push(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Json(req): Json<LeadPushRequest>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;
    let directory_id = req.directory_id;

    // The manual push is user-initiated, so "not connected" is an error the UI can
    // act on (unlike the capture path, which degrades quietly).
    if resolve_lead_conn(&s.db, Some(tenant_id), directory_id)
        .await
        .map_err(AppError::Internal)?
        .is_none()
    {
        return Err(AppError::BadRequest(
            "CoreSwift is not connected — store a csk_ key from CoreSwift's Integration Center first."
                .to_string(),
        ));
    }

    let payload: LeadPayload = req.into();
    if payload.email.is_none() && payload.phone.is_none() && payload.name.is_none() {
        return Err(AppError::Validation(
            "A lead needs at least one of email, phone or name.".to_string(),
        ));
    }

    let pushed = push_lead_to_coreswift(&s.db, Some(tenant_id), directory_id, payload)
        .await
        .map_err(AppError::Internal)?;

    Ok(Json(json!({
        "success": pushed,
        "pushed": pushed,
        "message": if pushed {
            "Lead pushed to CoreSwift."
        } else {
            "Nothing pushed."
        },
    })))
}

/// POST /api/v1/provider-keys/:provider/test — a REAL probe, not a stored-flag read.
///
/// For `coreswift` this hits the hub (`GET /api/external/lists`) with the resolved
/// key, so the answer is the hub's own: a bad/revoked key reports `valid: false`
/// instead of "configured: true".
pub async fn test_provider_key_live(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;
    let provider = provider.to_lowercase();

    if provider != "coreswift" {
        // Other providers have no live probe wired here; report honestly that the
        // key is stored without claiming it was validated against the vendor.
        let key = super::provider_keys_handler::resolve_provider_key_for_tenant(
            &s.db, tenant_id, &provider,
        )
        .await
        .ok_or_else(|| {
            AppError::NotFound(format!("No API key found for provider '{}'", provider))
        })?;
        return Ok(Json(json!({
            "provider": provider,
            "configured": true,
            "valid": Value::Null,
            "probed": false,
            "key_preview": mask_key(&key),
            "message": format!("{} key is stored (no live probe for this provider).", provider),
        })));
    }

    let conn = resolve_lead_conn(&s.db, Some(tenant_id), None)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| {
            AppError::NotFound(
                "No CoreSwift key stored for this tenant — connect first.".to_string(),
            )
        })?;

    let key_preview = mask_key(&conn.api_key);
    match hub_lists(&conn).await {
        Ok(lists) => {
            let count = lists
                .get("lists")
                .and_then(|l| l.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Ok(Json(json!({
                "provider": "coreswift",
                "configured": true,
                "valid": true,
                "base_url": conn.base_url,
                "key_preview": key_preview,
                "lists": count,
                "message": format!("CoreSwift reachable — {count} list(s) available."),
            })))
        }
        Err(e) => Ok(Json(json!({
            "provider": "coreswift",
            "configured": true,
            "valid": false,
            "base_url": conn.base_url,
            "key_preview": key_preview,
            "message": e,
        }))),
    }
}
