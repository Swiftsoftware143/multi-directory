//! Domain mapping CRUD and verification handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::models::*;

/// POST /api/v1/admin/domains
pub async fn register_domain(
    State(s): State<AppState>,
    Json(req): Json<RegisterDomainRequest>,
) -> ApiResult<impl IntoResponse> {
    // Validate domain format (safe for all operations)
    validate_domain_safe(&req.domain)?;

    // Check if domain already registered
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE domain = $1 "
    )
    .bind(&req.domain)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!("Domain '{}' is already registered", req.domain)));
    }

    let domain_type = req.domain_type.unwrap_or_else(|| "subfolder".to_string());
    let verification_token = Uuid::new_v4().to_string();
    let status = "pending".to_string();

    let mapping = sqlx::query_as::<_, DomainMapping>(
        r#"INSERT INTO domain_mappings (domain, type, status, verification_token)
           VALUES ($1, $2, $3, $4)
           RETURNING id, directory_id, domain, type as domain_type, status, ssl_enabled,
                     cloudflare_record_id, dns_records, verification_token, auto_configured,
                     created_at, updated_at"#
    )
    .bind(&req.domain)
    .bind(&domain_type)
    .bind(&status)
    .bind(&verification_token)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(mapping))))
}

/// GET /api/v1/admin/domains
pub async fn list_domains(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let domains = sqlx::query_as::<_, DomainMapping>(
        r#"SELECT id, directory_id, domain, type as domain_type, status, ssl_enabled,
                  cloudflare_record_id, dns_records, verification_token, auto_configured,
                  created_at, updated_at
           FROM domain_mappings ORDER BY created_at DESC"#
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(domains)))
}

/// DELETE /api/v1/admin/domains/:id
pub async fn remove_domain(
    State(s): State<AppState>,
    Path(domain_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM domain_mappings WHERE id = $1")
        .bind(domain_id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Domain mapping not found".to_string()));
    }

    Ok(Json(json!({"message": "Domain removed successfully"})))
}

/// POST /api/v1/admin/domains/:id/verify
pub async fn verify_domain(
    State(s): State<AppState>,
    Path(domain_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let mapping = sqlx::query_as::<_, DomainMapping>(
        r#"SELECT id, directory_id, domain, type as domain_type, status, ssl_enabled,
                  cloudflare_record_id, dns_records, verification_token, auto_configured,
                  created_at, updated_at
           FROM domain_mappings WHERE id = $1"#
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

    // Update status
    let updated = sqlx::query_as::<_, DomainMapping>(
        r#"UPDATE domain_mappings SET status = 'active', updated_at = NOW()
           WHERE id = $1
           RETURNING id, directory_id, domain, type as domain_type, status, ssl_enabled,
                     cloudflare_record_id, dns_records, verification_token, auto_configured,
                     created_at, updated_at"#
    )
    .bind(domain_id)
    .fetch_one(&s.db)
    .await?;

    // Attempt to provision nginx config and SSL
    let upstream_addr = format!("http://{}:{}", s.config.host, s.config.port);
    let provision_result = provision_nginx_site(&mapping.domain, &upstream_addr).await;
    let nginx_ok = provision_result.is_ok();
    if let Err(e) = provision_result {
        tracing::warn!("Nginx provisioning for {} failed: {}", mapping.domain, e);
    }

    let ssl_result = provision_ssl_certificate(&mapping.domain, &s.config.admin_email, &upstream_addr).await;
    let ssl_ok = ssl_result.is_ok();
    if let Err(e) = ssl_result {
        tracing::warn!("SSL provisioning for {} failed: {}", mapping.domain, e);
    }

    Ok(Json(json!({
        "message": "Domain verified and configured successfully",
        "domain": updated.domain,
        "status": updated.status,
        "nginx_configured": nginx_ok,
        "ssl_configured": ssl_ok,
    })))
}

/// GET /api/v1/admin/plans/:id/domains
pub async fn check_plan_domains(
    State(s): State<AppState>,
    Path(plan_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings "
    )
    .fetch_one(&s.db)
    .await?;

    let active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE status = 'active'"
    )
    .fetch_one(&s.db)
    .await?;

    let pending = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE status = 'pending'"
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
            return Err(AppError::Validation("Invalid characters in domain".to_string()));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(AppError::Validation("Domain label cannot start/end with hyphen".to_string()));
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
    use std::process::Command;
    use std::fs;

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
async fn provision_ssl_certificate(domain: &str, admin_email: &str, upstream_addr: &str) -> Result<(), String> {
    use std::process::Command;

    if let Err(e) = validate_domain_safe(domain) {
        return Err(format!("Invalid domain: {}", e.to_string()));
    }

    // Try certbot first - domain is validated so -d argument is safe
    let certbot = Command::new("certbot")
        .args(["--nginx", "-d", domain, "--non-interactive", "--agree-tos", "-m", admin_email])
        .output();

    match certbot {
        Ok(out) if out.status.success() => {
            tracing::info!("SSL certificate obtained for {} via certbot", domain);
            return Ok(());
        }
        Ok(out) => {
            tracing::warn!("Certbot failed for {}: {}", domain, String::from_utf8_lossy(&out.stderr));
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

    let _ = Command::new("systemctl")
        .args(["reload", "nginx"])
        .output();

    Ok(())
}
