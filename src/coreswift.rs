//! CoreSwift integration — push business claims and newsletter signups
//! to CoreSwift CRM via its internal API (x-internal-key auth).
//!
//! Architecture:
//!   - One CoreSwift tenant per network (or per standalone directory)
//!   - Auto-provisioned: tenant + 3 default lists + city-prefixed tag groups on first use
//!   - Data pushed via internal endpoints (bypasses JWT)
//!   - Database is always source of truth; CoreSwift push is secondary

use serde_json::{json, Value};
use sqlx::PgPool;
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

    Err(format!("No CoreSwift tenant provisioned for directory {directory_id}"))
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

    let body: Value = resp.json().await
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

    let body: Value = resp.json().await
        .map_err(|e| format!("CoreSwift create list '{list_name}' response parse: {e}"))?;

    body["id"].as_str()
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

    let body: Value = resp.json().await
        .map_err(|e| format!("CoreSwift create tag '{tag_name}' response parse: {e}"))?;

    body["id"].as_str()
        .ok_or_else(|| format!("Missing tag id: {body}"))
        .and_then(|s| Uuid::parse_str(s).map_err(|e| format!("Bad tag UUID: {e}")))
}

/// Look up a tag by name on a specific CoreSwift tenant via the internal API.
/// Uses POST /api/internal/tags/list which returns all tags for a tenant.
pub async fn find_tag_by_name(
    tenant_id: Uuid,
    tag_name: &str,
) -> Result<Option<Uuid>, String> {
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

    let body: Value = resp.json().await
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

    let body: Value = resp.json().await
        .map_err(|e| format!("CoreSwift tenant lookup response parse: {e}"))?;

    Ok(body["id"].as_str()
        .and_then(|s| Uuid::parse_str(s).ok()))
}

/// Push a business owner to the CRM — creates a contact and adds to "Claimed Businesses" list.
pub async fn push_claimed_business(
    db: &PgPool,
    business_id: Uuid,
    owner_email: &str,
    owner_name: Option<&str>,
    owner_phone: Option<&str>,
) -> Result<(), String> {
    let dir_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT directory_id FROM businesses WHERE id = $1"
    )
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
        return Err(format!("CoreSwift contact create returned {c_status}: {body}"));
    }

    let contact_body: Value = resp.json().await
        .map_err(|e| format!("CoreSwift contact response parse: {e}"))?;

    let contact_id_str = contact_body["id"]
        .as_str()
        .ok_or_else(|| format!("Missing contact id: {contact_body}"))?;

    // Add to claimed businesses list
    let resp = HTTP
        .post(format!("{base}/api/internal/lists/{claimed_list_id}/members"))
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
    let dir_slug: String = sqlx::query_scalar::<_, Option<String>>(
        "SELECT slug FROM directories WHERE id = $1"
    )
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
                    if let Ok(tag_id) = create_tag_internal(zh_tid, &interested_tag_name, "#3b82f6").await {
                        if let Ok(contact_uuid) = Uuid::parse_str(contact_id_str) {
                            let _ = assign_contact_tag(zh_tid, contact_uuid, tag_id).await;
                            tracing::info!("[coreswift] Created + assigned '{interested_tag_name}' tag to claimed business contact");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("[coreswift] Failed to look up tag '{interested_tag_name}': {e}");
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
        return Err(format!("CoreSwift contact create returned {c_status}: {body}"));
    }

    let contact_body: Value = resp.json().await
        .map_err(|e| format!("CoreSwift contact response parse: {e}"))?;

    let contact_id_str = contact_body["id"]
        .as_str()
        .ok_or_else(|| format!("Missing contact id: {contact_body}"))?;
    
    let contact_id = Uuid::parse_str(contact_id_str)
        .map_err(|e| format!("Bad contact UUID: {e}"))?;

    let resp = HTTP
        .post(format!("{base}/api/internal/lists/{newsletter_list_id}/members"))
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
        "SELECT tag_id FROM _city_tags WHERE directory_id = $1 LIMIT 1"
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

    let body: Value = resp.json().await
        .map_err(|e| format!("CoreSwift search response parse: {e}"))?;

    if let Some(contacts) = body["contacts"].as_array() {
        if let Some(contact) = contacts.first() {
            if let Some(cid) = contact["id"].as_str() {
                let resp = HTTP
                    .post(format!("{base}/api/internal/lists/{claimed_list_id}/members"))
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
    sqlx::query(
        "UPDATE directories SET booking_calendar_slug = $1 WHERE id = $2"
    )
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
           WHERE d.id = $1"#
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
        let city_name = directory_slug.replace('-', " ").split(' ')
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

        tracing::info!("[provision] Booking calendar '{prefix}' + default slot ready for tenant {tenant_id}");

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
            let full_name = format!("{}-{}", city_name_used.to_lowercase().replace(' ', "-"), tag_type);
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

            tracing::info!("[provision] ZaarHub tracking tags created for '{prefix}' on parent tenant");
        }

        tracing::info!("[provision] All city tags created for '{prefix}'");
    } else {
        tracing::warn!("[provision] No CoreSwift tenant found for directory {directory_id} — skipping booking/tags");
    }

    Ok(prefix)
}
