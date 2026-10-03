//! Domain mapping CRUD and verification handlers.
//!
//! FEATURE 2 (David, 2026-09-29): every directory — whether it sits inside a network or stands
//! alone — can be served on
//!   (i)   a SUBDOMAIN of its network's root domain  (palm-bay.zaarhub.com)
//!   (ii)  its own CUSTOM DOMAIN                     (a plumber's own domain)
//!   (iii) a SUBFOLDER path of a host                (zaarhub.com/palm-bay)
//!
//! One row per (host, url_path) in `domain_mappings`; a directory may hold several. Every
//! mapping carries a status (pending/active) and a live check that records what was ACTUALLY
//! observed (DNS resolution + HTTP probe) so the panel can say "not live yet" honestly — a
//! domain is never reported live while its DNS still points elsewhere.
//!
//! The public site keeps serving every directory exactly as before: the container's host-based
//! resolution (routes.rs) maps host -> directory slug and redirects to /d/<slug>.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::*;
use crate::AppState;

/// Mapping columns plus everything the panel needs to explain where a mapping points.
/// Kept in one place so every handler returns the same shape.
///
/// Gate rule 5d: `concat!` needs a LITERAL, not a `const`, so the statement is published as a
/// macro and every statement that uses it is assembled entirely at compile time (a run-time build
/// is the defect the rule names). `select_mappings_from!` differs ONLY in the FROM target —
/// `domain_mappings` (the plain table), `ins` (the INSERT … RETURNING CTE) and `upd` (the
/// UPDATE … RETURNING CTE) — so the three expansions are byte-identical outside that one token,
/// exactly what the old `SELECT_MAPPINGS.replace("FROM domain_mappings dm", …)` did at run time.
macro_rules! select_mappings_from {
    ($src:literal) => {
        concat!(
            r#"SELECT dm.id, dm.directory_id, dm.domain, dm.type, dm.status,
        dm.ssl_enabled, dm.cloudflare_record_id, dm.dns_records, dm.verification_token,
        dm.auto_configured, dm.url_path, dm.live_status,
        dm.last_checked_at::text AS last_checked_at, dm.last_check_detail,
        dm.created_at::text AS created_at, dm.updated_at::text AS updated_at,
        d.slug AS directory_slug, d.name AS directory_name, d.network_id,
        n.slug AS network_slug, n.root_domain
     FROM "#,
            $src,
            r#" dm
     LEFT JOIN directories d ON d.id = dm.directory_id
     LEFT JOIN networks n ON n.id = d.network_id"#
        )
    };
}
macro_rules! select_mappings {
    () => {
        select_mappings_from!("domain_mappings")
    };
}

/// One mapping as JSON. `domain_type` is kept alongside `type` because the older SPA in
/// portal.html reads either; nothing here is invented — every value comes from the row.
fn mapping_json(row: &sqlx::postgres::PgRow) -> Value {
    let domain_type: String = row.try_get("type").unwrap_or_else(|_| "subfolder".into());
    let url_path: String = row.try_get("url_path").unwrap_or_default();
    let host: String = row.try_get("domain").unwrap_or_default();
    json!({
        "id": row.try_get::<Uuid, _>("id").ok(),
        "directory_id": row.try_get::<Option<Uuid>, _>("directory_id").unwrap_or(None),
        "directory_slug": row.try_get::<Option<String>, _>("directory_slug").unwrap_or(None),
        "directory_name": row.try_get::<Option<String>, _>("directory_name").unwrap_or(None),
        "network_id": row.try_get::<Option<Uuid>, _>("network_id").unwrap_or(None),
        "network_slug": row.try_get::<Option<String>, _>("network_slug").unwrap_or(None),
        "root_domain": row.try_get::<Option<String>, _>("root_domain").unwrap_or(None),
        "domain": host,
        "url_path": url_path,
        "type": domain_type,
        "domain_type": domain_type,
        "url": format!("{}{}", host, url_path),
        "status": row.try_get::<Option<String>, _>("status").unwrap_or(None),
        "ssl_enabled": row.try_get::<Option<bool>, _>("ssl_enabled").unwrap_or(None),
        "verification_token": row.try_get::<Option<String>, _>("verification_token").unwrap_or(None),
        "auto_configured": row.try_get::<Option<bool>, _>("auto_configured").unwrap_or(None),
        "live_status": row.try_get::<Option<String>, _>("live_status").unwrap_or(None),
        "last_checked_at": row.try_get::<Option<String>, _>("last_checked_at").unwrap_or(None),
        "last_check_detail": row.try_get::<Option<String>, _>("last_check_detail").unwrap_or(None),
        "created_at": row.try_get::<Option<String>, _>("created_at").unwrap_or(None),
        "updated_at": row.try_get::<Option<String>, _>("updated_at").unwrap_or(None),
    })
}

/// Fold a host, a type and a path into what the database stores. Nothing outside this
/// vocabulary is accepted — a mapping kind that no code understands is worse than none.
fn normalize_type(raw: Option<&str>) -> Result<&'static str, AppError> {
    match raw.unwrap_or("custom").trim().to_ascii_lowercase().as_str() {
        "subdomain" => Ok("subdomain"),
        "custom" | "custom_domain" | "domain" => Ok("custom"),
        "subfolder" | "path" => Ok("subfolder"),
        other => Err(AppError::Validation(format!(
            "Unknown mapping type '{}' — use subdomain, custom or subfolder",
            other
        ))),
    }
}

/// A subfolder path is either empty (a whole host) or a clean `/segment/segment` tail.
fn normalize_path(raw: Option<&str>) -> Result<String, AppError> {
    let p = raw.unwrap_or("").trim().trim_end_matches('/').to_string();
    if p.is_empty() || p == "/" {
        return Ok(String::new());
    }
    let with_slash = if p.starts_with('/') {
        p
    } else {
        format!("/{}", p)
    };
    if with_slash.contains("..") || with_slash.contains(' ') || with_slash.contains('?') {
        return Err(AppError::Validation(
            "A subfolder path may not contain spaces, '..' or a query string".into(),
        ));
    }
    Ok(with_slash)
}

/// Lowercase the host and refuse anything that is not a bare hostname (scheme/path/userinfo
/// would silently produce a mapping that can never match an incoming Host header).
fn normalize_host(raw: &str) -> Result<String, AppError> {
    let host = raw
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .replace("https://", "")
        .replace("http://", "");
    let host = host.split('/').next().unwrap_or("").to_string();
    if host.contains('@') || host.contains(':') {
        return Err(AppError::Validation(
            "Enter a bare hostname (no scheme, port or credentials)".into(),
        ));
    }
    validate_domain_safe(&host)?;
    Ok(host)
}

/// Keep directories.url_type / url_value / custom_domain in step with the ACTIVE mapping the
/// panel shows, so the older code paths that read those columns keep telling the truth.
async fn sync_directory_url(db: &PgPool, directory_id: Option<Uuid>) -> Result<(), AppError> {
    let Some(dir) = directory_id else {
        return Ok(());
    };
    let row = sqlx::query(
        r#"SELECT domain, type, url_path FROM domain_mappings
           WHERE directory_id = $1 AND status = 'active'
           ORDER BY CASE type WHEN 'custom' THEN 0 WHEN 'subdomain' THEN 1 ELSE 2 END,
                    updated_at DESC
           LIMIT 1"#,
    )
    .bind(dir)
    .fetch_optional(db)
    .await?;

    match row {
        Some(r) => {
            let host: String = r.try_get("domain").unwrap_or_default();
            let kind: String = r.try_get("type").unwrap_or_else(|_| "subfolder".into());
            let path: String = r.try_get("url_path").unwrap_or_default();
            let value = if kind == "subfolder" {
                format!("{}{}", host, path)
            } else {
                host.clone()
            };
            let custom = if kind == "custom" { Some(host) } else { None };
            sqlx::query(
                "UPDATE directories SET url_type = $1, url_value = $2, custom_domain = $3, \
                 updated_at = NOW() WHERE id = $4",
            )
            .bind(&kind)
            .bind(&value)
            .bind(&custom)
            .bind(dir)
            .execute(db)
            .await?;
        }
        None => {
            // No active mapping left: back to a plain standalone directory.
            sqlx::query(
                "UPDATE directories SET url_type = 'standalone', url_value = NULL, \
                 custom_domain = NULL, updated_at = NOW() WHERE id = $1",
            )
            .bind(dir)
            .execute(db)
            .await?;
        }
    }
    Ok(())
}

/// POST /api/v1/admin/domains — attach a host (and optional subfolder path) to a directory.
pub async fn register_domain(
    State(s): State<AppState>,
    Json(req): Json<RegisterDomainRequest>,
) -> ApiResult<impl IntoResponse> {
    let host = normalize_host(&req.domain)?;
    let kind = normalize_type(req.domain_type.as_deref())?;
    let path = normalize_path(req.url_path.as_deref())?;

    if kind == "subfolder" && path.is_empty() {
        return Err(AppError::Validation(
            "A subfolder mapping needs a path, e.g. /palm-bay".into(),
        ));
    }
    if kind != "subfolder" && !path.is_empty() {
        return Err(AppError::Validation(
            "Only a subfolder mapping carries a path".into(),
        ));
    }

    // A mapping with no directory serves nothing — refuse it instead of storing an orphan row.
    let directory_id = req
        .directory_id
        .ok_or_else(|| AppError::Validation("Which directory is this host for?".into()))?;

    let dir = sqlx::query(
        r#"SELECT d.id, d.slug, d.name, d.network_id, n.root_domain
           FROM directories d LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1"#,
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let slug: String = dir.try_get("slug").unwrap_or_default();
    let root_domain: Option<String> = dir.try_get("root_domain").unwrap_or(None);

    // A subdomain only means something relative to the network's root domain. Checking here is
    // what stops the panel from storing "palm-bay.somewhere-else.com" as a network subdomain.
    if kind == "subdomain" {
        match root_domain
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            Some(root) => {
                let suffix = format!(".{}", root.to_ascii_lowercase());
                if !host.ends_with(&suffix) {
                    return Err(AppError::Validation(format!(
                        "A subdomain mapping for '{}' must end with the network root domain '{}'",
                        slug, root
                    )));
                }
            }
            None => {
                return Err(AppError::Validation(
                    "This directory's network has no root domain yet — set it first".into(),
                ))
            }
        }
    }

    // Re-registering the same host+path is a duplicate; two directories may share a host under
    // different paths.
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE domain = $1 AND url_path = $2",
    )
    .bind(&host)
    .bind(&path)
    .fetch_one(&s.db)
    .await?;
    if existing > 0 {
        return Err(AppError::Duplicate(format!(
            "{} is already registered",
            if path.is_empty() {
                host.clone()
            } else {
                format!("{}{}", host, path)
            }
        )));
    }

    let verification_token = Uuid::new_v4().to_string();
    let row = sqlx::query(concat!(
        "WITH ins AS ( INSERT INTO domain_mappings (directory_id, domain, type, url_path, status, verification_token) VALUES ($1, $2, $3, $4, 'pending', $5) RETURNING * ) ",
        select_mappings_from!("ins"),
        " WHERE dm.id = ins.id"
    ))
    .bind(directory_id)
    .bind(&host)
    .bind(kind)
    .bind(&path)
    .bind(&verification_token)
    .fetch_one(&s.db)
    .await?;

    sync_directory_url(&s.db, Some(directory_id)).await?;

    Ok((StatusCode::CREATED, Json(mapping_json(&row))))
}

/// GET /api/v1/admin/domains — every mapping (bare array, the shape portal.html already reads).
pub async fn list_domains(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(concat!(
        r#""#,
        select_mappings!(),
        r#" ORDER BY dm.created_at DESC"#
    ))
    .fetch_all(&s.db)
    .await?;
    let out: Vec<Value> = rows.iter().map(mapping_json).collect();
    Ok(Json(json!(out)))
}

/// GET /api/v1/admin/directories/:id/domains — one directory's mappings, plus the subdomain the
/// panel should suggest (its slug under the network root domain, when the network has one).
pub async fn list_directory_domains(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query(
        r#"SELECT d.id, d.slug, d.name, d.network_id, d.url_type, d.url_value, d.custom_domain,
                  n.slug AS network_slug, n.name AS network_name, n.root_domain
           FROM directories d LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1"#,
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let rows = sqlx::query(concat!(
        r#""#,
        select_mappings!(),
        r#" WHERE dm.directory_id = $1 ORDER BY dm.created_at ASC"#
    ))
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    let slug: String = dir.try_get("slug").unwrap_or_default();
    let root_domain: Option<String> = dir.try_get("root_domain").unwrap_or(None);
    let suggested = root_domain
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|root| format!("{}.{}", slug, root.to_ascii_lowercase()));

    let mappings: Vec<Value> = rows.iter().map(mapping_json).collect();
    let request = json!({
        "directory": {
            "id": id,
            "slug": slug,
            "name": dir.try_get::<String, _>("name").unwrap_or_default(),
            "url_type": dir.try_get::<Option<String>, _>("url_type").unwrap_or(None),
            "url_value": dir.try_get::<Option<String>, _>("url_value").unwrap_or(None),
            "custom_domain": dir.try_get::<Option<String>, _>("custom_domain").unwrap_or(None),
        },
        "network": dir.try_get::<Option<Uuid>, _>("network_id").unwrap_or(None).map(|nid| json!({
            "id": nid,
            "slug": dir.try_get::<Option<String>, _>("network_slug").unwrap_or(None),
            "name": dir.try_get::<Option<String>, _>("network_name").unwrap_or(None),
            "root_domain": root_domain,
        })),
        "suggested_subdomain": suggested,
        "data": mappings,
    });
    Ok(Json(request))
}

/// PUT /api/v1/admin/domains/:id — edit a mapping: rename the host, change its kind or path,
/// move it to another directory, or flip pending/active.
pub async fn update_domain(
    State(s): State<AppState>,
    Path(domain_id): Path<Uuid>,
    Json(req): Json<UpdateDomainRequest>,
) -> ApiResult<impl IntoResponse> {
    let current = sqlx::query(
        "SELECT id, directory_id, domain, type, url_path, status FROM domain_mappings WHERE id = $1",
    )
    .bind(domain_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Domain mapping not found".to_string()))?;

    let old_dir: Option<Uuid> = current.try_get("directory_id").unwrap_or(None);
    let host = match req
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(d) => normalize_host(d)?,
        None => current.try_get::<String, _>("domain").unwrap_or_default(),
    };
    let kind = match req.domain_type.as_deref() {
        Some(t) => normalize_type(Some(t))?,
        None => {
            let t: String = current
                .try_get("type")
                .unwrap_or_else(|_| "subfolder".into());
            normalize_type(Some(&t))?
        }
    };
    let path = match req.url_path.as_deref() {
        Some(p) => normalize_path(Some(p))?,
        None => current.try_get::<String, _>("url_path").unwrap_or_default(),
    };
    if kind == "subfolder" && path.is_empty() {
        return Err(AppError::Validation(
            "A subfolder mapping needs a path, e.g. /palm-bay".into(),
        ));
    }
    if kind != "subfolder" && !path.is_empty() {
        return Err(AppError::Validation(
            "Only a subfolder mapping carries a path".into(),
        ));
    }
    let directory_id = req.directory_id.or(old_dir);
    let status = match req.status.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if s == "active" || s == "pending" => s,
        Some(other) => {
            return Err(AppError::Validation(format!(
                "Unknown status '{}' — use pending or active",
                other
            )))
        }
        None => current
            .try_get::<Option<String>, _>("status")
            .unwrap_or(None)
            .unwrap_or_else(|| "pending".to_string()),
    };

    let dup = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE domain = $1 AND url_path = $2 AND id <> $3",
    )
    .bind(&host)
    .bind(&path)
    .bind(domain_id)
    .fetch_one(&s.db)
    .await?;
    if dup > 0 {
        return Err(AppError::Duplicate(format!(
            "{}{} is already registered",
            host, path
        )));
    }

    let ssl_enabled: Option<bool> = req.ssl_enabled.or(current
        .try_get::<Option<bool>, _>("ssl_enabled")
        .unwrap_or(None));

    let row = sqlx::query(concat!(
        "WITH upd AS ( UPDATE domain_mappings SET domain = $1, type = $2, url_path = $3, directory_id = $4, status = $5, ssl_enabled = $6, updated_at = NOW() WHERE id = $7 RETURNING * ) ",
        select_mappings_from!("upd"),
        " WHERE dm.id = upd.id"
    ))
    .bind(&host)
    .bind(kind)
    .bind(&path)
    .bind(directory_id)
    .bind(&status)
    .bind(ssl_enabled)
    .bind(domain_id)
    .fetch_one(&s.db)
    .await?;

    sync_directory_url(&s.db, directory_id).await?;
    if old_dir != directory_id {
        sync_directory_url(&s.db, old_dir).await?;
    }

    Ok(Json(mapping_json(&row)))
}

/// DELETE /api/v1/admin/domains/:id
pub async fn remove_domain(
    State(s): State<AppState>,
    Path(domain_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT directory_id FROM domain_mappings WHERE id = $1",
    )
    .bind(domain_id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    let result = sqlx::query("DELETE FROM domain_mappings WHERE id = $1")
        .bind(domain_id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Domain mapping not found".to_string()));
    }

    sync_directory_url(&s.db, dir).await?;

    Ok(Json(json!({"message": "Domain removed successfully"})))
}

/// POST /api/v1/admin/domains/:id/verify — TXT-record verification, then nginx + SSL.
pub async fn verify_domain(
    State(s): State<AppState>,
    Path(domain_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let mapping = sqlx::query_as::<_, DomainMapping>(
        r#"SELECT id, directory_id, domain, type as domain_type, status, ssl_enabled,
                  cloudflare_record_id, dns_records, verification_token, auto_configured,
                  created_at, updated_at
           FROM domain_mappings WHERE id = $1"#,
    )
    .bind(domain_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Domain mapping not found".to_string()))?;

    // DNS verification via TXT record check (using safe DNS library)
    let token_ref = mapping.verification_token.as_deref().unwrap_or("");
    let is_verified = check_dns_verification(&mapping.domain, token_ref).await;

    if !is_verified {
        return Err(AppError::BadRequest(
            "Domain verification failed. Please add the TXT record to your DNS.              Verification token: ".to_string() + token_ref
        ));
    }

    sqlx::query("UPDATE domain_mappings SET status = 'active', updated_at = NOW() WHERE id = $1")
        .bind(domain_id)
        .execute(&s.db)
        .await?;

    // Attempt to provision nginx config and SSL
    let upstream_addr = format!("http://{}:{}", s.config.host, s.config.port);
    let provision_result = provision_nginx_site(&mapping.domain, &upstream_addr).await;
    let nginx_ok = provision_result.is_ok();
    if let Err(e) = provision_result {
        tracing::warn!("Nginx provisioning for {} failed: {}", mapping.domain, e);
    }

    let ssl_result =
        provision_ssl_certificate(&mapping.domain, &s.config.admin_email, &upstream_addr).await;
    let ssl_ok = ssl_result.is_ok();
    if let Err(e) = ssl_result {
        tracing::warn!("SSL provisioning for {} failed: {}", mapping.domain, e);
    }

    sync_directory_url(&s.db, mapping.directory_id).await?;

    Ok(Json(json!({
        "message": "Domain verified and configured successfully",
        "domain": mapping.domain,
        "status": "active",
        "nginx_configured": nginx_ok,
        "ssl_configured": ssl_ok,
    })))
}

/// POST /api/v1/admin/domains/:id/check — is this mapping LIVE right now?
/// Records what was actually observed: DNS resolution of the host and an HTTP probe of the
/// exact URL the visitor would open. A host whose DNS does not resolve is reported as
/// `dns_pending` with the record that still has to be created — it is never called live.
pub async fn check_domain_live(
    State(s): State<AppState>,
    Path(domain_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query(
        "SELECT domain, url_path, directory_id, status FROM domain_mappings WHERE id = $1",
    )
    .bind(domain_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Domain mapping not found".to_string()))?;

    let host: String = row.try_get("domain").unwrap_or_default();
    let path: String = row.try_get("url_path").unwrap_or_default();
    let status: Option<String> = row.try_get("status").unwrap_or(None);

    let dns_ok = match tokio::net::lookup_host((host.as_str(), 443)).await {
        Ok(mut addrs) => addrs.next().is_some(),
        Err(_) => false,
    };

    let (live_status, code, detail) = if !dns_ok {
        (
            "dns_pending",
            None,
            format!(
                "No A/AAAA record for {} yet — point the domain at this server, then check again",
                host
            ),
        )
    } else {
        let url = format!(
            "https://{}{}",
            host,
            if path.is_empty() {
                "/".to_string()
            } else {
                format!("{}/", path)
            }
        );
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (compatible; SwiftOps/1.0)")
            .build();
        match client {
            Ok(c) => match c.get(&url).send().await {
                Ok(resp) => {
                    let code = resp.status().as_u16();
                    if code < 400 {
                        ("live", Some(code), format!("{} answered {}", url, code))
                    } else {
                        (
                            "unreachable",
                            Some(code),
                            format!("{} answered {} — check nginx for this host", url, code),
                        )
                    }
                }
                Err(e) => (
                    "dns_points_here_but_no_response",
                    None,
                    format!("DNS resolves but {} did not answer: {}", url, e),
                ),
            },
            Err(e) => ("error", None, format!("HTTP client error: {}", e)),
        }
    };

    sqlx::query(
        "UPDATE domain_mappings SET live_status = $1, last_checked_at = NOW(), \
         last_check_detail = $2, updated_at = NOW() WHERE id = $3",
    )
    .bind(live_status)
    .bind(&detail)
    .bind(domain_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "success": true,
        "domain": host,
        "path": path,
        "url": format!("{}{}", host, path),
        "mapping_status": status,
        "dns_resolves": dns_ok,
        "http_status": code,
        "live_status": live_status,
        "detail": detail,
    })))
}

/// GET /api/v1/admin/plans/:id/domains
pub async fn check_plan_domains(
    State(s): State<AppState>,
    Path(plan_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM domain_mappings ")
        .fetch_one(&s.db)
        .await?;

    let active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE status = 'active'",
    )
    .fetch_one(&s.db)
    .await?;

    let pending = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE status = 'pending'",
    )
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "plan_id": plan_id,
        "total_domains": count,
        "active_domains": active,
        "pending_domains": pending,
    })))
}

/// Validate a domain name is safe to use.
/// RFC 1035 compliant: letters, digits, hyphens, dots, max 253 chars
fn validate_domain_safe(domain: &str) -> Result<(), AppError> {
    if domain.is_empty() || domain.len() > 253 {
        return Err(AppError::Validation("Invalid domain length".to_string()));
    }
    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(AppError::Validation("Invalid domain label".to_string()));
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(AppError::Validation(
                "Invalid characters in domain".to_string(),
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(AppError::Validation(
                "Domain label cannot start/end with hyphen".to_string(),
            ));
        }
    }
    Ok(())
}

// ── Helper functions ─────────────────────────────────────────────────────────

/// Check DNS TXT record for verification token using trust-dns-resolver.
/// Safe: no shell commands, no command injection risk.
async fn check_dns_verification(domain: &str, token: &str) -> bool {
    use trust_dns_resolver::TokioAsyncResolver;

    let lookup = format!("_swift-verify.{}", domain);

    let resolver = match TokioAsyncResolver::tokio_from_system_conf() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to create DNS resolver: {}", e);
            return false;
        }
    };

    match resolver.txt_lookup(&lookup).await {
        Ok(response) => {
            for record in response.iter() {
                let txt_string = record.to_string();
                if txt_string.contains(token) {
                    return true;
                }
            }
            false
        }
        Err(e) => {
            tracing::warn!("DNS TXT lookup failed for {}: {}", lookup, e);
            false
        }
    }
}

/// Provision nginx site config for a custom domain.
/// Domain is validated before use. Paths use validated domain only.
async fn provision_nginx_site(domain: &str, upstream_addr: &str) -> Result<(), String> {
    use std::fs;
    use std::process::Command;

    if let Err(e) = validate_domain_safe(domain) {
        return Err(format!("Invalid domain: {}", e.to_string()));
    }

    let config_content = format!(
        r#"server {{
    listen 80;
    server_name {domain};

    location / {{
        proxy_pass {upstream_addr};
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }}
}}
"#,
        domain = domain
    );

    // Path uses domain name as filename - safe because domain is validated alphanumeric/hyphen only
    let config_path = format!("/etc/nginx/sites-available/{}", domain);
    fs::write(&config_path, &config_content)
        .map_err(|e| format!("Failed to write nginx config: {}", e))?;

    let enable_path = format!("/etc/nginx/sites-enabled/{}", domain);
    let _ = Command::new("ln")
        .args(["-sf", &config_path, &enable_path])
        .output();

    let test = Command::new("nginx")
        .args(["-t"])
        .output()
        .map_err(|e| format!("Nginx test failed: {}", e))?;

    if !test.status.success() {
        let stderr = String::from_utf8_lossy(&test.stderr);
        return Err(format!("Nginx config test failed: {}", stderr));
    }

    let reload = Command::new("systemctl")
        .args(["reload", "nginx"])
        .output()
        .map_err(|e| format!("Nginx reload failed: {}", e))?;

    if !reload.status.success() {
        let stderr = String::from_utf8_lossy(&reload.stderr);
        return Err(format!("Nginx reload failed: {}", stderr));
    }

    Ok(())
}

/// Provision SSL certificate via certbot or self-signed.
/// Domain is validated before use. Certbot -d arg uses single validated argument only.
async fn provision_ssl_certificate(
    domain: &str,
    admin_email: &str,
    upstream_addr: &str,
) -> Result<(), String> {
    use std::process::Command;

    if let Err(e) = validate_domain_safe(domain) {
        return Err(format!("Invalid domain: {}", e.to_string()));
    }

    // Try certbot first - domain is validated so -d argument is safe
    let certbot = Command::new("certbot")
        .args([
            "--nginx",
            "-d",
            domain,
            "--non-interactive",
            "--agree-tos",
            "-m",
            admin_email,
        ])
        .output();

    match certbot {
        Ok(out) if out.status.success() => {
            tracing::info!("SSL certificate obtained for {} via certbot", domain);
            return Ok(());
        }
        Ok(out) => {
            tracing::warn!(
                "Certbot failed for {}: {}",
                domain,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Err(_) => {
            tracing::warn!("Certbot not available for {}", domain);
        }
    }

    // Fallback: update nginx config to use self-signed certificate
    let ssl_config = format!(
        r#"server {{
    listen 443 ssl;
    server_name {domain};

    ssl_certificate /etc/ssl/certs/self-signed.crt;
    ssl_certificate_key /etc/ssl/private/self-signed.key;

    location / {{
        proxy_pass {upstream_addr};
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }}
}}

server {{
    listen 80;
    server_name {domain};
    return 301 https://$server_name$request_uri;
}}
"#,
        domain = domain
    );

    let config_path = format!("/etc/nginx/sites-available/{}", domain);
    std::fs::write(&config_path, &ssl_config)
        .map_err(|e| format!("Failed to write SSL nginx config: {}", e))?;

    let _ = Command::new("systemctl").args(["reload", "nginx"]).output();

    Ok(())
}
