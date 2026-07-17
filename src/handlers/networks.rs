//! Network CRUD and homepage section handlers.
//!
//! Networks group directories that share branding, theme, and root domain.
//! Homepage sections belong to either a network (shared) or a directory (standalone).

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

/// GET /api/v1/networks
pub async fn list_networks(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let networks = sqlx::query_as::<_, Network>(
        "SELECT * FROM networks ORDER BY created_at DESC"
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(networks)))
}

/// GET /api/v1/networks/:id
pub async fn get_network(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let network = sqlx::query_as::<_, Network>(
        "SELECT * FROM networks WHERE id = $1"
    )
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
        return Err(AppError::Validation("Name and slug are required".to_string()));
    }

    // Check slug uniqueness
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM networks WHERE slug = $1"
    )
    .bind(&req.slug)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!("Network slug '{}' already exists", req.slug)));
    }

    let network = sqlx::query_as::<_, Network>(
        r#"INSERT INTO networks (name, slug, description, root_domain, status)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING *"#
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
           ON CONFLICT (network_id) DO NOTHING"#
    )
    .bind(network.id)
    .execute(&s.db)
    .await?;

    // Auto-create a default hero section for the network homepage
    sqlx::query(
        r#"INSERT INTO homepage_sections (network_id, section_type, sort_order, title)
           VALUES ($1, 'hero', 0, $2)"#
    )
    .bind(network.id)
    .bind(&req.name)
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
    let existing = sqlx::query_as::<_, Network>(
        "SELECT * FROM networks WHERE id = $1"
    )
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
            "SELECT COUNT(*) FROM networks WHERE slug = $1 AND id != $2"
        )
        .bind(&new_slug)
        .bind(id)
        .fetch_one(&s.db)
        .await?;

        if slug_exists > 0 {
            return Err(AppError::Duplicate(format!("Slug '{}' already in use", new_slug)));
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
        r#"SELECT * FROM directories WHERE network_id = $1 ORDER BY created_at ASC"#
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
        "SELECT * FROM network_branding WHERE network_id = $1"
    )
    .bind(network_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Branding for network '{}' not found", network_id)))?;

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
           RETURNING *"#
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

// ── Homepage Sections ────────────────────────────────────────────────────────

/// GET /api/v1/networks/:id/homepage
pub async fn get_network_homepage(
    State(s): State<AppState>,
    Path(network_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let sections = sqlx::query_as::<_, HomepageSection>(
        "SELECT * FROM homepage_sections WHERE network_id = $1 AND is_active = true ORDER BY sort_order ASC"
    )
    .bind(network_id)
    .fetch_all(&s.db)
    .await?;

    // Also include the network's branding
    let branding = sqlx::query_as::<_, NetworkBranding>(
        "SELECT * FROM network_branding WHERE network_id = $1"
    )
    .bind(network_id)
    .fetch_optional(&s.db)
    .await?;

    let network = sqlx::query_as::<_, Network>(
        "SELECT * FROM networks WHERE id = $1"
    )
    .bind(network_id)
    .fetch_optional(&s.db)
    .await;

    Ok(Json(json!({
        "sections": sections,
        "branding": branding,
        "network": network.ok().flatten(),
    })))
}

/// GET /api/v1/directories/:slug/homepage
pub async fn get_directory_homepage(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    // If directory belongs to a network, return the network's homepage
    if let Some(network_id) = directory.network_id {
        let sections = sqlx::query_as::<_, HomepageSection>(
            "SELECT * FROM homepage_sections WHERE network_id = $1 AND is_active = true ORDER BY sort_order ASC"
        )
        .bind(network_id)
        .fetch_all(&s.db)
        .await?;

        let branding = sqlx::query_as::<_, NetworkBranding>(
            "SELECT * FROM network_branding WHERE network_id = $1"
        )
        .bind(network_id)
        .fetch_optional(&s.db)
        .await?;

        let network = sqlx::query_as::<_, Network>(
            "SELECT * FROM networks WHERE id = $1"
        )
        .bind(network_id)
        .fetch_optional(&s.db)
        .await?;

        return Ok(Json(json!({
            "directory": directory,
            "sections": sections,
            "branding": branding,
            "network": network,
            "shared": true,
        })));
    }

    // Standalone directory — return its own homepage sections
    let sections = sqlx::query_as::<_, HomepageSection>(
        "SELECT * FROM homepage_sections WHERE directory_id = $1 AND is_active = true ORDER BY sort_order ASC"
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "directory": directory,
        "sections": sections,
        "network": null,
        "branding": null,
        "shared": false,
    })))
}

/// POST /api/v1/homepage-sections
pub async fn create_homepage_section(
    State(s): State<AppState>,
    Json(req): Json<CreateHomepageSectionRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.network_id.is_none() && req.directory_id.is_none() {
        return Err(AppError::Validation("Either network_id or directory_id is required".to_string()));
    }
    if req.network_id.is_some() && req.directory_id.is_some() {
        return Err(AppError::Validation("Cannot set both network_id and directory_id — use one or the other".to_string()));
    }
    if req.section_type.is_empty() {
        return Err(AppError::Validation("section_type is required".to_string()));
    }

    let section = sqlx::query_as::<_, HomepageSection>(
        r#"INSERT INTO homepage_sections (network_id, directory_id, section_type, sort_order, title, subtitle, content, cta_text, cta_url, image_url, is_active)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING *"#
    )
    .bind(&req.network_id)
    .bind(&req.directory_id)
    .bind(&req.section_type)
    .bind(&req.sort_order.unwrap_or(0))
    .bind(&req.title)
    .bind(&req.subtitle)
    .bind(&req.content)
    .bind(&req.cta_text)
    .bind(&req.cta_url)
    .bind(&req.image_url)
    .bind(&req.is_active.unwrap_or(true))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(section))))
}

/// PUT /api/v1/homepage-sections/:id
pub async fn update_homepage_section(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateHomepageSectionRequest>,
) -> ApiResult<impl IntoResponse> {
    let section = sqlx::query_as::<_, HomepageSection>(
        r#"UPDATE homepage_sections
           SET section_type = COALESCE($1, section_type),
               sort_order = COALESCE($2, sort_order),
               title = COALESCE($3, title),
               subtitle = COALESCE($4, subtitle),
               content = COALESCE($5, content),
               cta_text = COALESCE($6, cta_text),
               cta_url = COALESCE($7, cta_url),
               image_url = COALESCE($8, image_url),
               is_active = COALESCE($9, is_active),
               updated_at = NOW()
           WHERE id = $10
           RETURNING *"#
    )
    .bind(&req.section_type)
    .bind(&req.sort_order)
    .bind(&req.title)
    .bind(&req.subtitle)
    .bind(&req.content)
    .bind(&req.cta_text)
    .bind(&req.cta_url)
    .bind(&req.image_url)
    .bind(&req.is_active)
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Homepage section '{}' not found", id)))?;

    Ok(Json(json!(section)))
}

/// DELETE /api/v1/homepage-sections/:id
pub async fn get_homepage_section(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let section = sqlx::query_as::<_, HomepageSection>(
        "SELECT * FROM homepage_sections WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Homepage section '{}' not found", id)))?;

    Ok(Json(json!(section)))
}

pub async fn delete_homepage_section(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM homepage_sections WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Homepage section '{}' not found", id)));
    }

    Ok(Json(json!({"deleted": true})))
}
