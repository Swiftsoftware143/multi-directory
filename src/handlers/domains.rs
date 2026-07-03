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
    // Validate domain format (basic)
    if req.domain.is_empty() || !req.domain.contains('.') {
        return Err(AppError::Validation("Invalid domain format".to_string()));
    }

    // Check if domain already registered
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings WHERE domain = \x241 "
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
           VALUES (\x241, \x242, \x243, \x244)
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
    let result = sqlx::query("DELETE FROM domain_mappings WHERE id = \x241")
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
           FROM domain_mappings WHERE id = \x241"#
    )
    .bind(domain_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Domain mapping not found".to_string()))?;

    // DNS verification via TXT record check
    let is_verified = check_dns_verification(&mapping.domain, &mapping.verification_token.as_deref().unwrap_or("")).await;

    if !is_verified {
        return Err(AppError::BadRequest(
            "Domain verification failed. Please add the TXT record to your DNS. \
             Verification token: ".to_string() + &mapping.verification_token.as_deref().unwrap_or("")
        ));
    }

    // Update status
    let updated = sqlx::query_as::<_, DomainMapping>(
        r#"UPDATE domain_mappings SET status = 'active', updated_at = NOW()
           WHERE id = \x241
           RETURNING id, directory_id, domain, type as domain_type, status, ssl_enabled,
                     cloudflare_record_id, dns_records, verification_token, auto_configured,
                     created_at, updated_at"#
    )
    .bind(domain_id)
    .fetch_one(&s.db)
    .await?;

    // Attempt to provision nginx config and SSL
    let provision_result = provision_nginx_site(&mapping.domain).await;
    let nginx_ok = provision_result.is_ok();
    if let Err(e) = provision_result {
        tracing::warn!("Nginx provisioning for {} failed: {}", mapping.domain, e);
    }

    let ssl_result = provision_ssl_certificate(&mapping.domain).await;
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

// ── Helper functions ─────────────────────────────────────────────────────────

/// Check DNS TXT record for verification token
async fn check_dns_verification(domain: &str, token: &str) -> bool {
    use std::process::Command;

    let output = Command::new("dig")
        .arg("TXT")
        .arg(&format!("_swift-verify.{}", domain))
        .arg("+short")
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            stdout.contains(token)
        }
        Err(_) => {
            // dig not available, try nslookup
            let output = Command::new("nslookup")
                .arg("-type=TXT")
                .arg(&format!("_swift-verify.{}", domain))
                .output();

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    stdout.contains(token)
                }
                Err(_) => false
            }
        }
    }
}

/// Provision nginx site config for a custom domain
async fn provision_nginx_site(domain: &str) -> Result<(), String> {
    use std::process::Command;
    use std::fs;

    let config_content = format!(
        r#"server {{
    listen 80;
    server_name {domain};

    location / {{
        proxy_pass http://127.0.0.1:3001;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }}
}}
"#,
        domain = domain
    );

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

/// Provision SSL certificate via certbot or self-signed
async fn provision_ssl_certificate(domain: &str) -> Result<(), String> {
    use std::process::Command;

    // Try certbot first
    let certbot = Command::new("certbot")
        .args(["--nginx", "-d", domain, "--non-interactive", "--agree-tos", "-m", "swiftsoftware143@yahoo.com"])
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
        proxy_pass http://127.0.0.1:3001;
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
