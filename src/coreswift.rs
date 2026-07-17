//! CoreSwift integration — push business claims and newsletter signups
//! to CoreSwift CRM via its internal API (x-internal-key auth).
//!
//! Architecture:
//!   - One CoreSwift tenant per network (or per standalone directory)
//!   - Auto-provisioned: tenant + default lists on first use
//!   - Data pushed via internal endpoints (bypasses JWT)
//!   - Database is always source of truth; CoreSwift push is secondary

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// CoreSwift URL — read from CORESWIFT_URL env var at runtime, fallback localhost:8084
fn coreswift_url() -> String {
    std::env::var("CORESWIFT_URL").unwrap_or_else(|_| "http://localhost:8084".to_string())
}

/// Internal key for CoreSwift API — MUST be set via CORESWIFT_INTERNAL_KEY env var
fn internal_key() -> String {
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
/// Checks directory-level first, then falls back to parent network.
pub async fn resolve_config(
    db: &PgPool,
    directory_id: Uuid,
) -> Result<(Uuid, Uuid, Uuid), String> {
    let row = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>)>(
        r#"SELECT d.coreswift_tenant_id, d.network_id, d.coreswift_list_id_claimed, d.coreswift_list_id_newsletter,
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

    let (dir_tid, network_id, dir_lc, dir_ln, net_tid) = row;

    if let (Some(tid), Some(lc), Some(ln)) = (dir_tid, dir_lc, dir_ln) {
        return Ok((tid, lc, ln));
    }

    if let Some(tid) = net_tid {
        if let Some(nid) = network_id {
            let (lc, ln) = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>)>(
                "SELECT coreswift_list_id_claimed, coreswift_list_id_newsletter FROM networks WHERE id = $1"
            )
            .bind(nid)
            .fetch_optional(db)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or_else(|| format!("Network {nid} not found"))?;

            if let (Some(lc), Some(ln)) = (lc, ln) {
                return Ok((tid, lc, ln));
            }
        }
    }

    Err(format!("No CoreSwift tenant provisioned for directory {directory_id}"))
}

fn cs_url(path: &str) -> String {
    format!("{}{}", coreswift_url(), path)
}

/// Provision a CoreSwift tenant for a new entity (network or standalone directory).
/// Creates: tenant account, list for claimed businesses, list for newsletter.
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

    let claimed_list = create_list(token, name, "Claimed Businesses").await?;
    let newsletter_list = create_list(token, name, "Newsletter Subscribers").await?;

    if is_network {
        sqlx::query(
            "UPDATE networks SET coreswift_tenant_id = $1, coreswift_list_id_claimed = $2, coreswift_list_id_newsletter = $3 WHERE id = $4"
        )
        .bind(tid)
        .bind(claimed_list)
        .bind(newsletter_list)
        .bind(entity_id)
        .execute(db)
        .await
        .map_err(|e| format!("DB update failed: {e}"))?;
    } else {
        sqlx::query(
            "UPDATE directories SET coreswift_tenant_id = $1, coreswift_list_id_claimed = $2, coreswift_list_id_newsletter = $3 WHERE id = $4"
        )
        .bind(tid)
        .bind(claimed_list)
        .bind(newsletter_list)
        .bind(entity_id)
        .execute(db)
        .await
        .map_err(|e| format!("DB update failed: {e}"))?;
    }

    tracing::info!("[coreswift] Provisioned tenant {tenant_id} for {name} ({slug})");
    Ok(())
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

    let (tenant_id, claimed_list_id, _) = resolve_config(db, dir_id).await?;
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

    tracing::info!("[coreswift] Pushed claimed business {business_id} (owner: {owner_email})");
    Ok(())
}

/// Push a newsletter signup to the CRM — creates a contact and adds to "Newsletter Subscribers" list.
pub async fn push_newsletter_signup(
    db: &PgPool,
    directory_id: Uuid,
    email: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let (tenant_id, _, newsletter_list_id) = resolve_config(db, directory_id).await?;
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

    tracing::info!("[coreswift] Pushed newsletter signup ({email})");
    Ok(())
}

/// Add a contact to the "Claimed Businesses" list in CoreSwift.
pub async fn add_to_claimed_list(
    db: &PgPool,
    directory_id: Uuid,
    contact_email: &str,
) -> Result<(), String> {
    let (tenant_id, claimed_list_id, _) = resolve_config(db, directory_id).await?;
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
