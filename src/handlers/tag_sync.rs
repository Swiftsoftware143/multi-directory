//! Cross-platform tag sync handler.
//!
//! When a user signs up or changes role in MultiDirectory, this handler
//! broadcasts the tag event to CoreSwift (contact CRM — tag + list assignment).
//!
//! IncentiveSwift is deliberately NOT contacted: ZaarHub loyalty is native Multi-Directory code
//! and network-scoped (one programme for the whole network — see loyalty_native). The only
//! permitted IncentiveSwift seam is the onboarding/IQS survey proxy (handlers::onboarding_survey).
//!
//! Calls are fire-and-forget with 5s timeouts. Errors are logged, not returned.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::coreswift::{coreswift_url, internal_key};
use crate::error::ApiResult;
use crate::AppState;

lazy_static::lazy_static! {
    static ref SYNC_CLIENT: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("Failed to build sync HTTP client");
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TagSyncEvent {
    pub event: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub phone: Option<String>,
    pub tags: Vec<String>,
    pub city_list: Option<String>, // CoreSwift list name to add to
    pub list_type: Option<String>, // businesses|sponsors|subscribers
    pub directory_slug: Option<String>,
    pub source: Option<String>,
    pub tenant_id: Option<String>, // CoreSwift tenant UUID (resolved by caller if known)
    pub coreswift_list_id: Option<String>, // Pre-resolved list UUID
}

/// Core sync logic — called by both the HTTP handler and fire_tag_sync.
async fn perform_tag_sync(db: &sqlx::PgPool, event: TagSyncEvent) {
    tracing::info!(
        "[tag-sync] Performing sync: email={} tags={:?} source={:?}",
        event.email,
        event.tags,
        event.source
    );

    // ── 0. The tenant comes from the LINK in the database, never from the caller. ──
    // A caller that knew no tenant used to send `tenant_id: null`, and the hub answered
    // 422 "tenant_id: invalid type: null, expected a string" — so every visitor and
    // supplier signup was dropped and no contact was ever created. Nothing linked now means
    // a quiet skip (logged), never a null on the wire.
    let tenant_id = match event
        .tenant_id
        .as_deref()
        .map(str::trim)
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(t) => Some(t),
        None => crate::coreswift::capture_tenant(db, None, event.directory_slug.as_deref()).await,
    };
    let Some(tenant_id) = tenant_id else {
        tracing::warn!(
            "[tag-sync] skipped for {}: no CoreSwift link for directory {:?} — connect one in the admin panel",
            event.email,
            event.directory_slug
        );
        return;
    };

    // The list the audience belongs in, resolved from the link (directory → its network →
    // the one connected network). An explicit id from the caller still wins.
    let list_uuid = match event
        .coreswift_list_id
        .as_deref()
        .map(str::trim)
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => Some(id),
        None => {
            crate::coreswift::audience_list(
                db,
                event.directory_slug.as_deref(),
                event.list_type.as_deref(),
            )
            .await
        }
    };

    let first_name = event.first_name.clone().unwrap_or_default();
    let last_name = event.last_name.clone().unwrap_or_default();
    let source = event
        .source
        .clone()
        .unwrap_or_else(|| "multidirectory".to_string());

    // ── 1. Sync to CoreSwift (fire-and-forget) ─────────────────────
    let cs_payload = json!({
        "source_app": format!("multidirectory/{}", source),
        "tenant_id": tenant_id.to_string(),
        "lead": {
            "id": "",
            "name": format!("{} {}", first_name, last_name).trim(),
            "email": event.email.clone(),
            "company": "",
        },
        "tags": event.tags.clone(),
        "added_tags": event.tags.clone(),
        "removed_tags": [],
        "triggered_by": source.clone(),
    });
    let email = event.email.clone();

    tokio::spawn(async move {
        let url = format!("{}/api/v1/webhooks/cross-app/tag-sync", coreswift_url());
        match SYNC_CLIENT
            .post(&url)
            .header("x-internal-key", internal_key())
            .json(&cs_payload)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    tracing::warn!("[tag-sync] CoreSwift sync returned {status}: {body}");
                    return;
                }
                tracing::info!("[tag-sync] CoreSwift sync OK for {email} (tenant {tenant_id})");
                // The hub answers with the contact it upserted — reuse that id for list
                // membership instead of creating a second contact for the same person.
                let contact_id = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("contact_id")
                            .and_then(|c| c.as_str())
                            .map(str::to_string)
                    })
                    .and_then(|s| Uuid::parse_str(&s).ok());
                if let (Some(list_id), Some(contact_id)) = (list_uuid, contact_id) {
                    add_contact_to_coreswift_list(tenant_id, list_id, contact_id).await;
                }
            }
            Err(e) => {
                tracing::warn!("[tag-sync] CoreSwift sync request failed: {e}");
            }
        }
    });

    // ── 2. IncentiveSwift: deliberately NOT called. ───────────────
    // ZaarHub loyalty is native to Multi-Directory and network-scoped (see
    // loyalty_native::enroll_visitor_in_network_loyalty). David's standing rule: the only
    // permitted IncentiveSwift seam is the onboarding/IQS survey proxy
    // (handlers::onboarding_survey). The old tag-contact POST here hit a programme that does
    // not exist on IS, so it failed on every signup and only polluted the logs.
}

/// POST /admin/tag-sync
/// Called internally when a user signs up or changes role.
pub async fn sync_tag_across_platforms(
    State(s): State<AppState>,
    Json(body): Json<TagSyncEvent>,
) -> ApiResult<Json<Value>> {
    // Spawn the sync work so the HTTP response returns immediately
    let db = s.db.clone();
    tokio::spawn(async move {
        perform_tag_sync(&db, body).await;
    });

    Ok(Json(json!({
        "status": "accepted",
        "message": "Tag sync broadcast initiated",
    })))
}

/// Add an ALREADY-CREATED contact to a CoreSwift list.
///
/// This used to create its own contact through `POST /api/internal/contacts` with
/// `"tenant_id": ""` — an invalid uuid the hub rejects with 400 — and, even when it did
/// work, it inserted a duplicate person instead of reusing the one tag-sync had just
/// upserted. The caller now passes the hub's own contact id.
async fn add_contact_to_coreswift_list(tenant_id: Uuid, list_id: Uuid, contact_id: Uuid) {
    let resp = SYNC_CLIENT
        .post(format!(
            "{}/api/internal/lists/{}/members",
            coreswift_url(),
            list_id
        ))
        .header("x-internal-key", internal_key())
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "contact_id": contact_id.to_string(),
        }))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => tracing::info!(
            "[tag-sync] contact {contact_id} added to list {list_id} (tenant {tenant_id})"
        ),
        Ok(r) => tracing::warn!("[tag-sync] list {list_id} add returned {}", r.status()),
        Err(e) => tracing::warn!("[tag-sync] list {list_id} add failed: {e}"),
    }
}

// ── Convenience function for signup flows ───────────────────────────────────

/// Fire a tag sync event asynchronously from a signup flow.
/// Calls the sync logic directly (no HTTP round-trip).
pub fn fire_tag_sync(
    db: &sqlx::PgPool,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    tags: Vec<String>,
    city_list: Option<String>,
    list_type: Option<String>,
    directory_slug: Option<String>,
    source: Option<String>,
    tenant_id: Option<String>,
    coreswift_list_id: Option<String>,
) {
    let event = TagSyncEvent {
        event: "contact_tagged".to_string(),
        email,
        first_name,
        last_name,
        phone,
        tags,
        city_list,
        list_type,
        directory_slug,
        source,
        tenant_id,
        coreswift_list_id,
    };

    let db = db.clone();
    tokio::spawn(async move {
        perform_tag_sync(&db, event).await;
    });
}

// NOTE (2026-09-21): register_member_in_is() is gone. It POSTed every signup to
// http://localhost:8083/api/v1/loyalty/external/register-member, an IncentiveSwift programme that
// does not exist, so it failed on every signup and polluted the logs. Loyalty enrolment is now
// native and network-scoped: loyalty_native::enroll_visitor_in_network_loyalty().
