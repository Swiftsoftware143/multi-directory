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
    let root = req
        .root_domain
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .replace("https://", "")
        .replace("http://", "");
    let root = root.split('/').next().unwrap_or("").to_string();
    if root.is_empty() || !root.contains('.') || root.contains(' ') {
        return Err(AppError::Validation(
            "Enter a bare domain, e.g. zaarhub.com".into(),
        ));
    }

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
    .bind(&req.root_domain)
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
    let new_root_domain = req.root_domain.or(existing.root_domain);
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
pub async fn delete_network(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
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
pub async fn update_network_branding(
    State(s): State<AppState>,
    Path(network_id): Path<Uuid>,
    Json(req): Json<UpdateNetworkBrandingRequest>,
) -> ApiResult<impl IntoResponse> {
    let branding = sqlx::query_as::<_, NetworkBranding>(
        r#"UPDATE network_branding
           SET logo_url = COALESCE($1, logo_url),
               logo_footer_url = COALESCE($2, logo_footer_url),
               favicon_url = COALESCE($3, favicon_url),
               primary_color = COALESCE($4, primary_color),
               secondary_color = COALESCE($5, secondary_color),
               accent_color = COALESCE($6, accent_color),
               background_color = COALESCE($7, background_color),
               text_color = COALESCE($8, text_color),
               heading_color = COALESCE($9, heading_color),
               heading_font = COALESCE($10, heading_font),
               body_font = COALESCE($11, body_font),
               updated_at = NOW()
           WHERE network_id = $12
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
