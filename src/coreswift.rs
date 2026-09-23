//! CoreSwift integration — push business claims and newsletter signups
//! to CoreSwift CRM via its internal API (x-internal-key auth).
//!
//! Architecture:
//!   - One CoreSwift tenant per network (or per standalone directory)
//!   - Auto-provisioned: tenant + 3 default lists + city-prefixed tag groups on first use
//!   - Data pushed via internal endpoints (bypasses JWT)
//!   - Database is always source of truth; CoreSwift push is secondary

use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// CoreSwift URL — read from CORESWIFT_URL env var at runtime, fallback localhost:8084
pub fn coreswift_url() -> String {
    std::env::var("CORESWIFT_URL").unwrap_or_else(|_| "http://localhost:8084".to_string())
}

/// Internal key for CoreSwift API — MUST be set via CORESWIFT_INTERNAL_KEY env var
pub fn internal_key() -> String {
    std::env::var("CORESWIFT_INTERNAL_KEY")
        .expect("CORESWIFT_INTERNAL_KEY environment variable must be set")
}

lazy_static::lazy_static! {
    static ref HTTP: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build reqwest client");
}

/// Resolve CoreSwift config for a directory.
/// Returns (tenant_id, claimed_list_id, newsletter_list_id, sponsors_list_id).
/// Checks directory-level first, then falls back to parent network.
pub async fn resolve_config(
    db: &PgPool,
    directory_id: Uuid,
) -> Result<(Uuid, Uuid, Uuid, Uuid), String> {
    let row = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>)>(
        r#"SELECT d.coreswift_tenant_id, d.network_id, d.coreswift_list_id_claimed, d.coreswift_list_id_newsletter, d.coreswift_list_id_sponsors,
                  n.coreswift_tenant_id AS net_tenant_id
           FROM directories d
           LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1"#
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?
    .ok_or_else(|| format!("Directory {directory_id} not found"))?;

    let (dir_tid, network_id, dir_lc, dir_ln, dir_ls, net_tid) = row;

    if let (Some(tid), Some(lc), Some(ln), Some(ls)) = (dir_tid, dir_lc, dir_ln, dir_ls) {
        return Ok((tid, lc, ln, ls));
    }

    if let Some(tid) = net_tid {
        if let Some(nid) = network_id {
            let (lc, ln, ls) = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<Uuid>)>(
                "SELECT coreswift_list_id_claimed, coreswift_list_id_newsletter, coreswift_list_id_sponsors FROM networks WHERE id = $1"
            )
            .bind(nid)
            .fetch_optional(db)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or_else(|| format!("Network {nid} not found"))?;

            if let (Some(lc), Some(ln), Some(ls)) = (lc, ln, ls) {
                return Ok((tid, lc, ln, ls));
            }
        }
    }

    Err(format!(
        "No CoreSwift tenant provisioned for directory {directory_id}"
    ))
}

fn cs_url(path: &str) -> String {
    format!("{}{}", coreswift_url(), path)
}

/// Provision a CoreSwift tenant for a new entity (network or standalone directory).
/// Creates: tenant account, 3 lists (claimed, newsletter, sponsors), and city-prefixed tag groups.
/// Stores the tenant + list IDs back in the database.
pub async fn provision_tenant(
    db: &PgPool,
    entity_id: Uuid,
    name: &str,
    slug: &str,
    is_network: bool,
) -> Result<(), String> {
    let email = format!("md-{slug}@local.coreswift");
    let pass = Uuid::new_v4().to_string();
    let base = coreswift_url();

    let resp = HTTP
        .post(format!("{base}/api/auth/register"))
        .json(&json!({
            "name": format!("Multi-Directory: {name}"),
            "email": email,
            "password": pass,
            "account_name": name,
            "account_slug": slug,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift register failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift register returned {status}: {body}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift register parse failed: {e}"))?;

    let tenant_id = body["account"]["id"]
        .as_str()
        .ok_or_else(|| format!("Missing account.id in register response: {body}"))?;
    let token = body["access_token"]
        .as_str()
        .ok_or_else(|| format!("Missing access_token in register response: {body}"))?;

    let tid = Uuid::parse_str(tenant_id).map_err(|e| format!("Bad tenant UUID: {e}"))?;

    // Create the 3 base lists
    let claimed_list = create_list(token, name, "Claimed Businesses").await?;
    let newsletter_list = create_list(token, name, "Newsletter Subscribers").await?;
    let sponsors_list = create_list(token, name, "Sponsors").await?;

    if is_network {
        sqlx::query(
            "UPDATE networks SET coreswift_tenant_id = $1, coreswift_list_id_claimed = $2, coreswift_list_id_newsletter = $3, coreswift_list_id_sponsors = $4 WHERE id = $5"
        )
        .bind(tid)
        .bind(claimed_list)
        .bind(newsletter_list)
        .bind(sponsors_list)
        .bind(entity_id)
        .execute(db)
        .await
        .map_err(|e| format!("DB update failed: {e}"))?;
    } else {
        sqlx::query(
            "UPDATE directories SET coreswift_tenant_id = $1, coreswift_list_id_claimed = $2, coreswift_list_id_newsletter = $3, coreswift_list_id_sponsors = $4 WHERE id = $5"
        )
        .bind(tid)
        .bind(claimed_list)
        .bind(newsletter_list)
        .bind(sponsors_list)
        .bind(entity_id)
        .execute(db)
        .await
        .map_err(|e| format!("DB update failed: {e}"))?;
    }

    tracing::info!("[coreswift] Provisioned tenant {tenant_id} for {name} ({slug})");
    Ok(())
}

/// Create city-prefixed tags for a directory's CoreSwift tenant.
/// Tags follow the pattern: {city_prefix}-{tag_type} (e.g. "pb-featured", "pb-sponsors").
/// This is called when a new city is added to a network to set up the tag group.
/// Currently creates placeholder tags — real tag names come from the admin settings.
pub async fn provision_city_tags(
    db: &PgPool,
    directory_id: Uuid,
    city_prefix: &str,
) -> Result<Vec<(String, Uuid)>, String> {
    let (tenant_id, _, _, _) = resolve_config(db, directory_id).await?;
    let mut results = Vec::new();

    // Create initial placeholder tags for this city
    // Tag types are deliberately generic — admins rename them later via the UI
    let initial_tags = vec![
        ("featured", "#f59e0b"),
        ("sponsors", "#10b981"),
        ("premium", "#8b5cf6"),
    ];

    for (tag_type, color) in &initial_tags {
        let full_name = format!("{}-{}", city_prefix, tag_type);
        match create_tag_internal(tenant_id, &full_name, color).await {
            Ok(tag_id) => {
                results.push((full_name, tag_id));
            }
            Err(e) => {
                tracing::warn!("[coreswift] Failed to create tag '{full_name}': {e}");
                // Non-fatal — continue with other tags
            }
        }
    }

    Ok(results)
}

/// Create a static list in CoreSwift, return its UUID.
async fn create_list(token: &str, _name: &str, list_name: &str) -> Result<Uuid, String> {
    let base = coreswift_url();
    let resp = HTTP
        .post(format!("{base}/api/lists"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "name": list_name,
            "list_type": "static",
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift create list '{list_name}' failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift list create returned {status}: {body}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift create list '{list_name}' response parse: {e}"))?;

    body["id"]
        .as_str()
        .or_else(|| body["list"]["id"].as_str())
        .ok_or_else(|| format!("Missing list id: {body}"))
        .and_then(|s| Uuid::parse_str(s).map_err(|e| format!("Bad list UUID: {e}")))
}

/// Create or find a tag in CoreSwift, return its UUID.
/// Tags follow the convention: {prefix}-{name} (e.g. "pb-featured", "pc-sponsors").
/// Uses the internal API (x-internal-key) to bypass JWT.
/// If the tag already exists, returns the existing ID (idempotent).
pub async fn create_tag_internal(
    tenant_id: Uuid,
    tag_name: &str,
    color: &str,
) -> Result<Uuid, String> {
    let base = coreswift_url();
    let key = internal_key();
    let resp = HTTP
        .post(format!("{base}/api/internal/tags"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "name": tag_name,
            "color": color,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift create tag '{tag_name}' failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift tag create returned {status}: {body}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift create tag '{tag_name}' response parse: {e}"))?;

    body["id"]
        .as_str()
        .ok_or_else(|| format!("Missing tag id: {body}"))
        .and_then(|s| Uuid::parse_str(s).map_err(|e| format!("Bad tag UUID: {e}")))
}

/// Look up a tag by name on a specific CoreSwift tenant via the internal API.
/// Uses POST /api/internal/tags/list which returns all tags for a tenant.
pub async fn find_tag_by_name(tenant_id: Uuid, tag_name: &str) -> Result<Option<Uuid>, String> {
    let base = coreswift_url();
    let key = internal_key();
    let resp = HTTP
        .post(format!("{base}/api/internal/tags/list"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift list tags failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift list tags returned {status}: {body}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift list tags response parse: {e}"))?;

    if let Some(tags) = body["tags"].as_array() {
        for tag in tags {
            if let Some(name) = tag["name"].as_str() {
                if name == tag_name {
                    if let Some(id_str) = tag["id"].as_str() {
                        if let Ok(id) = Uuid::parse_str(id_str) {
                            return Ok(Some(id));
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Look up a CoreSwift tenant by slug via the internal API.
/// Uses POST /api/internal/tenants/lookup which returns tenant id/name/slug.
pub async fn find_tenant_by_slug(slug: &str) -> Result<Option<Uuid>, String> {
    let base = coreswift_url();
    let key = internal_key();
    let resp = HTTP
        .post(format!("{base}/api/internal/tenants/lookup"))
        .header("x-internal-key", &key)
        .json(&json!({
            "slug": slug,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift tenant lookup failed: {e}"))?;

    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift tenant lookup returned {status}: {body}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift tenant lookup response parse: {e}"))?;

    Ok(body["id"].as_str().and_then(|s| Uuid::parse_str(s).ok()))
}

/// Push a business owner to the CRM — creates a contact and adds to "Claimed Businesses" list.
pub async fn push_claimed_business(
    db: &PgPool,
    business_id: Uuid,
    owner_email: &str,
    owner_name: Option<&str>,
    owner_phone: Option<&str>,
) -> Result<(), String> {
    let dir_id = sqlx::query_scalar::<_, Uuid>("SELECT directory_id FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("DB error: {e}"))?
        .ok_or_else(|| format!("Business {business_id} not found"))?;

    let (tenant_id, claimed_list_id, _, _) = resolve_config(db, dir_id).await?;
    let base = coreswift_url();
    let key = internal_key();

    // Create contact
    let resp = HTTP
        .post(format!("{base}/api/internal/contacts"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "first_name": owner_name.unwrap_or("Business"),
            "last_name": "Owner",
            "email": owner_email,
            "phone": owner_phone,
            "notes": format!("Claimed business {business_id}")
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift contact create failed: {e}"))?;

    let c_status = resp.status();
    if !c_status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "CoreSwift contact create returned {c_status}: {body}"
        ));
    }

    let contact_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift contact response parse: {e}"))?;

    let contact_id_str = contact_body["id"]
        .as_str()
        .ok_or_else(|| format!("Missing contact id: {contact_body}"))?;

    // Add to claimed businesses list
    let resp = HTTP
        .post(format!(
            "{base}/api/internal/lists/{claimed_list_id}/members"
        ))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "contact_id": contact_id_str,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift list add failed: {e}"))?;

    let ls = resp.status();
    if !ls.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("[coreswift] List add returned {ls}: {body}");
    }

    // Assign biz-zaarhub-interested tag on the ZaarHub parent tenant
    let dir_slug: String =
        sqlx::query_scalar::<_, Option<String>>("SELECT slug FROM directories WHERE id = $1")
            .bind(dir_id)
            .fetch_optional(db)
            .await
            .unwrap_or(None)
            .flatten()
            .unwrap_or_default();

    if !dir_slug.is_empty() {
        // Derive short prefix (palm-bay → pb)
        let prefix: String = dir_slug
            .split('-')
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_lowercase();
        let interested_tag_name = format!("{}-biz-zh-interested", prefix);

        // Find the tag on the ZaarHub parent tenant
        // Uses CoreSwift API; the tenants table is in the coreswift database, not multidirectory
        let zaarhub_tenant_id: Option<Uuid> = find_tenant_by_slug("zaarhub").await.unwrap_or(None);

        if let Some(zh_tid) = zaarhub_tenant_id {
            // Look up the tag via CoreSwift API (not SQL — tags are in coreswift DB)
            match find_tag_by_name(zh_tid, &interested_tag_name).await {
                Ok(Some(tag_id)) => {
                    if let Ok(contact_uuid) = Uuid::parse_str(contact_id_str) {
                        let _ = assign_contact_tag(zh_tid, contact_uuid, tag_id).await;
                        tracing::info!("[coreswift] Assigned '{interested_tag_name}' tag to claimed business contact");
                    }
                }
                Ok(None) => {
                    tracing::warn!("[coreswift] Tag '{interested_tag_name}' not found on ZaarHub parent tenant — creating it");
                    // Create it on the fly so assignment still works
                    if let Ok(tag_id) =
                        create_tag_internal(zh_tid, &interested_tag_name, "#3b82f6").await
                    {
                        if let Ok(contact_uuid) = Uuid::parse_str(contact_id_str) {
                            let _ = assign_contact_tag(zh_tid, contact_uuid, tag_id).await;
                            tracing::info!("[coreswift] Created + assigned '{interested_tag_name}' tag to claimed business contact");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[coreswift] Failed to look up tag '{interested_tag_name}': {e}"
                    );
                }
            }
        }
    }

    tracing::info!("[coreswift] Pushed claimed business {business_id} (owner: {owner_email})");
    Ok(())
}

/// Push a newsletter signup to the CRM — creates a contact and adds to "Newsletter Subscribers" list.
/// Returns the CoreSwift contact ID so callers can assign tags.
pub async fn push_newsletter_signup(
    db: &PgPool,
    directory_id: Uuid,
    email: &str,
    name: Option<&str>,
) -> Result<Uuid, String> {
    let (tenant_id, _, newsletter_list_id, _) = resolve_config(db, directory_id).await?;
    let base = coreswift_url();
    let key = internal_key();

    let resp = HTTP
        .post(format!("{base}/api/internal/contacts"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "first_name": name.unwrap_or("Newsletter"),
            "last_name": "Subscriber",
            "email": email,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift contact create failed: {e}"))?;

    let c_status = resp.status();
    if !c_status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "CoreSwift contact create returned {c_status}: {body}"
        ));
    }

    let contact_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift contact response parse: {e}"))?;

    let contact_id_str = contact_body["id"]
        .as_str()
        .ok_or_else(|| format!("Missing contact id: {contact_body}"))?;

    let contact_id =
        Uuid::parse_str(contact_id_str).map_err(|e| format!("Bad contact UUID: {e}"))?;

    let resp = HTTP
        .post(format!(
            "{base}/api/internal/lists/{newsletter_list_id}/members"
        ))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "contact_id": contact_id_str,
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift list add failed: {e}"))?;

    let ls = resp.status();
    if !ls.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("[coreswift] List add returned {ls}: {body}");
    }

    // If there's a city tag, assign it
    if let Ok(Some((tag_id,))) = sqlx::query_as::<_, (Uuid,)>(
        "SELECT tag_id FROM _city_tags WHERE directory_id = $1 LIMIT 1",
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    {
        let _ = assign_contact_tag_for_subscriber(tenant_id, contact_id, tag_id).await;
    }

    tracing::info!("[coreswift] Pushed newsletter signup ({email})");
    Ok(contact_id)
}

/// Assign a city newsletter tag to a CoreSwift contact.
async fn assign_contact_tag_for_subscriber(
    tenant_id: Uuid,
    contact_id: Uuid,
    tag_id: Uuid,
) -> Result<(), String> {
    let base = coreswift_url();
    let key = internal_key();

    let resp = HTTP
        .post(format!("{base}/api/internal/tags/assign"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "entity_id": contact_id.to_string(),
            "entity_type": "contact",
            "tag_id": tag_id.to_string(),
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift tag assign failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift tag assign returned {status}: {body}"));
    }

    tracing::info!("[coreswift] Assigned tag {tag_id} to contact {contact_id}");
    Ok(())
}

/// Add a contact to the "Claimed Businesses" list in CoreSwift.
pub async fn add_to_claimed_list(
    db: &PgPool,
    directory_id: Uuid,
    contact_email: &str,
) -> Result<(), String> {
    let (tenant_id, claimed_list_id, _, _) = resolve_config(db, directory_id).await?;
    let base = coreswift_url();
    let key = internal_key();

    let resp = HTTP
        .get(format!("{base}/api/contacts/search?q={contact_email}"))
        .header("x-internal-key", &key)
        .send()
        .await
        .map_err(|e| format!("CoreSwift search failed: {e}"))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("CoreSwift search response parse: {e}"))?;

    if let Some(contacts) = body["contacts"].as_array() {
        if let Some(contact) = contacts.first() {
            if let Some(cid) = contact["id"].as_str() {
                let resp = HTTP
                    .post(format!(
                        "{base}/api/internal/lists/{claimed_list_id}/members"
                    ))
                    .header("x-internal-key", &key)
                    .json(&json!({
                        "tenant_id": tenant_id.to_string(),
                        "contact_id": cid,
                    }))
                    .send()
                    .await
                    .map_err(|e| format!("CoreSwift list add failed: {e}"))?;

                if resp.status().is_success() {
                    return Ok(());
                }
            }
        }
    }

    Err(format!("Contact {contact_email} not found in CoreSwift"))
}

/// Assign a tag to a contact in CoreSwift by contact and tag ID.
/// Uses the internal API (x-internal-key).
pub async fn assign_contact_tag(
    tenant_id: Uuid,
    contact_id: Uuid,
    tag_id: Uuid,
) -> Result<(), String> {
    let base = coreswift_url();
    let key = internal_key();

    let resp = HTTP
        .post(format!("{base}/api/internal/tags/assign"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "entity_id": contact_id.to_string(),
            "entity_type": "contact",
            "tag_id": tag_id.to_string(),
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift tag assign failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift tag assign returned {status}: {body}"));
    }

    Ok(())
}

/// Remove a tag from a contact in CoreSwift by contact and tag ID.
/// Uses the internal API (x-internal-key).
pub async fn remove_contact_tag(
    tenant_id: Uuid,
    contact_id: Uuid,
    tag_id: Uuid,
) -> Result<(), String> {
    let base = coreswift_url();
    let key = internal_key();

    let resp = HTTP
        .post(format!("{base}/api/internal/tags/delete"))
        .header("x-internal-key", &key)
        .json(&json!({
            "tenant_id": tenant_id.to_string(),
            "tag_id": tag_id.to_string(),
            "entity_id": contact_id.to_string(),
            "entity_type": "contact",
        }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift tag remove failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("CoreSwift tag remove returned {status}: {body}"));
    }

    Ok(())
}

// ── Full directory provisioning (all-in-one) ────────────────────────────────

/// Provision ALL CoreSwift resources for a newly created directory city.
///
/// This is the single entry point called from `create_directory`.
/// It handles:
/// 1. Setting the city prefix as `booking_calendar_slug`
/// 2. Creating the booking calendar + default slot in CoreSwift
/// 3. Creating city-prefixed tags (featured, sponsors, premium)
///
/// Returns the city prefix (e.g. "pb-" for "palm-bay") on success.
pub async fn provision_directory_resources(
    db: &PgPool,
    directory_id: Uuid,
    directory_slug: &str,
) -> Result<String, String> {
    // Derive city prefix: first two letters of each word, joined with "-"
    // "palm-bay" → "pb-", "st-petersburg" → "sp-"
    let prefix: String = directory_slug
        .split('-')
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_lowercase();
    let prefix = format!("{}-", prefix);

    // Step 1: Set booking_calendar_slug
    sqlx::query("UPDATE directories SET booking_calendar_slug = $1 WHERE id = $2")
        .bind(&prefix)
        .bind(directory_id)
        .execute(db)
        .await
        .map_err(|e| format!("Failed to set booking_calendar_slug: {e}"))?;

    tracing::info!("[provision] Set booking_calendar_slug='{prefix}' for directory {directory_id}");

    // Step 2: Resolve tenant ID - could be on directory or its parent network
    let tenant_id = sqlx::query_scalar::<_, Option<Uuid>>(
        r#"SELECT COALESCE(d.coreswift_tenant_id, n.coreswift_tenant_id)
           FROM directories d
           LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1"#,
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("Failed to resolve tenant ID: {e}"))?
    .flatten();

    if let Some(tenant_id) = tenant_id {
        // Step 2a: Create booking calendar + default slot in CoreSwift
        let base = coreswift_url();
        let key = internal_key();

        // Derive city name for display
        let city_name = directory_slug
            .replace('-', " ")
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        // Create calendar (idempotent — duplicate key returns existing)
        let _ = HTTP
            .post(format!("{}/api/internal/bookings/calendars", base))
            .header("x-internal-key", &key)
            .json(&json!({
                "name": format!("{} Directory Bookings", city_name),
                "slug": &prefix,
                "description": format!("Appointment bookings for {} directory", city_name),
                "calendar_type": "city",
                "metadata": {
                    "tenant_id": tenant_id.to_string(),
                    "city_slug": directory_slug,
                }
            }))
            .send()
            .await;

        // Create default slot (idempotent)
        let _ = HTTP
            .post(format!("{}/api/internal/bookings/slots/default", base))
            .header("x-internal-key", &key)
            .json(&json!({
                "tenant_id": tenant_id.to_string(),
                "calendar_slug": &prefix,
                "slot_name": "Appointment Booking",
                "total_slots": -1,
                "default_duration_days": 1,
            }))
            .send()
            .await;

        tracing::info!(
            "[provision] Booking calendar '{prefix}' + default slot ready for tenant {tenant_id}"
        );

        // Step 2b: Create city-prefixed tags on the directory's tenant
        // Listing tags — these define the business tiers available in the directory
        let listing_tags: Vec<(&str, &str)> = vec![
            ("featured", "#f59e0b"),
            ("listed", "#6b7280"),
            ("premium", "#8b5cf6"),
            ("sponsors", "#10b981"),
        ];

        let tenant_id_str = tenant_id.to_string();
        let city_name_used = city_name.clone();

        for (tag_type, color) in &listing_tags {
            let full_name = format!(
                "{}-{}",
                city_name_used.to_lowercase().replace(' ', "-"),
                tag_type
            );
            let _ = HTTP
                .post(format!("{}/api/internal/tags", base))
                .header("x-internal-key", &key)
                .json(&json!({
                    "tenant_id": tenant_id_str.as_str(),
                    "name": &full_name,
                    "color": color,
                }))
                .send()
                .await;
        }

        // Step 2c: Create ZaarHub tracking tags on the parent tenant
        // Uses CoreSwift API; the tenants table is in the coreswift database, not multidirectory
        let zaarhub_tenant_id: Option<Uuid> = find_tenant_by_slug("zaarhub").await.unwrap_or(None);

        if let Some(zh_tid) = zaarhub_tenant_id {
            let zh_tid_str = zh_tid.to_string();
            let zh_prefix = prefix.trim_end_matches('-').to_string();
            let zh_tags: Vec<(&str, &str)> = vec![
                ("fb-zh", "#3b82f6"),
                ("biz-zh-interested", "#3b82f6"),
                ("biz-zh-qualified", "#22c55e"),
                ("nl-zh", "#ec4899"),
                ("outofarea-zh", "#ef4444"),
                ("sponsor-zh", "#14b8a6"),
                ("unsub-zh", "#6b7280"),
            ];

            for (suffix, color) in &zh_tags {
                let full_name = format!("{}-{}", zh_prefix, suffix);
                let _ = HTTP
                    .post(format!("{}/api/internal/tags", base))
                    .header("x-internal-key", &key)
                    .json(&json!({
                        "tenant_id": zh_tid_str.as_str(),
                        "name": &full_name,
                        "color": color,
                    }))
                    .send()
                    .await;
            }

            tracing::info!(
                "[provision] ZaarHub tracking tags created for '{prefix}' on parent tenant"
            );
        }

        tracing::info!("[provision] All city tags created for '{prefix}'");
    } else {
        tracing::warn!("[provision] No CoreSwift tenant found for directory {directory_id} — skipping booking/tags");
    }

    Ok(prefix)
}
// ─────────────────────────────────────────────────────────────────────────────
// ─────────────────────────────────────────────────────────────────────────────
// ─────────────────────────────────────────────────────────────────────────────
// Loyalty / Directory → CoreSwift drill-down (Phase 2a)
//
// David's model:
//   - Directory = STANDALONE or part of a NETWORK (ZaarHub = network of directories).
//   - 3 participant types per directory, each flowing into its OWN CoreSwift list:
//       users/customers, businesses, suppliers  (CoreSwift = backend comms hub).
//   - Surveys/quizzes from IQS, Loyalty, or the directory itself assign TAGS,
//     which must propagate into CoreSwift against the same contact.
//
// All pushes go through CoreSwift's EXTERNAL personal-key API (/api/external/contacts)
// which supports: free-text `company`+`title`, email upsert, `list_id` membrship,
// `tags[]` (auto-create + idempotent assign), and `fields{}` auto-provisioning.
// ─────────────────────────────────────────────────────────────────────────────

/// Typed CoreSwift connection resolved for a directory (with network fallback).
#[derive(Clone)]
pub struct CoreSwiftConn {
    pub tenant_id: Uuid,
    pub api_key: String,
    pub base_url: String,
    pub users_list_id: Option<Uuid>,
    pub businesses_list_id: Option<Uuid>,
    pub suppliers_list_id: Option<Uuid>,
}

/// Resolve the per-directory CoreSwift connection + typed lists.
/// Directory-level wins; falls back to parent network (shared tenant networks).
pub async fn resolve_cs_conn(db: &PgPool, directory_id: Uuid) -> Result<CoreSwiftConn, String> {
    let row = sqlx::query_as::<_, (
        Option<Uuid>,          // dir tenant
        Option<Uuid>,          // network_id
        Option<Vec<u8>>,       // dir key
        Option<String>,        // dir base_url
        Option<Uuid>,          // dir users list
        Option<Uuid>,          // dir businesses list
        Option<Uuid>,          // dir suppliers list
        Option<Uuid>,          // net tenant
        Option<Vec<u8>>,       // net key
        Option<String>,        // net base_url
        Option<Uuid>,          // net users list
        Option<Uuid>,          // net businesses list
        Option<Uuid>,          // net suppliers list
    )>(
        r#"SELECT d.coreswift_tenant_id, d.network_id,
                  d.coreswift_personal_key_encrypted, d.coreswift_base_url,
                  d.coreswift_list_id_users, d.coreswift_list_id_businesses, d.coreswift_list_id_suppliers,
                  n.coreswift_tenant_id,
                  n.coreswift_personal_key_encrypted, n.coreswift_base_url,
                  n.coreswift_list_id_users, n.coreswift_list_id_businesses, n.coreswift_list_id_suppliers
           FROM directories d
           LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1"#
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error resolving CoreSwift conn: {e}"))?
    .ok_or_else(|| format!("Directory {directory_id} not found"))?;

    let (
        dir_tid,
        _net,
        dir_key,
        dir_base,
        dir_ul,
        dir_bl,
        dir_sl,
        net_tid,
        net_key,
        net_base,
        net_ul,
        net_bl,
        net_sl,
    ) = row;

    // Tenant: dir first, then net
    let tenant_id = dir_tid
        .or(net_tid)
        .ok_or_else(|| format!("No CoreSwift tenant provisioned for directory {directory_id}"))?;

    // Key: dir first, then net. A missing personal key is NOT an error any more — the
    // tenant + list link alone drives the internal-API push (the same mechanism the claim
    // and newsletter pushes already use). The `csk_` personal key is only needed for the
    // field-mapping path (`/api/external/contacts`), so a directory can connect, receive
    // its signups and carry the identity BEFORE anyone pastes a key.
    let (enc_key, base_url) = match (&dir_key, &dir_base) {
        (Some(k), b) => (
            Some(k.clone()),
            b.clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(coreswift_url),
        ),
        (None, _) => (
            net_key.clone(),
            net_base
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(coreswift_url),
        ),
    };

    // Decrypt the stored personal key with the app's enc:v1 helper (the same path every
    // other BYOK credential uses). A row we cannot decrypt counts as "no key": the
    // ciphertext must never go on the wire.
    let api_key = match enc_key {
        Some(bytes) if !bytes.is_empty() => {
            let stored = String::from_utf8_lossy(&bytes).to_string();
            crate::security::provider_key_crypto::decrypt_for_use(db, &stored, "coreswift")
                .await
                .unwrap_or_default()
        }
        _ => String::new(),
    };

    // Lists: dir first, then net fallback
    let users_list_id = dir_ul.or(net_ul);
    let businesses_list_id = dir_bl.or(net_bl);
    let suppliers_list_id = dir_sl.or(net_sl);

    Ok(CoreSwiftConn {
        tenant_id,
        api_key,
        base_url,
        users_list_id,
        businesses_list_id,
        suppliers_list_id,
    })
}

/// Participant classification → target list id.
#[derive(Clone, Copy, PartialEq)]
pub enum ParticipantType {
    User,
    Business,
    Supplier,
}

impl ParticipantType {
    fn list_id(&self, conn: &CoreSwiftConn) -> Option<Uuid> {
        match self {
            ParticipantType::User => conn.users_list_id,
            ParticipantType::Business => conn.businesses_list_id,
            ParticipantType::Supplier => conn.suppliers_list_id,
        }
    }
    fn tag(&self) -> &'static str {
        match self {
            ParticipantType::User => "directory-user",
            ParticipantType::Business => "directory-business",
            ParticipantType::Supplier => "directory-supplier",
        }
    }
}

/// POST to CoreSwift /api/external/contacts using the personal key.
/// Returns the hub's contact id (201 body `{ id, ... }`).
async fn push_external_contact(
    conn: &CoreSwiftConn,
    body: serde_json::Value,
) -> Result<Uuid, String> {
    if conn.api_key.is_empty() {
        return Err(
            "no CoreSwift personal key is stored for this connection — the external ".to_string()
                + "field-mapping path needs a csk_ key from CoreSwift's Integration Center",
        );
    }
    let resp = HTTP
        .post(format!("{}/api/external/contacts", conn.base_url))
        .header("Authorization", format!("Bearer {}", conn.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("CoreSwift external contact push failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "CoreSwift external contact returned {status}: {text}"
        ));
    }
    let id = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
        .and_then(|s| Uuid::parse_str(&s).ok());
    id.ok_or_else(|| format!("CoreSwift external contact returned {status} without an id: {text}"))
}

/// Build the base external-contact body with list + type tag + session tags.
fn build_contact_body(
    conn: &CoreSwiftConn,
    ptype: ParticipantType,
    extra_tags: &[String],
) -> serde_json::Map<String, serde_json::Value> {
    let mut body = serde_json::Map::new();
    if let Some(lid) = ptype.list_id(conn) {
        body.insert("list_id".into(), serde_json::json!(lid.to_string()));
    }
    let mut tags: Vec<String> = vec![ptype.tag().to_string(), "source:multidirectory".to_string()];
    tags.extend(extra_tags.iter().cloned());
    body.insert("tags".into(), serde_json::json!(tags));
    body.insert("source_app".into(), serde_json::json!("multidirectory"));
    body
}

/// Push a loyalty member (consumer visitor) into CoreSwift CRM.
/// classification = User (customers list) + loyalty tags + loyalty state as custom fields.
pub async fn push_loyalty_member(
    db: &PgPool,
    directory_id: Uuid,
    member_id: Uuid,
) -> Result<(), String> {
    let row = sqlx::query_as::<_, (
        String, // visitor name
        String, // email
        Option<String>, // phone
        i32,    // points_balance
        i32,    // lifetime_points
        Option<String>, // tier name
        chrono::DateTime<chrono::Utc>, // member_since
        Option<chrono::DateTime<chrono::Utc>>, // last_checkin_at
        i32,    // total_checkins
        i32,    // current_streak
        Option<String>, // referral_code
        i32,    // total_referrals
        Option<chrono::NaiveDate>, // birthday
        String, // program name
    )>(
        r#"SELECT
             COALESCE(va.name, ''),
             va.email,
             va.phone,
             m.points_balance,
             m.lifetime_points,
             t.name AS tier_name,
             m.member_since,
             m.last_checkin_at,
             (SELECT COUNT(*) FROM loyalty_checkins c WHERE c.member_id = m.id)::int AS total_checkins,
             m.current_streak,
             m.referral_code,
             m.total_referrals,
             m.birthday,
             p.name AS program_name
           FROM loyalty_members m
           JOIN visitor_accounts va ON va.id = m.visitor_account_id
           JOIN loyalty_programs p ON p.id = m.program_id
           LEFT JOIN loyalty_tiers t ON t.id = m.tier_id
           WHERE m.id = $1"#,
    )
    .bind(member_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error loading loyalty member: {e}"))?
    .ok_or_else(|| format!("Loyalty member {member_id} not found"))?;

    let (
        name,
        email,
        phone,
        points_balance,
        lifetime_points,
        tier_name,
        member_since,
        last_checkin_at,
        total_checkins,
        current_streak,
        referral_code,
        total_referrals,
        birthday,
        program_name,
    ) = row;
    let (first_name, last_name) = split_name(&name);

    let conn = resolve_cs_conn(db, directory_id).await?;

    let mut fields = serde_json::Map::new();
    fields.insert("points_balance".into(), serde_json::json!(points_balance));
    fields.insert("lifetime_points".into(), serde_json::json!(lifetime_points));
    fields.insert("total_checkins".into(), serde_json::json!(total_checkins));
    fields.insert("current_streak".into(), serde_json::json!(current_streak));
    fields.insert("total_referrals".into(), serde_json::json!(total_referrals));
    fields.insert(
        "member_since".into(),
        serde_json::json!(member_since.format("%Y-%m-%d").to_string()),
    );
    if let Some(t) = &tier_name {
        fields.insert("tier".into(), serde_json::json!(t));
    }
    if let Some(lc) = &last_checkin_at {
        fields.insert(
            "last_checkin_at".into(),
            serde_json::json!(lc.format("%Y-%m-%d").to_string()),
        );
    }
    if let Some(rc) = &referral_code {
        fields.insert("referral_code".into(), serde_json::json!(rc));
    }
    if let Some(bd) = &birthday {
        fields.insert(
            "birthday".into(),
            serde_json::json!(bd.format("%Y-%m-%d").to_string()),
        );
    }
    fields.insert("loyalty_program".into(), serde_json::json!(program_name));

    let mut body = build_contact_body(
        &conn,
        ParticipantType::User,
        &["loyalty-member".to_string()],
    );
    body.insert("first_name".into(), serde_json::json!(first_name));
    body.insert("last_name".into(), serde_json::json!(last_name));
    body.insert("email".into(), serde_json::json!(email));
    body.insert("phone".into(), serde_json::json!(phone));
    body.insert("fields".into(), serde_json::json!(fields));

    push_external_contact(&conn, serde_json::Value::Object(body)).await?;
    tracing::info!("[coreswift] Pushed loyalty member {member_id} ({name}) to CoreSwift");
    Ok(())
}

/// Push a business loyalty participant into CoreSwift CRM.
/// classification = Business (businesses list) + `company` + `title` + loyalty-side fields.
pub async fn push_loyalty_business(
    db: &PgPool,
    directory_id: Uuid,
    business_id: Uuid,
) -> Result<(), String> {
    let row = sqlx::query_as::<
        _,
        (
            String,         // business name
            Option<String>, // email
            Option<String>, // phone
            Option<String>, // website
            Option<String>, // city
            Option<String>, // state
            Option<String>, // business_type
            i64,            // deals count
            i64,            // events count
        ),
    >(
        r#"SELECT
             b.name,
             b.email,
             b.phone,
             b.website,
             b.city,
             b.state,
             b.business_type,
             (SELECT COUNT(*) FROM deals d WHERE d.business_id = b.id) AS deals_count,
             (SELECT COUNT(*) FROM community_events e WHERE e.business_id = b.id) AS events_count
           FROM businesses b
           WHERE b.id = $1"#,
    )
    .bind(business_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error loading business: {e}"))?
    .ok_or_else(|| format!("Business {business_id} not found"))?;

    let (name, email, phone, website, city, state, business_type, deals_count, events_count) = row;

    let conn = resolve_cs_conn(db, directory_id).await?;

    let mut fields = serde_json::Map::new();
    fields.insert("loyalty_deals_count".into(), serde_json::json!(deals_count));
    fields.insert(
        "loyalty_events_count".into(),
        serde_json::json!(events_count),
    );
    fields.insert("loyalty_participant".into(), serde_json::json!("business"));
    if let Some(bt) = &business_type {
        fields.insert("business_type".into(), serde_json::json!(bt));
    }

    let mut body = build_contact_body(
        &conn,
        ParticipantType::Business,
        &["loyalty-business".to_string()],
    );
    body.insert("first_name".into(), serde_json::json!(name));
    body.insert("last_name".into(), serde_json::json!(""));
    body.insert("email".into(), serde_json::json!(email));
    body.insert("phone".into(), serde_json::json!(phone));
    body.insert("company".into(), serde_json::json!(name));
    body.insert("title".into(), serde_json::json!("Business"));
    body.insert("city".into(), serde_json::json!(city));
    body.insert("state".into(), serde_json::json!(state));
    body.insert("notes".into(), serde_json::json!(website));
    body.insert("fields".into(), serde_json::json!(fields));

    push_external_contact(&conn, serde_json::Value::Object(body)).await?;
    tracing::info!("[coreswift] Pushed loyalty business {business_id} ({name}) to CoreSwift");
    Ok(())
}

/// Split a full name into (first_name, last_name).
fn split_name(full: &str) -> (String, String) {
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return ("Loyalty".to_string(), "Member".to_string());
    }
    let mut parts = trimmed.split_whitespace();
    let first = parts.next().unwrap_or("").to_string();
    let last = parts.collect::<Vec<_>>().join(" ");
    (first, last)
}

// ─────────────────────────────────────────────────────────────────────────────
// Inbound lead push — fleet standard R2: data flows DOWNWARD into CoreSwift
// ─────────────────────────────────────────────────────────────────────────────
//
// MultiDirectory is CAPTURE software: a directory enquiry, a visitor booking
// request or an enrichment-created lead IS a lead. CoreSwift is the hub and the
// single home for every lead, so the real capture events push their lead into the
// tenant's CoreSwift account through the hub's external API
// (`POST /api/external/contacts`, `Authorization: Bearer csk_…`).
//
// This reuses THIS module's connection + HTTP layer ([`push_external_contact`]) —
// there is deliberately no second CoreSwift client in the app.
//
// Connection resolution (tenant BYOK first, never env-only):
//   1. `provider_keys` row for provider `coreswift` belonging to the tenant that
//      owns the capture (the row the Integration Center writes): explicit tenant
//      → directory owner → parent-network owner → MultiDirectory platform tenant.
//   2. the pre-existing directory/network-level key
//      (`directories.coreswift_personal_key_encrypted` via [`resolve_cs_conn`]) so
//      loyalty / claim / newsletter pushes keep working unchanged.
//   3. nothing → the capture SUCCEEDS locally and the push is skipped quietly
//      (log lines only, never a user-visible error).
//
// Base-URL resolution order (fleet standard, never hardcode-only):
//   provider_keys.base_url → integration_provider_presets.base_url → CORESWIFT_URL → default.

/// A lead captured by MultiDirectory, on its way to the CoreSwift hub.
#[derive(Debug, Clone, Default)]
pub struct LeadPayload {
    pub email: Option<String>,
    pub phone: Option<String>,
    /// Full name — split into first/last when those are not given.
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
    /// Hub list to put the contact in (optional).
    pub list_id: Option<Uuid>,
    /// Hub tags (idempotent; auto-created on the hub side).
    pub tags: Vec<String>,
    /// Extra key/values — the hub auto-provisions them as per-tenant custom fields.
    pub fields: serde_json::Map<String, Value>,
}

impl LeadPayload {
    /// The hub rejects a body with none of first_name / email / phone, so a lead
    /// with no way to be identified is skipped rather than sent to fail.
    fn has_identity(&self) -> bool {
        fn nonempty(v: &Option<String>) -> bool {
            v.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false)
        }
        nonempty(&self.email) || nonempty(&self.phone) || nonempty(&self.name)
    }
}

/// Step 2 of the base-URL resolution order.
async fn preset_base_url(db: &PgPool, provider: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT base_url FROM integration_provider_presets \
         WHERE key = $1 AND is_active = true AND base_url <> '' LIMIT 1",
    )
    .bind(provider)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// The typed CoreSwift lists already recorded for a directory (informational).
async fn directory_lists(
    db: &PgPool,
    directory_id: Option<Uuid>,
) -> (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>) {
    let Some(dir) = directory_id else {
        return (None, None, None, None);
    };
    sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>)>(
        "SELECT coreswift_list_id_users, coreswift_list_id_businesses, \
                coreswift_list_id_suppliers, coreswift_tenant_id \
         FROM directories WHERE id = $1",
    )
    .bind(dir)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .unwrap_or((None, None, None, None))
}

/// Tenant candidates for a capture, in priority order.
async fn lead_tenant_candidates(
    db: &PgPool,
    tenant_id: Option<Uuid>,
    directory_id: Option<Uuid>,
) -> Vec<Uuid> {
    let mut out: Vec<Uuid> = Vec::new();
    fn add(cand: Option<Uuid>, out: &mut Vec<Uuid>) {
        if let Some(c) = cand {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }

    add(tenant_id, &mut out);

    if let Some(dir) = directory_id {
        let row = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>)>(
            r#"SELECT u.tenant_id, nu.tenant_id
               FROM directories d
               LEFT JOIN users u ON u.id = d.owner_id
               LEFT JOIN networks n ON n.id = d.network_id
               LEFT JOIN users nu ON nu.id = n.owner_id
               WHERE d.id = $1"#,
        )
        .bind(dir)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
        if let Some((owner_tid, net_tid)) = row {
            add(owner_tid, &mut out);
            add(net_tid, &mut out);
        }
    }

    // MultiDirectory is platform-operated: the Admin Panel Integration Center
    // writes provider_keys under the platform tenant (the same place the
    // google_places / mailgun rows live), so it is a real tenant candidate.
    if let Ok(platform) = Uuid::parse_str("00000000-0000-0000-0000-000000000001") {
        add(Some(platform), &mut out);
    }
    add(Some(Uuid::nil()), &mut out);
    out
}

/// Resolve the CoreSwift hub connection a capture event must push through.
/// `Ok(None)` = the tenant has not connected CoreSwift → capture proceeds locally.
///
/// Order: the NATIVE per-directory / per-network link first (David's model — the linking
/// unit is a network or a standalone directory), then the platform-level BYOK key held for
/// the caller's MD tenant.
pub async fn resolve_lead_conn(
    db: &PgPool,
    tenant_id: Option<Uuid>,
    directory_id: Option<Uuid>,
) -> Result<Option<CoreSwiftConn>, String> {
    if let Some(dir) = directory_id {
        if let Ok(conn) = resolve_cs_conn(db, dir).await {
            return Ok(Some(conn));
        }
    }

    for cand in lead_tenant_candidates(db, tenant_id, directory_id).await {
        let row = sqlx::query_as::<_, (String, Option<String>)>(
            r#"SELECT api_key, base_url
               FROM provider_keys
               WHERE tenant_id = $1 AND provider = 'coreswift' AND is_active = true
               ORDER BY is_default DESC, updated_at DESC
               LIMIT 1"#,
        )
        .bind(cand)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("DB error resolving CoreSwift key: {e}"))?;

        if let Some((stored_key, base_url)) = row {
            // The credential is enc:v1 ciphertext at rest — decrypt with the env-only master key
            // before it goes on the wire. A row we cannot decrypt is treated as not connected
            // (never push ciphertext to the hub) and the next candidate is tried.
            let Some(api_key) =
                crate::security::provider_key_crypto::decrypt_for_use(db, &stored_key, "coreswift")
                    .await
            else {
                continue;
            };
            let base = base_url
                .filter(|s| !s.trim().is_empty())
                .or(preset_base_url(db, "coreswift").await)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(coreswift_url);
            let (ul, bl, sl, hub_tenant) = directory_lists(db, directory_id).await;
            return Ok(Some(CoreSwiftConn {
                tenant_id: hub_tenant.unwrap_or(cand),
                api_key,
                base_url: base,
                users_list_id: ul,
                businesses_list_id: bl,
                suppliers_list_id: sl,
            }));
        }
    }

    // Legacy path: directory/network-level key already on the directories row.
    if let Some(dir) = directory_id {
        if let Ok(conn) = resolve_cs_conn(db, dir).await {
            return Ok(Some(conn));
        }
    }

    Ok(None)
}

/// `GET {hub}/api/external/lists` — the tenant's CoreSwift lists (picker proxy).
pub async fn hub_lists(conn: &CoreSwiftConn) -> Result<Value, String> {
    if conn.api_key.is_empty() {
        return Err(
            "this connection has no personal key — the list picker needs a csk_ key from \
             CoreSwift's Integration Center"
                .to_string(),
        );
    }
    let resp = HTTP
        .get(format!("{}/api/external/lists", conn.base_url))
        .header("Authorization", format!("Bearer {}", conn.api_key))
        .send()
        .await
        .map_err(|e| format!("CoreSwift lists request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("CoreSwift lists returned {status}: {text}"));
    }
    serde_json::from_str::<Value>(&text).map_err(|e| format!("CoreSwift lists: bad JSON: {e}"))
}

/// Build the hub contact body from a captured lead.
fn lead_body(conn: &CoreSwiftConn, lead: &LeadPayload) -> Value {
    let mut body = serde_json::Map::new();

    let explicit_first = lead
        .first_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let (first, last) = match explicit_first {
        Some(f) => (
            f,
            lead.last_name
                .clone()
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        None => match lead
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(full) => split_name(full),
            None => ("Lead".to_string(), String::new()),
        },
    };
    body.insert("first_name".into(), json!(first));
    body.insert("last_name".into(), json!(last));

    let mut put = |k: &str, v: &Option<String>| {
        if let Some(val) = v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            body.insert(k.to_string(), json!(val));
        }
    };
    put("email", &lead.email);
    put("phone", &lead.phone);
    put("company", &lead.company);
    put("title", &lead.title);
    put("city", &lead.city);
    put("state", &lead.state);
    put("postal_code", &lead.postal_code);
    put("address_line1", &lead.address_line1);
    put("notes", &lead.notes);

    if let Some(lid) = lead.list_id.or(conn.users_list_id) {
        body.insert("list_id".into(), json!(lid.to_string()));
    }

    let mut tags: Vec<String> = vec!["source:multidirectory".to_string()];
    for t in &lead.tags {
        if !t.trim().is_empty() && !tags.contains(t) {
            tags.push(t.trim().to_string());
        }
    }
    body.insert("tags".into(), json!(tags));
    body.insert("source_app".into(), json!("multidirectory"));
    if !lead.fields.is_empty() {
        body.insert("fields".into(), Value::Object(lead.fields.clone()));
    }

    Value::Object(body)
}

/// THE INBOUND PATH — push one captured lead into the tenant's CoreSwift account.
///
/// Two transports, one contract (the contact always lands in the linked tenant):
///   * a connection carrying a `csk_` personal key  → `POST /api/external/contacts`
///     (Bearer key; the hub stores `fields` as per-tenant data points — the field-mapping path
///     card B68 builds on);
///   * a connection with only the tenant + lists linked (no personal key yet) →
///     `POST /api/internal/contacts` + list membership, the same internal transport the
///     claim and newsletter pushes already use.
///
/// `Ok(Some(contact_id))` = pushed (a contact row exists on the hub)
/// `Ok(None)`             = nothing to push / no CoreSwift link for this capture
///                          (quiet skip — the capture already succeeded locally)
/// `Err(_)`               = a real hub/gateway failure worth surfacing in another layer
pub async fn push_lead_to_coreswift(
    db: &PgPool,
    tenant_id: Option<Uuid>,
    directory_id: Option<Uuid>,
    lead: LeadPayload,
) -> Result<Option<Uuid>, String> {
    if !lead.has_identity() {
        tracing::debug!("[coreswift] lead push skipped: no email/phone/name to identify it");
        return Ok(None);
    }

    let Some(conn) = resolve_lead_conn(db, tenant_id, directory_id).await? else {
        tracing::debug!(
            "[coreswift] lead push skipped: CoreSwift not connected (tenant {:?}, directory {:?})",
            tenant_id,
            directory_id
        );
        return Ok(None);
    };

    let contact_id = if conn.api_key.is_empty() {
        // No personal key on this link: carry the identity across with the internal
        // transport so the contact still lands in the right tenant and list.
        let id = push_contact_internal(&conn, &lead).await?;
        tracing::info!(
            "[coreswift] lead pushed via the internal transport (tenant {}, list {:?})",
            conn.tenant_id,
            lead.list_id.or(conn.users_list_id)
        );
        id
    } else {
        let body = lead_body(&conn, &lead);
        push_external_contact(&conn, body).await?
    };

    tracing::info!(
        "[coreswift] lead pushed to CoreSwift hub ({}) for tenant {:?} / directory {:?} as contact {}",
        conn.base_url,
        tenant_id,
        directory_id,
        contact_id
    );
    Ok(Some(contact_id))
}

/// Internal transport: create the contact (tenant from the LINK, never from the caller) and
/// put it in the target list, then apply the tags. Returns the hub contact id.
///
/// The internal contacts endpoint stores no custom fields, so the answer set travels in
/// `notes` for now — an interim mapping until the questionnaire→data-point mapping (card B68)
/// writes real contact fields through the external API.
async fn push_contact_internal(conn: &CoreSwiftConn, lead: &LeadPayload) -> Result<Uuid, String> {
    let base = coreswift_url();
    let key = internal_key();

    let explicit_first = lead
        .first_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let (first, last) = match explicit_first {
        Some(f) => (
            f,
            lead.last_name
                .clone()
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        None => match lead
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(full) => split_name(full),
            None => ("Lead".to_string(), String::new()),
        },
    };

    let mut notes = lead.notes.clone().unwrap_or_default();
    if !lead.fields.is_empty() {
        let mut pairs: Vec<String> = lead
            .fields
            .iter()
            .filter_map(|(k, v)| match v {
                Value::Null => None,
                Value::String(s) if s.trim().is_empty() => None,
                Value::String(s) => Some(format!("{k}={s}")),
                other => Some(format!("{k}={other}")),
            })
            .collect();
        pairs.sort();
        if !pairs.is_empty() {
            if !notes.is_empty() {
                notes.push_str(" | ");
            }
            notes.push_str(&pairs.join("; "));
        }
    }

    let mut body = json!({
        "tenant_id": conn.tenant_id.to_string(),
        "first_name": first,
        "last_name": last,
        "notes": notes,
    });
    if let Some(email) = lead.email.as_deref().filter(|s| !s.trim().is_empty()) {
        body["email"] = json!(email);
    }
    if let Some(phone) = lead.phone.as_deref().filter(|s| !s.trim().is_empty()) {
        body["phone"] = json!(phone);
    }

    let resp = HTTP
        .post(format!("{base}/api/internal/contacts"))
        .header("x-internal-key", &key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("CoreSwift internal contact create failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "CoreSwift internal contact returned {status}: {text}"
        ));
    }
    let contact_id = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
        .and_then(|s| Uuid::parse_str(&s).ok())
        .ok_or_else(|| {
            format!("CoreSwift internal contact returned {status} without an id: {text}")
        })?;

    if let Some(list_id) = lead.list_id.or(conn.users_list_id) {
        let resp = HTTP
            .post(format!("{base}/api/internal/lists/{list_id}/members"))
            .header("x-internal-key", &key)
            .json(&json!({
                "tenant_id": conn.tenant_id.to_string(),
                "contact_id": contact_id.to_string(),
            }))
            .send()
            .await
            .map_err(|e| format!("CoreSwift list add failed: {e}"))?;
        let ls = resp.status();
        if !ls.is_success() {
            let b = resp.text().await.unwrap_or_default();
            tracing::warn!("[coreswift] list {list_id} add returned {ls}: {b}");
        }
    }

    // Tags: find-or-create on the hub, then assign. Bounded so one lead cannot fan out into
    // an unbounded number of round trips.
    for tag in lead.tags.iter().take(8) {
        let name = tag.trim();
        if name.is_empty() {
            continue;
        }
        let tag_id = match find_tag_by_name(conn.tenant_id, name).await {
            Ok(Some(id)) => Some(id),
            Ok(None) => create_tag_internal(conn.tenant_id, name, "#0ea5e9")
                .await
                .ok(),
            Err(e) => {
                tracing::warn!("[coreswift] tag lookup '{name}' failed: {e}");
                None
            }
        };
        if let Some(id) = tag_id {
            if let Err(e) = assign_contact_tag(conn.tenant_id, contact_id, id).await {
                tracing::warn!("[coreswift] tag assign '{name}' failed: {e}");
            }
        }
    }

    Ok(contact_id)
}

// ─────────────────────────────────────────────────────────────────────────────
// NATIVE LINK — the Admin Panel's connect / test / status / disconnect surface.
//
// David's model: the linking unit is a NETWORK or a STANDALONE DIRECTORY. A directory in a
// connected network inherits that network's account and carries its own per-city lists; a
// directory running on its own gets its own account. The link lives in the row itself —
// tenant id, base URL, an OPTIONAL personal key (encrypted at rest with the app's enc:v1
// helper) and the six typed list ids. Nothing is server-wide, nothing is hardcoded, and an
// unconfigured link is a quiet skip: never a panic, never a fake success.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkScope {
    Network,
    Directory,
}

impl LinkScope {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "network" | "networks" => Some(Self::Network),
            "directory" | "directories" => Some(Self::Directory),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Directory => "directory",
        }
    }

    fn table(&self) -> &'static str {
        match self {
            Self::Network => "networks",
            Self::Directory => "directories",
        }
    }
}

/// The six list columns plus the tenant/base/key state, selected identically from both tables.
const LINK_COLUMNS: &str = "coreswift_tenant_id, coreswift_base_url, coreswift_key_prefix, \
     (coreswift_personal_key_encrypted IS NOT NULL \
      AND length(coreswift_personal_key_encrypted) > 0) AS key_configured, \
     coreswift_list_id_users, coreswift_list_id_businesses, coreswift_list_id_suppliers, \
     coreswift_list_id_sponsors, coreswift_list_id_claimed, coreswift_list_id_newsletter";

type LinkRow = (
    Option<Uuid>,
    Option<String>,
    Option<String>,
    bool,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
);

fn uuid_of(v: &Value, key: &str) -> Option<Uuid> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn link_json(scope: LinkScope, id: Uuid, name: &str, row: &LinkRow) -> Value {
    json!({
        "scope": scope.as_str(),
        "id": id,
        "name": name,
        "linked_here": row.0.is_some(),
        "tenant_id": row.0,
        "base_url": row.1,
        "key_prefix": row.2,
        "key_configured": row.3,
        "lists": {
            "users": row.4,
            "businesses": row.5,
            "suppliers": row.6,
            "sponsors": row.7,
            "claimed": row.8,
            "newsletter": row.9,
        },
    })
}

async fn read_link_row(
    db: &PgPool,
    scope: LinkScope,
    id: Uuid,
) -> Result<Option<(String, LinkRow)>, String> {
    let sql = format!(
        "SELECT name, {LINK_COLUMNS} FROM {} WHERE id = $1",
        scope.table()
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("DB error reading the CoreSwift link: {e}"))?;

    // Read by column NAME: the link row is wider than sqlx's tuple Decode set.
    let Some(row) = row else { return Ok(None) };
    let name: String = row
        .try_get("name")
        .map_err(|e| format!("DB error reading name: {e}"))?;
    let base_url: Option<String> = row
        .try_get("coreswift_base_url")
        .map_err(|e| format!("DB error reading coreswift_base_url: {e}"))?;
    let key_prefix: Option<String> = row
        .try_get("coreswift_key_prefix")
        .map_err(|e| format!("DB error reading coreswift_key_prefix: {e}"))?;
    let key_configured: bool = row
        .try_get("key_configured")
        .map_err(|e| format!("DB error reading key_configured: {e}"))?;
    let uuid_col = |name: &str| -> Result<Option<Uuid>, String> {
        row.try_get::<Option<Uuid>, _>(name)
            .map_err(|e| format!("DB error reading {name}: {e}"))
    };

    Ok(Some((
        name,
        (
            uuid_col("coreswift_tenant_id")?,
            base_url,
            key_prefix,
            key_configured,
            uuid_col("coreswift_list_id_users")?,
            uuid_col("coreswift_list_id_businesses")?,
            uuid_col("coreswift_list_id_suppliers")?,
            uuid_col("coreswift_list_id_sponsors")?,
            uuid_col("coreswift_list_id_claimed")?,
            uuid_col("coreswift_list_id_newsletter")?,
        ),
    )))
}

async fn parent_network_id(db: &PgPool, directory_id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar::<_, Option<Uuid>>("SELECT network_id FROM directories WHERE id = $1")
        .bind(directory_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .flatten()
}

/// Read the connection state of one network or directory (what the Admin Panel's card shows).
/// `Ok(None)` = no such row. A directory reports both its own link and the network it inherits.
pub async fn link_status(db: &PgPool, scope: LinkScope, id: Uuid) -> Result<Option<Value>, String> {
    let Some((name, row)) = read_link_row(db, scope, id).await? else {
        return Ok(None);
    };
    let mut out = link_json(scope, id, &name, &row);

    if scope == LinkScope::Directory {
        if let Some(nid) = parent_network_id(db, id).await {
            if let Some((net_name, net_row)) = read_link_row(db, LinkScope::Network, nid).await? {
                out["effective_tenant_id"] = match row.0 {
                    Some(t) => json!(t),
                    None => json!(net_row.0),
                };
                out["inherits_from"] = link_json(LinkScope::Network, nid, &net_name, &net_row);
                return Ok(Some(out));
            }
        }
        out["effective_tenant_id"] = json!(row.0);
    } else {
        out["effective_tenant_id"] = json!(row.0);
    }
    Ok(Some(out))
}

/// Connect (or update) a link. Lists that are not supplied keep their stored value, so the
/// panel can save the tenant first and pick lists afterwards. `Ok(None)` = no such row.
pub async fn save_link(
    db: &PgPool,
    scope: LinkScope,
    id: Uuid,
    tenant_id: Uuid,
    base_url: Option<String>,
    personal_key: Option<String>,
    lists: &Value,
) -> Result<Option<Value>, String> {
    let plain_key = personal_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());

    let stored_key: Option<Vec<u8>> = match plain_key {
        Some(k) => Some(
            crate::security::provider_key_crypto::encrypt_for_storage(db, k)
                .await
                .map_err(|e| format!("could not encrypt the CoreSwift key: {e}"))?
                .into_bytes(),
        ),
        None => None,
    };
    let prefix: Option<String> = plain_key.map(|k| k.chars().take(12).collect());

    let table = scope.table();
    let sql = format!(
        "UPDATE {table} SET \
           coreswift_tenant_id = $2, \
           coreswift_base_url = COALESCE($3, coreswift_base_url), \
           coreswift_personal_key_encrypted = COALESCE($4::bytea, coreswift_personal_key_encrypted), \
           coreswift_key_prefix = COALESCE($5, coreswift_key_prefix), \
           coreswift_list_id_users = COALESCE($6::uuid, coreswift_list_id_users), \
           coreswift_list_id_businesses = COALESCE($7::uuid, coreswift_list_id_businesses), \
           coreswift_list_id_suppliers = COALESCE($8::uuid, coreswift_list_id_suppliers), \
           coreswift_list_id_sponsors = COALESCE($9::uuid, coreswift_list_id_sponsors), \
           coreswift_list_id_claimed = COALESCE($10::uuid, coreswift_list_id_claimed), \
           coreswift_list_id_newsletter = COALESCE($11::uuid, coreswift_list_id_newsletter) \
         WHERE id = $1"
    );

    let res = sqlx::query(&sql)
        .bind(id)
        .bind(tenant_id)
        .bind(base_url.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(&stored_key)
        .bind(&prefix)
        .bind(uuid_of(lists, "users"))
        .bind(uuid_of(lists, "businesses"))
        .bind(uuid_of(lists, "suppliers"))
        .bind(uuid_of(lists, "sponsors"))
        .bind(uuid_of(lists, "claimed"))
        .bind(uuid_of(lists, "newsletter"))
        .execute(db)
        .await
        .map_err(|e| format!("DB error saving the CoreSwift link: {e}"))?;

    if res.rows_affected() == 0 {
        return Ok(None);
    }
    tracing::info!(
        "[coreswift] link saved for {} {id} (tenant {tenant_id}, key {})",
        scope.as_str(),
        if stored_key.is_some() {
            "set"
        } else {
            "unchanged"
        }
    );
    link_status(db, scope, id).await
}

/// Disconnect: clear every link column on this row. A directory inside a connected network
/// falls back to inheriting the network again — that is the documented behaviour.
pub async fn clear_link(db: &PgPool, scope: LinkScope, id: Uuid) -> Result<bool, String> {
    let sql = format!(
        "UPDATE {} SET \
           coreswift_tenant_id = NULL, coreswift_base_url = NULL, \
           coreswift_personal_key_encrypted = NULL, coreswift_key_prefix = NULL, \
           coreswift_list_id_users = NULL, coreswift_list_id_businesses = NULL, \
           coreswift_list_id_suppliers = NULL, coreswift_list_id_sponsors = NULL, \
           coreswift_list_id_claimed = NULL, coreswift_list_id_newsletter = NULL \
         WHERE id = $1",
        scope.table()
    );
    let res = sqlx::query(&sql)
        .bind(id)
        .execute(db)
        .await
        .map_err(|e| format!("DB error clearing the CoreSwift link: {e}"))?;
    tracing::info!("[coreswift] link cleared for {} {id}", scope.as_str());
    Ok(res.rows_affected() > 0)
}

/// The effective connection of one link, network-level included.
pub async fn conn_for_link(
    db: &PgPool,
    scope: LinkScope,
    id: Uuid,
) -> Result<Option<CoreSwiftConn>, String> {
    match scope {
        LinkScope::Directory => Ok(resolve_cs_conn(db, id).await.ok()),
        LinkScope::Network => {
            let row = sqlx::query_as::<
                _,
                (
                    Option<Uuid>,
                    Option<String>,
                    Option<Vec<u8>>,
                    Option<Uuid>,
                    Option<Uuid>,
                    Option<Uuid>,
                ),
            >(
                "SELECT coreswift_tenant_id, coreswift_base_url, \
                        coreswift_personal_key_encrypted, coreswift_list_id_users, \
                        coreswift_list_id_businesses, coreswift_list_id_suppliers \
                 FROM networks WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(|e| format!("DB error reading the CoreSwift connection: {e}"))?;

            let Some((tenant, base, key, ul, bl, sl)) = row else {
                return Ok(None);
            };
            let Some(tenant_id) = tenant else {
                return Ok(None);
            };
            let api_key = match key {
                Some(bytes) if !bytes.is_empty() => {
                    let stored = String::from_utf8_lossy(&bytes).to_string();
                    crate::security::provider_key_crypto::decrypt_for_use(db, &stored, "coreswift")
                        .await
                        .unwrap_or_default()
                }
                _ => String::new(),
            };
            let base_url = match base.filter(|s| !s.is_empty()) {
                Some(b) => b,
                None => match preset_base_url(db, "coreswift").await {
                    Some(b) => b,
                    None => coreswift_url(),
                },
            };
            Ok(Some(CoreSwiftConn {
                tenant_id,
                api_key,
                base_url,
                users_list_id: ul,
                businesses_list_id: bl,
                suppliers_list_id: sl,
            }))
        }
    }
}

/// A REAL probe: personal-key connections list their lists through the external API; a
/// tenant-only link proves reachability + that the tenant exists through the internal API
/// (the same one every push uses). Never reports `ok: true` for an unreachable hub.
pub async fn probe_conn(conn: &CoreSwiftConn) -> Result<Value, String> {
    let base = conn.base_url.clone();
    if !conn.api_key.is_empty() {
        return Ok(match hub_lists(conn).await {
            Ok(lists) => json!({
                "ok": true,
                "transport": "personal-key",
                "base_url": base,
                "lists": lists.get("lists").and_then(|l| l.as_array()).map(|a| a.len()).unwrap_or(0),
                "detail": "CoreSwift accepted the personal key.",
            }),
            Err(e) => json!({
                "ok": false,
                "transport": "personal-key",
                "base_url": base,
                "detail": e,
            }),
        });
    }

    let resp = HTTP
        .post(format!("{}/api/internal/tags/list", coreswift_url()))
        .header("x-internal-key", internal_key())
        .json(&json!({ "tenant_id": conn.tenant_id.to_string() }))
        .send()
        .await
        .map_err(|e| format!("CoreSwift probe failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Ok(json!({
        "ok": status.is_success(),
        "transport": "internal",
        "base_url": base,
        "status": status.as_u16(),
        "detail": if status.is_success() {
            "CoreSwift answered for this tenant; the fleet internal key is accepted.".to_string()
        } else {
            text.chars().take(300).collect::<String>()
        },
    }))
}

/// The CoreSwift tenant a directory is linked to — its own, or its network's.
pub async fn resolve_directory_tenant(
    db: &PgPool,
    directory_id: Option<Uuid>,
    directory_slug: Option<&str>,
) -> Option<Uuid> {
    let row = sqlx::query_as::<_, (Option<Uuid>,)>(
        r#"SELECT COALESCE(d.coreswift_tenant_id, n.coreswift_tenant_id)
           FROM directories d
           LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1 OR d.slug = $2
           ORDER BY (d.id = $1) DESC
           LIMIT 1"#,
    )
    .bind(directory_id)
    .bind(directory_slug)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    row.and_then(|(t,)| t)
}

/// The platform's one connected network — the fallback for captures that carry no directory
/// (a B2B supplier registering on the community site, for example). Deliberately refuses to
/// guess once more than one network is connected: two accounts means the caller must say
/// which directory the capture belongs to.
pub async fn default_network_tenant(db: &PgPool) -> Option<Uuid> {
    let rows: Vec<Option<Uuid>> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT coreswift_tenant_id FROM networks WHERE coreswift_tenant_id IS NOT NULL LIMIT 2",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let found: Vec<Uuid> = rows.into_iter().flatten().collect();
    match found.len() {
        1 => found.first().copied(),
        0 => None,
        _ => {
            tracing::warn!(
                "[coreswift] {} networks are connected — refusing to guess which account a \
                 directory-less capture belongs to",
                found.len()
            );
            None
        }
    }
}

/// The tenant a capture event pushes into: the directory's link, else the one connected
/// network. `None` = nothing is linked → the caller skips the push quietly.
pub async fn capture_tenant(
    db: &PgPool,
    directory_id: Option<Uuid>,
    directory_slug: Option<&str>,
) -> Option<Uuid> {
    match resolve_directory_tenant(db, directory_id, directory_slug).await {
        Some(t) => Some(t),
        None => default_network_tenant(db).await,
    }
}

/// The list a captured audience belongs in, resolved from the LINK (directory, else its
/// network, else the one connected network) — never from a hardcoded id.
pub async fn audience_list(
    db: &PgPool,
    directory_slug: Option<&str>,
    list_type: Option<&str>,
) -> Option<Uuid> {
    let kind = list_type
        .unwrap_or("subscribers")
        .trim()
        .to_ascii_lowercase();

    let row: Option<(
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
    )> = match directory_slug.map(str::trim).filter(|s| !s.is_empty()) {
        Some(slug) => sqlx::query_as(
            r#"SELECT COALESCE(d.coreswift_list_id_users, n.coreswift_list_id_users),
                      COALESCE(d.coreswift_list_id_businesses, n.coreswift_list_id_businesses),
                      COALESCE(d.coreswift_list_id_suppliers, n.coreswift_list_id_suppliers),
                      COALESCE(d.coreswift_list_id_newsletter, n.coreswift_list_id_newsletter),
                      COALESCE(d.coreswift_list_id_claimed, n.coreswift_list_id_claimed),
                      COALESCE(d.coreswift_list_id_sponsors, n.coreswift_list_id_sponsors)
               FROM directories d
               LEFT JOIN networks n ON n.id = d.network_id
               WHERE d.slug = $1
               LIMIT 1"#,
        )
        .bind(slug)
        .fetch_optional(db)
        .await
        .ok()
        .flatten(),
        None => {
            let rows: Vec<(
                Option<Uuid>,
                Option<Uuid>,
                Option<Uuid>,
                Option<Uuid>,
                Option<Uuid>,
                Option<Uuid>,
            )> = sqlx::query_as(
                "SELECT coreswift_list_id_users, coreswift_list_id_businesses, \
                            coreswift_list_id_suppliers, coreswift_list_id_newsletter, \
                            coreswift_list_id_claimed, coreswift_list_id_sponsors \
                     FROM networks WHERE coreswift_tenant_id IS NOT NULL LIMIT 2",
            )
            .fetch_all(db)
            .await
            .unwrap_or_default();
            if rows.len() == 1 {
                rows.into_iter().next()
            } else {
                None
            }
        }
    };

    let (users, businesses, suppliers, newsletter, claimed, sponsors) = row?;
    match kind.as_str() {
        "businesses" | "business" => businesses.or(claimed),
        "suppliers" | "supplier" => suppliers,
        "sponsors" | "sponsor" => sponsors,
        "subscribers" | "subscriber" | "newsletter" => newsletter.or(users),
        _ => users,
    }
}
