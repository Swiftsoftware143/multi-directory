//! Cross-platform tag sync handler.
//!
//! When a user signs up or changes role in MultiDirectory, this handler
//! broadcasts the tag event to:
//!   1. CoreSwift (contact CRM — tag + list assignment)
//!   2. IncentiveSwift (loyalty engine — tag for campaign eligibility)
//!
//! Both calls are fire-and-forget with 5s timeouts. Errors are logged, not returned.

use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiResult;
use crate::coreswift::{internal_key, coreswift_url};

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
    pub city_list: Option<String>,      // CoreSwift list name to add to
    pub list_type: Option<String>,       // businesses|sponsors|subscribers
    pub directory_slug: Option<String>,
    pub source: Option<String>,
    pub tenant_id: Option<String>,       // CoreSwift tenant UUID (resolved by caller if known)
    pub coreswift_list_id: Option<String>, // Pre-resolved list UUID
}

/// Core sync logic — called by both the HTTP handler and fire_tag_sync.
async fn perform_tag_sync(event: TagSyncEvent) {
    tracing::info!(
        "[tag-sync] Performing sync: email={} tags={:?} source={:?}",
        event.email, event.tags, event.source
    );

    let first_name = event.first_name.clone().unwrap_or_default();
    let last_name = event.last_name.clone().unwrap_or_default();
    let phone = event.phone.clone().unwrap_or_default();
    let source = event.source.clone().unwrap_or_else(|| "multidirectory".to_string());
    let directory_slug = event.directory_slug.clone().unwrap_or_default();
    let coreswift_list_id = event.coreswift_list_id.clone();

    // ── 1. Sync to CoreSwift (fire-and-forget) ─────────────────────
    let cs_url = format!("{}/api/v1/webhooks/cross-app/tag-sync", coreswift_url());
    let cs_key = internal_key();

    {
        let email = event.email.clone();
        let cs_payload = json!({
            "source_app": format!("multidirectory/{}", source),
            "tenant_id": &event.tenant_id,
            "lead": {
                "id": "",
                "name": format!("{} {}", first_name, last_name).trim(),
                "email": email,
                "company": "",
            },
            "tags": event.tags.clone(),
            "added_tags": event.tags.clone(),
            "removed_tags": [],
            "triggered_by": source.clone(),
        });
        let cs_url_1 = cs_url.clone();
        let cs_key_1 = cs_key.clone();

        tokio::spawn(async move {
            match SYNC_CLIENT
                .post(&cs_url_1)
                .header("x-internal-key", &cs_key_1)
                .json(&cs_payload)
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!("[tag-sync] CoreSwift sync OK for {}", email);
                    } else {
                        let status = resp.status();
                        let body_text = resp.text().await.unwrap_or_default();
                        tracing::warn!("[tag-sync] CoreSwift sync returned {status}: {body_text}");
                    }
                }
                Err(e) => {
                    tracing::warn!("[tag-sync] CoreSwift sync request failed: {e}");
                }
            }
        });
    }

    // ── 2. Sync to IncentiveSwift (fire-and-forget) ────────────────
    let is_url = "http://localhost:8083/api/v1/loyalty/external/tag-contact".to_string();
    let email_is = event.email.clone();

    {
        let is_payload = json!({
            "event": "contact_tagged",
            "email": event.email,
            "first_name": first_name,
            "last_name": last_name,
            "phone": phone,
            "tags": event.tags,
            "source": source,
            "directory_slug": directory_slug,
        });

        tokio::spawn(async move {
            match SYNC_CLIENT
                .post(&is_url)
                .json(&is_payload)
                .send()
                .await
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!("[tag-sync] IncentiveSwift sync OK for {}", email_is);
                    } else {
                        let status = resp.status();
                        let body_text = resp.text().await.unwrap_or_default();
                        tracing::warn!("[tag-sync] IncentiveSwift sync returned {status}: {body_text}");
                    }
                }
                Err(e) => {
                    tracing::warn!("[tag-sync] IncentiveSwift sync request failed: {e}");
                }
            }
        });
    }

    // ── 3. List membership (CoreSwift) — low priority ──────────
    if let Some(ref list_id) = coreswift_list_id {
        if let Ok(list_uuid) = Uuid::parse_str(list_id) {
            let email_cs = event.email.clone();
            let cs_url_c = cs_url.clone();
            let cs_key_c = cs_key.clone();
            tokio::spawn(async move {
                let _ = add_contact_to_coreswift_list(cs_url_c, cs_key_c, list_uuid, email_cs).await;
            });
        }
    }
}

/// POST /admin/tag-sync
/// Called internally when a user signs up or changes role.
pub async fn sync_tag_across_platforms(
    State(_s): State<AppState>,
    Json(body): Json<TagSyncEvent>,
) -> ApiResult<Json<Value>> {
    // Spawn the sync work so the HTTP response returns immediately
    tokio::spawn(async move {
        perform_tag_sync(body).await;
    });

    Ok(Json(json!({
        "status": "accepted",
        "message": "Tag sync broadcast initiated",
    })))
}

/// Add a contact to a CoreSwift list by list UUID.
async fn add_contact_to_coreswift_list(
    coreswift_url: String,
    internal_key: String,
    list_id: Uuid,
    email: String,
) -> Result<(), ()> {
    // Find or create the contact in CoreSwift first to get the CoreSwift contact ID
    let resp = SYNC_CLIENT
        .post(format!("{}/api/internal/contacts", coreswift_url))
        .header("x-internal-key", &internal_key)
        .json(&json!({
            "tenant_id": "",
            "first_name": &email,
            "last_name": "Contact",
            "email": &email,
        }))
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("[tag-sync] Failed to create CoreSwift contact for list add: {e}");
        })?;

    if !resp.status().is_success() {
        tracing::warn!("[tag-sync] CoreSwift contact create returned {}", resp.status());
        return Err(());
    }

    let contact_body: Value = resp.json().await.map_err(|_| ())?;
    let contact_id_str = contact_body["id"].as_str().unwrap_or("").to_string();

    if contact_id_str.is_empty() {
        return Err(());
    }

    let _ = SYNC_CLIENT
        .post(format!("{}/api/internal/lists/{}/members", coreswift_url, list_id))
        .header("x-internal-key", &internal_key)
        .json(&json!({
            "contact_id": contact_id_str,
        }))
        .send()
        .await;

    Ok(())
}

// ── Convenience function for signup flows ───────────────────────────────────

/// Fire a tag sync event asynchronously from a signup flow.
/// Calls the sync logic directly (no HTTP round-trip).
pub fn fire_tag_sync(
    _db: &sqlx::PgPool,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    phone: Option<String>,
    tags: Vec<String>,
    city_list: Option<String>,
    _list_type: Option<String>,
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
        list_type: None,
        directory_slug,
        source,
        tenant_id,
        coreswift_list_id,
    };

    tokio::spawn(async move {
        perform_tag_sync(event).await;
    });
}
