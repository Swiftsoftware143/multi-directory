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
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::coreswift::{
    clear_link, conn_for_link, coreswift_url, hub_lists, link_status, probe_conn,
    push_lead_to_coreswift, resolve_lead_conn, save_link, CoreSwiftConn, LeadPayload, LinkScope,
};
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

    let contact_id = push_lead_to_coreswift(&s.db, Some(tenant_id), directory_id, payload)
        .await
        .map_err(AppError::Internal)?;

    Ok(Json(json!({
        "success": contact_id.is_some(),
        "pushed": contact_id.is_some(),
        "coreswift_contact_id": contact_id,
        "message": if contact_id.is_some() {
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

// ─────────────────────────────────────────────────────────────────────────────
// Card B67 — the NATIVE per-network / per-directory CoreSwift connection.
//
// Operable entirely from the Admin Panel: connect, test, see the status, choose the lists,
// disconnect. The link (tenant id, base URL, optional `csk_` personal key, the six typed
// list ids) lives on the `networks` / `directories` row for that account — nothing
// server-wide, nothing hardcoded, nothing server-provisioned behind the operator's back.
//
// Every route here is PLATFORM-OPERATOR-only: these write credentials and move a tenant's
// captures into (or out of) another account.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LinkQuery {
    pub scope: String,
    pub id: Uuid,
}

impl LinkQuery {
    fn scope(&self) -> Result<LinkScope, AppError> {
        LinkScope::parse(&self.scope).ok_or_else(|| {
            AppError::Validation("scope must be 'network' or 'directory'".to_string())
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct LinkSaveRequest {
    pub scope: String,
    pub id: Uuid,
    pub tenant_id: Uuid,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub personal_key: Option<String>,
    #[serde(default)]
    pub lists: Value,
}

/// GET /api/v1/integrations/coreswift/connection/targets — the picker's own list: every
/// network and every directory with its current link state. Operator-only.
pub async fn coreswift_connection_targets(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let networks = sqlx::query_as::<_, (Uuid, String, String, Option<Uuid>, bool)>(
        "SELECT id, name, slug, coreswift_tenant_id, \
                (coreswift_personal_key_encrypted IS NOT NULL \
                 AND length(coreswift_personal_key_encrypted) > 0) \
         FROM networks ORDER BY name",
    )
    .fetch_all(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("DB error listing networks: {e}")))?;

    let directories =
        sqlx::query_as::<_, (Uuid, String, String, Option<Uuid>, Option<Uuid>, bool)>(
            "SELECT id, name, slug, network_id, coreswift_tenant_id, \
                (coreswift_personal_key_encrypted IS NOT NULL \
                 AND length(coreswift_personal_key_encrypted) > 0) \
         FROM directories ORDER BY name",
        )
        .fetch_all(&s.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error listing directories: {e}")))?;

    Ok(Json(json!({
        "success": true,
        "networks": networks.into_iter().map(|(id, name, slug, tenant, key)| json!({
            "id": id, "name": name, "slug": slug, "tenant_id": tenant, "key_configured": key,
        })).collect::<Vec<_>>(),
        "directories": directories.into_iter().map(|(id, name, slug, network_id, tenant, key)| json!({
            "id": id, "name": name, "slug": slug, "network_id": network_id,
            "tenant_id": tenant, "key_configured": key,
        })).collect::<Vec<_>>(),
    })))
}

/// GET /api/v1/integrations/coreswift/connection?scope=network|directory&id=<uuid>
pub async fn coreswift_connection_get(
    State(s): State<AppState>,
    Query(q): Query<LinkQuery>,
) -> ApiResult<impl IntoResponse> {
    let scope = q.scope()?;
    let connection = link_status(&s.db, scope, q.id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound(format!("No {} with id {}", scope.as_str(), q.id)))?;
    Ok(Json(json!({ "success": true, "connection": connection })))
}

/// POST /api/v1/integrations/coreswift/connection — connect or update a link.
///
/// The connection is PROBED against the hub before it is stored, so the panel can never
/// report "connected" for a tenant the CRM will reject on the next push.
pub async fn coreswift_connection_save(
    State(s): State<AppState>,
    Json(req): Json<LinkSaveRequest>,
) -> ApiResult<impl IntoResponse> {
    let scope = LinkScope::parse(&req.scope).ok_or_else(|| {
        AppError::Validation("scope must be 'network' or 'directory'".to_string())
    })?;

    let candidate = CoreSwiftConn {
        tenant_id: req.tenant_id,
        api_key: req
            .personal_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .unwrap_or("")
            .to_string(),
        base_url: req
            .base_url
            .clone()
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(coreswift_url),
        users_list_id: None,
        businesses_list_id: None,
        suppliers_list_id: None,
    };

    let probe = probe_conn(&candidate).await.map_err(AppError::Internal)?;
    if probe.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(AppError::Validation(format!(
            "CoreSwift refused this connection: {}",
            probe
                .get("detail")
                .and_then(|d| d.as_str())
                .unwrap_or("no detail returned")
        )));
    }

    let connection = save_link(
        &s.db,
        scope,
        req.id,
        req.tenant_id,
        req.base_url,
        req.personal_key,
        &req.lists,
    )
    .await
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound(format!("No {} with id {}", scope.as_str(), req.id)))?;

    Ok(Json(json!({
        "success": true,
        "message": "CoreSwift connected.",
        "connection": connection,
        "probe": probe,
    })))
}

/// DELETE /api/v1/integrations/coreswift/connection?scope=&id=
pub async fn coreswift_connection_delete(
    State(s): State<AppState>,
    Query(q): Query<LinkQuery>,
) -> ApiResult<impl IntoResponse> {
    let scope = q.scope()?;
    let cleared = clear_link(&s.db, scope, q.id)
        .await
        .map_err(AppError::Internal)?;
    if !cleared {
        return Err(AppError::NotFound(format!(
            "No {} with id {}",
            scope.as_str(),
            q.id
        )));
    }
    Ok(Json(json!({
        "success": true,
        "message": "CoreSwift disconnected. Captures continue locally.",
    })))
}

/// POST /api/v1/integrations/coreswift/connection/test — a REAL probe of the stored link.
pub async fn coreswift_connection_test(
    State(s): State<AppState>,
    Query(q): Query<LinkQuery>,
) -> ApiResult<impl IntoResponse> {
    let scope = q.scope()?;
    let conn = conn_for_link(&s.db, scope, q.id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "This {} has no CoreSwift tenant linked — connect it first.",
                scope.as_str()
            ))
        })?;

    let probe = probe_conn(&conn).await.map_err(AppError::Internal)?;
    Ok(Json(json!({
        "success": true,
        "tenant_id": conn.tenant_id,
        "key_configured": !conn.api_key.is_empty(),
        "probe": probe,
    })))
}

/// GET /api/v1/integrations/coreswift/connection/lists?scope=&id=
/// The stored lists, plus the hub's own list picker when a personal key is present (the
/// external API endpoint the picker needs; the internal transport can create contacts but
/// not enumerate the tenant's lists).
pub async fn coreswift_connection_lists(
    State(s): State<AppState>,
    Query(q): Query<LinkQuery>,
) -> ApiResult<impl IntoResponse> {
    let scope = q.scope()?;
    let connection = link_status(&s.db, scope, q.id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound(format!("No {} with id {}", scope.as_str(), q.id)))?;

    let (hub, hub_error) = match conn_for_link(&s.db, scope, q.id)
        .await
        .map_err(AppError::Internal)?
    {
        Some(conn) if !conn.api_key.is_empty() => match hub_lists(&conn).await {
            Ok(v) => (Some(v), Value::Null),
            Err(e) => (None, json!(e)),
        },
        Some(_) => (
            None,
            json!(
                "No personal key on this connection — the hub picker needs a csk_ key. \
                 The lists already stored for this link are shown below."
            ),
        ),
        None => (None, json!("No CoreSwift tenant linked yet.")),
    };

    Ok(Json(json!({
        "success": true,
        "connection": connection,
        "hub": hub,
        "hub_error": hub_error,
    })))
}
