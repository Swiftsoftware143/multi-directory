//! Network CRUD handlers.
//!
//! Networks group directories that share branding, theme, and root domain.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::*;
use crate::AppState;

/// Normalise a network root domain to its bare form (`https://X/` -> `x`), or `None` when the
/// value is blank. A non-blank value that is not a bare domain is a validation error.
///
/// Shared by the create/update editors and the dedicated `PUT /networks/:id/root-domain` endpoint
/// so the panel cannot save a value the subdomain-mapping feature could never use (B136: the
/// create form previously accepted `"not a domain"` while the dedicated endpoint rejected it).
fn normalize_root_domain(raw: Option<String>) -> Result<Option<String>, AppError> {
    let Some(raw) = raw else { return Ok(None) };
    let root = raw
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .replace("https://", "")
        .replace("http://", "");
    let root = root.split('/').next().unwrap_or("").to_string();
    if root.is_empty() {
        return Ok(None);
    }
    if !root.contains('.') || root.contains(' ') {
        return Err(AppError::Validation(
            "Enter a bare domain, e.g. zaarhub.com".into(),
        ));
    }
    Ok(Some(root))
}

/// GET /api/v1/networks
pub async fn list_networks(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let networks = sqlx::query_as::<_, Network>("SELECT * FROM networks ORDER BY created_at DESC")
        .fetch_all(&s.db)
        .await?;

    Ok(Json(json!(networks)))
}

/// Body for PUT /api/v1/networks/:id/root-domain.
#[derive(Debug, serde::Deserialize)]
pub struct RootDomainRequest {
    pub root_domain: String,
}

/// PUT /api/v1/networks/:id/root-domain — the domain a network's directories hang their
/// subdomains off (`palm-bay.<root_domain>`). Without it a network has no subdomain space, so
/// Feature 2 refuses subdomain mappings until it is set.
pub async fn set_root_domain(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<RootDomainRequest>,
) -> ApiResult<impl IntoResponse> {
    let root = normalize_root_domain(Some(req.root_domain))?
        .ok_or_else(|| AppError::Validation("Enter a bare domain, e.g. zaarhub.com".into()))?;

    let network = sqlx::query_as::<_, Network>(
        "UPDATE networks SET root_domain = $1, updated_at = NOW() WHERE id = $2 RETURNING *",
    )
    .bind(&root)
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Network '{}' not found", id)))?;

    Ok(Json(json!(network)))
}

/// GET /api/v1/networks/:id
pub async fn get_network(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let network = sqlx::query_as::<_, Network>("SELECT * FROM networks WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!("Network '{}' not found", id)))?;

    Ok(Json(json!(network)))
}

/// POST /api/v1/networks
pub async fn create_network(
    State(s): State<AppState>,
    Json(req): Json<CreateNetworkRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() || req.slug.is_empty() {
        return Err(AppError::Validation(
            "Name and slug are required".to_string(),
        ));
    }

    let root_domain = normalize_root_domain(req.root_domain.clone())?;

    // Check slug uniqueness
    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM networks WHERE slug = $1")
        .bind(&req.slug)
        .fetch_one(&s.db)
        .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!(
            "Network slug '{}' already exists",
            req.slug
        )));
    }

    let network = sqlx::query_as::<_, Network>(
        r#"INSERT INTO networks (name, slug, description, root_domain, status)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#,
    )
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(&root_domain)
    .bind(&req.status.unwrap_or_else(|| "active".to_string()))
    .fetch_one(&s.db)
    .await?;

    // Auto-create default branding for the network
    sqlx::query(
        r#"INSERT INTO network_branding (network_id)
           VALUES ($1)
           ON CONFLICT (network_id) DO NOTHING"#,
    )
    .bind(network.id)
    .execute(&s.db)
    .await?;

    // Provision CoreSwift tenant for this network
    let db = s.db.clone();
    let net_id = network.id;
    let net_name = req.name.clone();
    let net_slug = req.slug.clone();
    tokio::spawn(async move {
        match crate::coreswift::provision_tenant(&db, net_id, &net_name, &net_slug, true).await {
            Ok(_) => tracing::info!("[network] CoreSwift provisioned for {net_slug}"),
            Err(e) => tracing::warn!("[network] CoreSwift provisioning failed for {net_slug}: {e}"),
        }
    });

    Ok((StatusCode::CREATED, Json(json!(network))))
}

/// PUT /api/v1/networks/:id
pub async fn update_network(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateNetworkRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Network>("SELECT * FROM networks WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!("Network '{}' not found", id)))?;

    let new_name = req.name.unwrap_or(existing.name.clone());
    let new_slug = req.slug.unwrap_or(existing.slug.clone());
    let new_description = req.description.or(existing.description);
    let new_root_domain = match req.root_domain {
        Some(raw) => normalize_root_domain(Some(raw))?,
        None => existing.root_domain,
    };
    let new_status = req.status.or(existing.status);

    if new_slug != existing.slug {
        let slug_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM networks WHERE slug = $1 AND id != $2",
        )
        .bind(&new_slug)
        .bind(id)
        .fetch_one(&s.db)
        .await?;

        if slug_exists > 0 {
            return Err(AppError::Duplicate(format!(
                "Slug '{}' already in use",
                new_slug
            )));
        }
    }

    let network = sqlx::query_as::<_, Network>(
        r#"UPDATE networks
           SET name = $1, slug = $2, description = $3, root_domain = $4, status = $5, updated_at = NOW()
           WHERE id = $6
           RETURNING *"#
    )
    .bind(&new_name)
    .bind(&new_slug)
    .bind(&new_description)
    .bind(&new_root_domain)
    .bind(&new_status)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(network)))
}

/// DELETE /api/v1/networks/:id
///
/// `directories.network_id` is `ON DELETE SET NULL`, so an unguarded delete would silently
/// orphan every city in the network (and CASCADE would silently erase its branding, homepage,
/// ledger and treasury rows). Refuse while any directory still hangs off the network — the
/// operator re-homes or deletes the cities first, an honest failure instead of quiet data loss.
pub async fn delete_network(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let attached =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE network_id = $1")
            .bind(id)
            .fetch_one(&s.db)
            .await?;

    if attached > 0 {
        return Err(AppError::Validation(format!(
            "This network still has {attached} director{} attached. Re-home or delete them first — deleting would orphan every city.",
            if attached == 1 { "y" } else { "ies" }
        )));
    }

    let result = sqlx::query("DELETE FROM networks WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Network '{}' not found", id)));
    }

    Ok(Json(json!({"deleted": true})))
}

/// GET /api/v1/networks/:id/directories
pub async fn list_network_directories(
    State(s): State<AppState>,
    Path(network_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let directories = sqlx::query_as::<_, Directory>(
        r#"SELECT * FROM directories WHERE network_id = $1 ORDER BY created_at ASC"#,
    )
    .bind(network_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(directories)))
}

/// GET /api/v1/networks/:id/branding
pub async fn get_network_branding(
    State(s): State<AppState>,
    Path(network_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let branding = sqlx::query_as::<_, NetworkBranding>(
        "SELECT * FROM network_branding WHERE network_id = $1",
    )
    .bind(network_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!(
        "Branding for network '{}' not found",
        network_id
    )))?;

    Ok(Json(json!(branding)))
}

/// PUT /api/v1/networks/:id/branding
///
/// UPSERT, not UPDATE: a network created before the branding row existed (ZaarHub) has no
/// `network_branding` row, so a plain `UPDATE … RETURNING *` matched 0 rows and `fetch_one`
/// turned the save into an HTTP 500 — the branding editor was unusable for that network.
/// `network_id` is UNIQUE, so INSERT…ON CONFLICT creates the row on first save and COALESCE
/// still keeps whatever the caller left blank. (found by B91 gap #3 verification)
pub async fn update_network_branding(
    State(s): State<AppState>,
    Path(network_id): Path<Uuid>,
    Json(req): Json<UpdateNetworkBrandingRequest>,
) -> ApiResult<impl IntoResponse> {
    let branding = sqlx::query_as::<_, NetworkBranding>(
        r#"INSERT INTO network_branding
               (network_id, logo_url, logo_footer_url, favicon_url, primary_color,
                secondary_color, accent_color, background_color, text_color, heading_color,
                heading_font, body_font)
           VALUES ($12, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           ON CONFLICT (network_id) DO UPDATE SET
               logo_url = COALESCE(EXCLUDED.logo_url, network_branding.logo_url),
               logo_footer_url = COALESCE(EXCLUDED.logo_footer_url, network_branding.logo_footer_url),
               favicon_url = COALESCE(EXCLUDED.favicon_url, network_branding.favicon_url),
               primary_color = COALESCE(EXCLUDED.primary_color, network_branding.primary_color),
               secondary_color = COALESCE(EXCLUDED.secondary_color, network_branding.secondary_color),
               accent_color = COALESCE(EXCLUDED.accent_color, network_branding.accent_color),
               background_color = COALESCE(EXCLUDED.background_color, network_branding.background_color),
               text_color = COALESCE(EXCLUDED.text_color, network_branding.text_color),
               heading_color = COALESCE(EXCLUDED.heading_color, network_branding.heading_color),
               heading_font = COALESCE(EXCLUDED.heading_font, network_branding.heading_font),
               body_font = COALESCE(EXCLUDED.body_font, network_branding.body_font),
               updated_at = NOW()
           RETURNING *"#,
    )
    .bind(&req.logo_url)
    .bind(&req.logo_footer_url)
    .bind(&req.favicon_url)
    .bind(&req.primary_color)
    .bind(&req.secondary_color)
    .bind(&req.accent_color)
    .bind(&req.background_color)
    .bind(&req.text_color)
    .bind(&req.heading_color)
    .bind(&req.heading_font)
    .bind(&req.body_font)
    .bind(network_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(branding)))
}

#[cfg(test)]
mod root_domain_tests {
    use super::normalize_root_domain;

    #[test]
    fn bare_domain_is_kept() {
        assert_eq!(
            normalize_root_domain(Some("zaarhub.com".into())).unwrap(),
            Some("zaarhub.com".to_string())
        );
    }

    #[test]
    fn scheme_path_and_trailing_dot_are_stripped() {
        assert_eq!(
            normalize_root_domain(Some("https://ZaarHub.com/".into())).unwrap(),
            Some("zaarhub.com".to_string())
        );
    }

    #[test]
    fn blank_becomes_none() {
        assert_eq!(normalize_root_domain(Some("   ".into())).unwrap(), None);
        assert_eq!(normalize_root_domain(None).unwrap(), None);
    }

    #[test]
    fn junk_is_rejected() {
        assert!(normalize_root_domain(Some("not a domain".into())).is_err());
        assert!(normalize_root_domain(Some("localhost".into())).is_err());
    }
}
