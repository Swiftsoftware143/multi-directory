//! Directory and category CRUD handlers.
//!
//! Updated with template engine support:
//! - Create/Update accept template + color_scheme
//! - GET /api/v1/directories/:slug/render returns HTML rendered with template
//! - GET /api/v1/directories/:slug/preview returns template preview

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::{validate_pagination, ApiResult, AppError};
use crate::handlers::tenant_scope;
use crate::models::*;
use crate::template_engine;
use crate::tracking_script;
use crate::AppState;

/// GET /api/v1/directories
pub async fn list_directories(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let page = params
        .get("page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    // B120 — SELLABLE STANDARD: an ACTIVATED directory belongs to the buyer's tenant, and that
    // tenant must see only its own directories. A tenant that has bought a directory is a
    // directory operator and is scoped strictly to what it owns; the platform operator keeps the
    // full list. A tenant that owns NO directory (business owners in the platform tenant, the
    // shared /portal picker) is left unchanged — every shared city has `owner_id IS NULL`, so
    // scoping them would empty those pickers.
    let scope_tid = match tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret) {
        Ok(claims) if !tenant_scope::is_platform_operator(&claims) => {
            let tid = tenant_scope::caller_tenant(&claims)?;
            let owns = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM directories d JOIN users u ON u.id = d.owner_id \
                 WHERE u.tenant_id = $1)",
            )
            .bind(tid)
            .fetch_one(&s.db)
            .await?;
            if owns {
                Some(tid)
            } else {
                None
            }
        }
        _ => None,
    };

    let (total, directories) = if let Some(tid) = scope_tid {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM directories WHERE owner_id IN \
             (SELECT id FROM users WHERE tenant_id = $1)",
        )
        .bind(tid)
        .fetch_one(&s.db)
        .await?;
        let rows = sqlx::query_as::<_, Directory>(
            "SELECT * FROM directories WHERE owner_id IN \
             (SELECT id FROM users WHERE tenant_id = $1) ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(tid)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await?;
        (total, rows)
    } else {
        let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories")
            .fetch_one(&s.db)
            .await?;
        let rows = sqlx::query_as::<_, Directory>(
            "SELECT * FROM directories ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 ",
        )
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await?;
        (total, rows)
    };

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!(PaginatedResponse {
        data: directories,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// GET /api/v1/directories/:slug
pub async fn get_directory(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    // B120 — a directory-owning tenant (a BUYER) may read only its own directories: the shared
    // platform cities are not its business. A tenant that owns no directory (business owners in
    // the platform tenant) keeps the existing read access, so their portal pickers still work.
    let claims = tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    if !tenant_scope::is_platform_operator(&claims) {
        let tid = tenant_scope::caller_tenant(&claims)?;
        let owns = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM directories d JOIN users u ON u.id = d.owner_id \
             WHERE u.tenant_id = $1)",
        )
        .bind(tid)
        .fetch_one(&s.db)
        .await?;
        if owns && !tenant_scope::can_admin_directory(&s.db, &claims, directory.id).await? {
            return Err(AppError::NotFound(format!(
                "Directory '{}' not found",
                slug
            )));
        }
    }

    Ok(Json(json!(directory)))
}

/// POST /api/v1/directories
pub async fn create_directory(
    State(s): State<AppState>,
    Json(req): Json<CreateDirectoryRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() {
        return Err(AppError::Validation(
            "Directory name is required".to_string(),
        ));
    }

    // Auto-generate slug if not provided
    let slug = match &req.slug {
        Some(s) if !s.is_empty() => s.clone(),
        _ => req
            .name
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
            .replace(' ', "-")
            .chars()
            .take(80)
            .collect::<String>(),
    };

    if slug.is_empty() {
        return Err(AppError::Validation(
            "Slug is required (auto-generation failed)".to_string(),
        ));
    }

    // Check slug uniqueness
    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&s.db)
        .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!(
            "Directory slug '{}' already exists",
            slug
        )));
    }

    let template = req
        .template
        .as_deref()
        .unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let template = if template_engine::is_valid_template(template) {
        template
    } else {
        template_engine::TEMPLATE_LOCAL_BUSINESS
    };

    // Determine network mode
    let network_mode = req.network_mode.as_deref().unwrap_or("standalone");
    let (network_id, url_type, url_value, custom_domain) =
        resolve_network_config(&s, &req, &slug, network_mode).await?;

    let template_config = req.template_config.clone().unwrap_or_default();

    // Color scheme: use provided, inherit from network, or default
    let color_scheme = if let Some(cs) = req.color_scheme.clone() {
        cs
    } else if network_mode == "connect" {
        if let Some(nid) = network_id {
            let nb = sqlx::query_as::<_, crate::models::NetworkBranding>(
                "SELECT * FROM network_branding WHERE network_id = $1",
            )
            .bind(nid)
            .fetch_optional(&s.db)
            .await?;
            if let Some(ref b) = nb {
                serde_json::json!({
                    "primary": b.primary_color.as_deref().unwrap_or("#2563eb"),
                    "secondary": b.secondary_color.as_deref().unwrap_or("#64748b"),
                    "accent": b.accent_color.as_deref().unwrap_or("#f59e0b"),
                    "background": b.background_color.as_deref().unwrap_or("#ffffff"),
                    "text": b.text_color.as_deref().unwrap_or("#1e293b"),
                    "heading": b.heading_color.as_deref().unwrap_or("#0f172a"),
                })
            } else {
                template_engine::default_color_scheme()
            }
        } else {
            template_engine::default_color_scheme()
        }
    } else {
        template_engine::default_color_scheme()
    };

    let mut directory = sqlx::query_as::<_, Directory>(
        r#"INSERT INTO directories (name, slug, description, status, template, color_scheme, network_id, url_type, url_value, custom_domain, city, template_config, head_injection, body_injection, footer_injection, email_signature_html, email_signature_text, state, support_email, contact_email, contact_phone, legal_name)
           VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10, $11, $12::jsonb, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)
           RETURNING *"#
    )
    .bind(&req.name)
    .bind(&slug)
    .bind(&req.description)
    .bind(&req.status.unwrap_or_else(|| "draft".to_string()))
    .bind(template)
    .bind(&color_scheme.to_string())
    .bind(&network_id)
    .bind(&url_type)
    .bind(&url_value)
    .bind(&custom_domain)
    .bind(&req.city)
    .bind(&template_config.to_string())
    .bind(&req.head_injection)
    .bind(&req.body_injection)
    .bind(&req.footer_injection)
    .bind(&req.email_signature_html)
    .bind(&req.email_signature_text)
    .bind(&req.state)
    .bind(&req.support_email)
    .bind(&req.contact_email)
    .bind(&req.contact_phone)
    .bind(&req.legal_name)
    .fetch_one(&s.db)
    .await?;

    // If network_mode="new_network", create the network and link it
    if network_mode == "new_network" {
        let network_slug = format!("network-{}", &slug);
        let network = sqlx::query_as::<_, crate::models::Network>(
            r#"INSERT INTO networks (name, slug, description, root_domain)
               VALUES ($1, $2, $3, $4)
               RETURNING *"#,
        )
        .bind(&req.name)
        .bind(&network_slug)
        .bind(&req.description)
        .bind(&custom_domain)
        .fetch_one(&s.db)
        .await?;

        // Create default branding for the network
        sqlx::query(
            r#"INSERT INTO network_branding (network_id, primary_color, secondary_color, accent_color, background_color, text_color, heading_color)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (network_id) DO NOTHING"#
        )
        .bind(network.id)
        .bind(color_scheme.get("primary").and_then(|v| v.as_str()).unwrap_or("#2563eb"))
        .bind(color_scheme.get("secondary").and_then(|v| v.as_str()).unwrap_or("#64748b"))
        .bind(color_scheme.get("accent").and_then(|v| v.as_str()).unwrap_or("#f59e0b"))
        .bind(color_scheme.get("background").and_then(|v| v.as_str()).unwrap_or("#ffffff"))
        .bind(color_scheme.get("text").and_then(|v| v.as_str()).unwrap_or("#1e293b"))
        .bind(color_scheme.get("heading").and_then(|v| v.as_str()).unwrap_or("#0f172a"))
        .execute(&s.db)
        .await?;

        // Link directory to the new network
        sqlx::query("UPDATE directories SET network_id = $1 WHERE id = $2")
            .bind(network.id)
            .bind(directory.id)
            .execute(&s.db)
            .await?;

        // Re-fetch directory to get updated network_id
        directory = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE id = $1")
            .bind(directory.id)
            .fetch_one(&s.db)
            .await?;

        // For new networks, provision the network tenant + directory resources
        let db2 = s.db.clone();
        let dir_id2 = directory.id;
        let dir_name2 = req.name.clone();
        let dir_slug2 = slug.clone();
        tokio::spawn(async move {
            // First provision the network tenant
            match crate::coreswift::provision_tenant(&db2, dir_id2, &dir_name2, &dir_slug2, true)
                .await
            {
                Ok(_) => tracing::info!(
                    "[directory] CoreSwift network tenant provisioned for {dir_slug2}"
                ),
                Err(e) => {
                    tracing::warn!("[directory] CoreSwift network tenant provisioning failed: {e}")
                }
            }
            // Then provision directory resources (booking calendar, tags, etc.)
            match crate::coreswift::provision_directory_resources(&db2, dir_id2, &dir_slug2).await {
                Ok(prefix) => tracing::info!(
                    "[directory] CoreSwift resources provisioned for {dir_slug2} (prefix={prefix})"
                ),
                Err(e) => tracing::warn!(
                    "[directory] CoreSwift resource provisioning failed for {dir_slug2}: {e}"
                ),
            }
        });
    }

    // Provision CoreSwift tenant + all resources for standalone directories
    if network_mode == "standalone" {
        let db = s.db.clone();
        let dir_id = directory.id;
        let dir_name = req.name.clone();
        let dir_slug = slug.clone();
        tokio::spawn(async move {
            match crate::coreswift::provision_tenant(&db, dir_id, &dir_name, &dir_slug, false).await
            {
                Ok(_) => {
                    tracing::info!("[directory] CoreSwift tenant provisioned for {dir_slug}");
                    match crate::coreswift::provision_directory_resources(&db, dir_id, &dir_slug).await {
                        Ok(prefix) => tracing::info!("[directory] CoreSwift resources provisioned for {dir_slug} (prefix={prefix})"),
                        Err(e) => tracing::warn!("[directory] CoreSwift resource provisioning failed for {dir_slug}: {e}"),
                    }
                }
                Err(e) => {
                    tracing::warn!("[directory] CoreSwift provisioning failed for {dir_slug}: {e}")
                }
            }
        });
    }

    // B118 — DEFAULTS OVER BLANKS: a directory is complete the moment it exists. Fill the
    // per-directory defaults (standard ad zones, and system email templates for a standalone
    // directory) so the operator never has to seed them by hand. Non-fatal: a seeding failure
    // is logged and the directory is still created.
    let report = provision_directory_defaults(&s.db, directory.id).await;
    tracing::info!(
        "[directory] provisioned defaults for {}: {} ad zone(s), {} email template(s)",
        slug,
        report.ad_zones_created,
        report.email_templates_created
    );

    Ok((StatusCode::CREATED, Json(json!(directory))))
}

/// Resolve network config for a directory being created.
async fn resolve_network_config(
    s: &AppState,
    req: &CreateDirectoryRequest,
    slug: &str,
    network_mode: &str,
) -> ApiResult<(Option<Uuid>, Option<String>, Option<String>, Option<String>)> {
    match network_mode {
        "connect" => {
            let network_id = req.parent_network_id.ok_or(AppError::Validation(
                "parent_network_id is required when network_mode='connect'".to_string(),
            ))?;

            let network_exists =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM networks WHERE id = $1")
                    .bind(network_id)
                    .fetch_one(&s.db)
                    .await?;

            if network_exists == 0 {
                return Err(AppError::NotFound(format!(
                    "Network '{}' not found",
                    network_id
                )));
            }

            let url_type = req
                .url_type
                .clone()
                .unwrap_or_else(|| "subfolder".to_string());
            let url_value = req.url_value.clone().unwrap_or_else(|| slug.to_string());
            let custom_domain = req.custom_domain.clone();

            Ok((
                Some(network_id),
                Some(url_type),
                Some(url_value),
                custom_domain,
            ))
        }
        "new_network" => {
            let url_type = req
                .url_type
                .clone()
                .unwrap_or_else(|| "standalone".to_string());
            let url_value = req.url_value.clone().or_else(|| Some(slug.to_string()));
            let custom_domain = req.custom_domain.clone();
            Ok((None, Some(url_type), url_value, custom_domain))
        }
        _ => {
            // Standalone
            Ok((
                None,
                Some("standalone".to_string()),
                Some(slug.to_string()),
                req.custom_domain.clone(),
            ))
        }
    }
}

/// PUT /api/v1/directories/:slug
pub async fn update_directory(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(req): Json<UpdateDirectoryRequest>,
) -> ApiResult<impl IntoResponse> {
    // B120 — only the platform operator or the directory's own owning tenant may edit it.
    let claims = tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    tenant_scope::assert_directory_admin_by_slug(&s.db, &claims, &slug).await?;

    let existing = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    let new_name = req.name.unwrap_or(existing.name);
    let new_slug = req.slug.unwrap_or(existing.slug.clone());
    let new_description = req.description.or(existing.description);
    let new_status = req.status.or(existing.status);
    let new_template = req.template.unwrap_or(
        existing
            .template
            .unwrap_or_else(|| template_engine::TEMPLATE_LOCAL_BUSINESS.to_string()),
    );
    let new_color_scheme = req.color_scheme.or(existing.color_scheme);
    let new_network_id = req.network_id.or(existing.network_id);
    let new_url_type = req.url_type.or(existing.url_type);
    let new_url_value = req.url_value.or(existing.url_value);
    let new_custom_domain = req.custom_domain.or(existing.custom_domain);
    let new_city = req.city.or(existing.city);
    let new_state = req.state.or(existing.state);
    let new_support_email = req.support_email.or(existing.support_email);
    let new_contact_email = req.contact_email.or(existing.contact_email);
    let new_contact_phone = req.contact_phone.or(existing.contact_phone);
    let new_legal_name = req.legal_name.or(existing.legal_name);
    let new_head_injection = req
        .head_injection
        .clone()
        .or(existing.head_injection.clone());
    let new_body_injection = req
        .body_injection
        .clone()
        .or(existing.body_injection.clone());
    let new_footer_injection = req
        .footer_injection
        .clone()
        .or(existing.footer_injection.clone());
    let new_template_config = req.template_config.clone().or(existing.template_config);
    let new_email_signature_html = req
        .email_signature_html
        .clone()
        .or(existing.email_signature_html);
    let new_email_signature_text = req
        .email_signature_text
        .clone()
        .or(existing.email_signature_text);

    if new_slug != slug {
        let slug_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM directories WHERE slug = $1 AND id != $2",
        )
        .bind(&new_slug)
        .bind(existing.id)
        .fetch_one(&s.db)
        .await?;

        if slug_exists > 0 {
            return Err(AppError::Duplicate(format!(
                "Slug '{}' already in use",
                new_slug
            )));
        }
    }

    let directory = sqlx::query_as::<_, Directory>(
        "UPDATE directories SET name = $1, slug = $2, description = $3, status = $4, template = $5, color_scheme = $6::jsonb, network_id = $7, url_type = $8, url_value = $9, custom_domain = $10, city = $11, template_config = $12::jsonb, head_injection = $14, body_injection = $15, footer_injection = $16, email_signature_html = $17, email_signature_text = $18, state = $19, support_email = $20, contact_email = $21, contact_phone = $22, legal_name = $23, updated_at = NOW() WHERE id = $13 RETURNING *"
    )
    .bind(&new_name)
    .bind(&new_slug)
    .bind(&new_description)
    .bind(&new_status)
    .bind(&new_template)
    // B120 — bind `Option<String>` so a NULL color_scheme/template_config stays SQL NULL.
    // Binding `""` (the old unwrap_or_default) is invalid JSON and made every PUT on a
    // directory with a NULL jsonb column fail with `invalid input syntax for type json`.
    .bind(new_color_scheme.as_ref().map(|v| v.to_string()))
    .bind(&new_network_id)
    .bind(&new_url_type)
    .bind(&new_url_value)
    .bind(&new_custom_domain)
    .bind(&new_city)
    .bind(new_template_config.as_ref().map(|v| v.to_string()))
    .bind(existing.id)
    .bind(&new_head_injection)
    .bind(&new_body_injection)
    .bind(&new_footer_injection)
    .bind(&new_email_signature_html)
    .bind(&new_email_signature_text)
    .bind(&new_state)
    .bind(&new_support_email)
    .bind(&new_contact_email)
    .bind(&new_contact_phone)
    .bind(&new_legal_name)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(directory)))
}

/// DELETE /api/v1/directories/:slug
pub async fn delete_directory(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    // B120 — destroying a directory is the platform operator's call alone.
    let claims = tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    if !tenant_scope::is_platform_operator(&claims) {
        return Err(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )));
    }

    let result = sqlx::query("DELETE FROM directories WHERE slug = \x241")
        .bind(&slug)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )));
    }

    Ok((
        StatusCode::OK,
        Json(json!({"message": "Directory deleted successfully"})),
    ))
}

/// POST /api/v1/directories/:id/primary — make this directory its network's PRIMARY directory.
///
/// B97: "the main directory admin = the first city" was an IMPLICIT convention based on
/// created_at ordering — fragile, and confusing to a buyer. This endpoint makes the choice
/// explicit and admin-settable: exactly one holder per network, defaulting to the first-created
/// directory (seeded by migration 132). A standalone directory has no network, so there is
/// nothing to delegate to → 400.
pub async fn set_primary_directory(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    tenant_scope::assert_directory_admin(&s.db, &claims, id).await?;

    let network_id =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT network_id FROM directories WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("directory not found".to_string()))?;

    let Some(network_id) = network_id else {
        return Err(AppError::BadRequest(
            "This directory is not part of a network, so it has no primary to set.".to_string(),
        ));
    };

    // Clear first, then set, in one transaction: the partial unique index
    // `directories_one_primary_per_network` must never observe two primaries at once.
    let mut tx = s.db.begin().await?;
    sqlx::query("UPDATE directories SET is_primary = false WHERE network_id = $1 AND is_primary")
        .bind(network_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE directories SET is_primary = true, updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let directory = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE id = $1")
        .bind(id)
        .fetch_one(&s.db)
        .await?;

    Ok(Json(json!({ "ok": true, "directory": directory })))
}

/// GET /api/v1/directories/:slug/render — render directory page with template
pub async fn render_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    let categories = sqlx::query_as::<_, DirectoryCategory>(
        "SELECT * FROM directory_categories WHERE directory_id = \x241 ORDER BY sort_order ASC, name ASC "
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    let businesses = sqlx::query_as::<_, Business>(
        "SELECT * FROM businesses WHERE directory_id = \x241 AND is_active = true ORDER BY rating DESC NULLS LAST, name ASC "
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    // Load business meta
    let mut meta_map = HashMap::new();
    for biz in &businesses {
        if let Ok(meta) = sqlx::query_as::<_, BusinessMeta>(
            "SELECT * FROM business_meta WHERE business_id = \x241 AND template = \x242 ",
        )
        .bind(biz.id)
        .bind(
            directory
                .template
                .as_deref()
                .unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS),
        )
        .fetch_optional(&s.db)
        .await
        {
            if let Some(m) = meta {
                meta_map.insert(biz.id, m.meta_data);
            }
        }
    }

    let template_id = directory
        .template
        .as_deref()
        .unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let cats_val = serde_json::to_value(&categories).unwrap_or_default();
    let biz_val = serde_json::to_value(&businesses).unwrap_or_default();
    let ctx = template_engine::build_template_context(&dir_val, &biz_val, &cats_val, None, None);
    let html = engine
        .render_directory_page(template_id, &ctx)
        .map_err(|e| AppError::Internal(e))?;

    // Inject visitor tracking script into directory page
    let mut output = if directory.tracking_enabled.unwrap_or(true) {
        crate::tracking_script::inject_tracking_script(&html)
    } else {
        html
    };

    // Inject custom head / body / footer code
    if let Some(ref hi) = directory.head_injection {
        if !hi.trim().is_empty() {
            let safe_hi = crate::template_engine::sanitize_html(hi);
            output = output.replace("</head>", &format!("\n{}\n</head>", safe_hi));
        }
    }
    if let Some(ref bi) = directory.body_injection {
        if !bi.trim().is_empty() {
            let safe_bi = crate::template_engine::sanitize_html(bi);
            output = output.replace("<body", &format!("\n{}\n<body", safe_bi));
        }
    }
    if let Some(ref fi) = directory.footer_injection {
        if !fi.trim().is_empty() {
            let safe_fi = crate::template_engine::sanitize_html(fi);
            output = output.replace("</body>", &format!("\n{}\n</body>", safe_fi));
        }
    }

    // Inject survey widget if onboarding_survey is enabled in feature_config
    if let Some(ref fc) = directory.feature_config {
        if fc
            .get("onboarding_survey")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let survey_tag = "<script src=\"/survey-widget.js\"></script>";
            output = output.replace("</head>", &format!("\n{}\n</head>", survey_tag));
        }
    }

    Ok(axum::response::Html(output))
}

/// GET /api/v1/templates — list available templates
pub async fn list_templates() -> ApiResult<impl IntoResponse> {
    let templates = template_engine::get_available_templates();
    Ok(Json(json!(templates)))
}

// ── Categories ───────────────────────────────────────────────────────────────

/// GET /api/v1/directories/:slug/categories
/// Returns categories with optional parent_name for display
pub async fn list_categories(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    let categories = sqlx::query_as::<_, DirectoryCategoryWithParent>(
        "SELECT dc.id, dc.directory_id, dc.name, dc.slug, dc.sort_order, dc.parent_id, p.name as parent_name FROM directory_categories dc LEFT JOIN directory_categories p ON p.id = dc.parent_id WHERE dc.directory_id = \x241 ORDER BY dc.sort_order ASC, dc.name ASC"
    )
    .bind(dir.id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(categories)))
}

/// POST /api/v1/directories/:slug/categories
pub async fn create_category(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() || req.slug.is_empty() {
        return Err(AppError::Validation(
            "Name and slug are required".to_string(),
        ));
    }

    let dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    let category = sqlx::query_as::<_, DirectoryCategory>(
        "INSERT INTO directory_categories (directory_id, name, slug, sort_order, parent_id) VALUES (\x241, \x242, \x243, \x244, \x245) RETURNING *"
    )
    .bind(dir.id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(req.sort_order.unwrap_or(0))
    .bind(req.parent_id)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(category))))
}

/// PUT /api/v1/directories/:slug/categories/:category_id
pub async fn update_category(
    State(s): State<AppState>,
    Path((slug, category_id)): Path<(String, Uuid)>,
    Json(req): Json<UpdateCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    let existing = sqlx::query_as::<_, DirectoryCategory>(
        "SELECT * FROM directory_categories WHERE id = \x241 AND directory_id = \x242 ",
    )
    .bind(category_id)
    .bind(dir.id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Category not found".to_string()))?;

    let new_name = req.name.unwrap_or(existing.name);
    let new_slug = req.slug.unwrap_or(existing.slug);
    let new_sort_order = req.sort_order.unwrap_or(existing.sort_order.unwrap_or(0));

    // Prevent setting parent_id to self
    if req.parent_id == Some(category_id) {
        return Err(AppError::Validation(
            "A category cannot be its own parent".to_string(),
        ));
    }

    let new_parent_id = req.parent_id.or(existing.parent_id);

    let category = sqlx::query_as::<_, DirectoryCategory>(
        "UPDATE directory_categories SET name = \x241, slug = \x242, sort_order = \x243, parent_id = \x244
           WHERE id = \x245 RETURNING *"
    )
    .bind(&new_name)
    .bind(&new_slug)
    .bind(new_sort_order)
    .bind(new_parent_id)
    .bind(category_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(category)))
}

/// DELETE /api/v1/directories/:slug/categories/:category_id
/// Supports ?force=true&reassign_to=UUID query params for safe delete with reassign
pub async fn delete_category(
    State(s): State<AppState>,
    Path((slug, category_id)): Path<(String, Uuid)>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let _dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    // Check for existing businesses
    let business_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM businesses WHERE category_id = \x241")
            .bind(category_id)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);

    // Check for subcategories
    let subcategory_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM directory_categories WHERE parent_id = \x241")
            .bind(category_id)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);

    let force = params.get("force").map(|v| v == "true").unwrap_or(false);

    if !force && (business_count > 0 || subcategory_count > 0) {
        return Err(AppError::Validation(format!(
            "Cannot delete category: {} business(es) and {} subcategor(ies) depend on it. Use ?force=true&reassign_to=UUID to reassign, or ?force=true without reassign_to to delete dependent records.",
            business_count, subcategory_count
        )));
    }

    if force && business_count > 0 {
        if let Some(reassign_to) = params
            .get("reassign_to")
            .and_then(|v| Uuid::parse_str(v).ok())
        {
            // Reassign businesses to target category
            sqlx::query("UPDATE businesses SET category_id = \x241 WHERE category_id = \x242")
                .bind(reassign_to)
                .bind(category_id)
                .execute(&s.db)
                .await?;
        } else {
            // Delete all businesses in this category
            sqlx::query("DELETE FROM businesses WHERE category_id = \x241")
                .bind(category_id)
                .execute(&s.db)
                .await?;
        }
    }

    // If force and reassign_to for subcategories, move subcategories up to parent's parent
    if force && subcategory_count > 0 {
        let parent_of_deleted = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT parent_id FROM directory_categories WHERE id = \x241",
        )
        .bind(category_id)
        .fetch_optional(&s.db)
        .await?
        .flatten();

        sqlx::query("UPDATE directory_categories SET parent_id = \x241 WHERE parent_id = \x242")
            .bind(parent_of_deleted)
            .bind(category_id)
            .execute(&s.db)
            .await?;
    }

    // Clear category_id from visitor_events
    sqlx::query("UPDATE visitor_events SET category_id = NULL WHERE category_id = \x241")
        .bind(category_id)
        .execute(&s.db)
        .await?;

    // Now delete the category itself
    let cur = sqlx::query("DELETE FROM directory_categories WHERE id = \x241")
        .bind(category_id)
        .execute(&s.db)
        .await?;

    if cur.rows_affected() == 0 {
        return Err(AppError::NotFound("Category not found".to_string()));
    }

    Ok(Json(json!({"message": "Category deleted successfully"})))
}

/// POST /api/v1/directories/:slug/categories/bulk-move
pub async fn categories_bulk_move(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<BulkMoveRequest>,
) -> ApiResult<impl IntoResponse> {
    let _dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    if req.category_ids.is_empty() {
        return Err(AppError::Validation("No category IDs provided".to_string()));
    }

    let mut affected = 0usize;

    if req.move_businesses {
        let result =
            sqlx::query("UPDATE businesses SET category_id = \x241 WHERE category_id = ANY(\x242)")
                .bind(req.target_category_id)
                .bind(&req.category_ids)
                .execute(&s.db)
                .await?;
        affected += result.rows_affected() as usize;
    }

    if req.move_subcategories {
        let result =
            sqlx::query("UPDATE directory_categories SET parent_id = \x241 WHERE id = ANY(\x242)")
                .bind(req.target_category_id)
                .bind(&req.category_ids)
                .execute(&s.db)
                .await?;
        affected += result.rows_affected() as usize;
    }

    Ok(Json(json!(CategoryBulkResult {
        success: true,
        message: "Bulk move completed".to_string(),
        affected_categories: req.category_ids.len(),
    })))
}

/// POST /api/v1/directories/:slug/categories/bulk-delete
pub async fn categories_bulk_delete(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<BulkDeleteCategoriesRequest>,
) -> ApiResult<impl IntoResponse> {
    let _dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    if req.category_ids.is_empty() {
        return Err(AppError::Validation("No category IDs provided".to_string()));
    }

    if let Some(reassign_to) = req.reassign_to {
        // Reassign businesses to target category
        sqlx::query("UPDATE businesses SET category_id = \x241 WHERE category_id = ANY(\x242)")
            .bind(reassign_to)
            .bind(&req.category_ids)
            .execute(&s.db)
            .await?;

        // Move subcategories up
        sqlx::query(
            "UPDATE directory_categories SET parent_id = NULL WHERE parent_id = ANY(\x241)",
        )
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;
    } else {
        // Delete all businesses in these categories
        sqlx::query("DELETE FROM businesses WHERE category_id = ANY(\x241)")
            .bind(&req.category_ids)
            .execute(&s.db)
            .await?;
    }

    // Clear category_id from visitor_events
    sqlx::query("UPDATE visitor_events SET category_id = NULL WHERE category_id = ANY(\x241)")
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;

    // Delete the categories
    sqlx::query("DELETE FROM directory_categories WHERE id = ANY(\x241)")
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;

    Ok(Json(json!(CategoryBulkResult {
        success: true,
        message: "Bulk delete completed".to_string(),
        affected_categories: req.category_ids.len(),
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// B118 — NEW-DIRECTORY PROVISIONING: DEFAULTS OVER BLANKS
//
// David (2026-10-02): "To be as little to do manually as I grow." A directory must
// render as a complete, presentable site the moment it exists — nothing a directory
// needs may be blank on creation. Most of that is already inherited for a city in a
// network (branding, categories, plans, legal pages, nav and sitemap are platform /
// network-wide and shared). The pieces that are genuinely PER-DIRECTORY and were
// previously left empty on create are:
//   * the standard ad zones  (every directory is sold with the same six slots)
//   * the system email templates (only a STANDALONE directory needs its own copies —
//     a city inherits its network's by design, so seeding it would break inheritance)
// This routine is idempotent: it only fills what is missing and never overwrites an
// admin's edit, so it is safe to re-run on an existing directory (back-fill).
// ─────────────────────────────────────────────────────────────────────────────

/// The six standard ad-zone slots, byte-identical to the admin Ad Zones panel
/// (frontend/admin-panel.html `AZ_SLOTS`) so the panel and a freshly provisioned
/// directory can never disagree. (key, label, width, height, price_monthly)
pub const STANDARD_AD_ZONES: &[(&str, &str, i32, i32, f64)] = &[
    ("sidebar_top", "Sidebar Top (300x250)", 300, 250, 75.0),
    ("sidebar_bottom", "Sidebar Bottom (300x600)", 300, 600, 50.0),
    ("header_banner", "Header Banner (728x90)", 728, 90, 150.0),
    (
        "between_listings",
        "Between Listings (468x60)",
        468,
        60,
        40.0,
    ),
    ("footer_banner", "Footer Banner (728x90)", 728, 90, 60.0),
    (
        "mobile_interstitial",
        "Mobile Interstitial (320x100)",
        320,
        100,
        100.0,
    ),
];

/// System email events every standalone directory gets a starter template for.
/// (event_key, display name, subject, body_html, body_text)
const SYSTEM_EMAIL_TEMPLATES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "password_reset",
        "Password reset",
        "Reset your {{site_name}} password",
        "<p>Hi {{name}},</p><p>We received a request to reset your {{site_name}} password. Use the link below to choose a new one:</p><p><a href=\"{{reset_url}}\">Reset my password</a></p><p>If you did not ask for this, you can ignore this email — your password stays unchanged.</p><p>— {{site_name}}</p>",
        "Hi {{name}},\n\nWe received a request to reset your {{site_name}} password.\nChoose a new one here: {{reset_url}}\n\nIf you did not ask for this, ignore this email — your password stays unchanged.\n\n— {{site_name}}",
    ),
    (
        "signup_confirmation",
        "Signup confirmation",
        "Welcome to {{site_name}}",
        "<p>Hi {{name}},</p><p>Thanks for joining {{site_name}}. Your account is ready.</p><p><a href=\"{{login_url}}\">Sign in</a> to start exploring local businesses.</p><p>— {{site_name}}</p>",
        "Hi {{name}},\n\nThanks for joining {{site_name}}. Your account is ready.\nSign in here: {{login_url}}\n\n— {{site_name}}",
    ),
    (
        "claim_verification",
        "Business claim verification",
        "Your {{site_name}} verification code",
        "<p>Hi {{name}},</p><p>Your verification code to claim <strong>{{business_name}}</strong> is:</p><p style=\"font-size:22px;font-weight:700;letter-spacing:2px\">{{code}}</p><p>If you did not request this, ignore this email.</p><p>— {{site_name}}</p>",
        "Hi {{name}},\n\nYour verification code to claim {{business_name}} is: {{code}}\n\nIf you did not request this, ignore this email.\n\n— {{site_name}}",
    ),
    (
        "statement",
        "Monthly statement",
        "Your {{site_name}} statement is ready",
        "<p>Hi {{name}},</p><p>Your {{period}} statement is ready. Summary:</p><ul><li>Points issued: {{points_issued}}</li><li>Amount due: {{amount_due}}</li></ul><p><a href=\"{{statement_url}}\">View your full statement</a></p><p>— {{site_name}}</p>",
        "Hi {{name}},\n\nYour {{period}} statement is ready.\nPoints issued: {{points_issued}}\nAmount due: {{amount_due}}\n\nView it here: {{statement_url}}\n\n— {{site_name}}",
    ),
    (
        "notification",
        "General notification",
        "{{title}}",
        "<p>Hi {{name}},</p><p>{{message}}</p><p>— {{site_name}}</p>",
        "Hi {{name}},\n\n{{message}}\n\n— {{site_name}}",
    ),
];

/// Seed the six standard ad zones for a directory. Idempotent per (directory, zone_key):
/// an existing zone — including one an admin has repriced or sold — is never touched.
pub async fn seed_standard_ad_zones(
    db: &sqlx::PgPool,
    directory_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let mut created = 0u64;
    for (key, label, width, height, price) in STANDARD_AD_ZONES {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM ad_zones WHERE directory_id = $1 AND zone_key = $2",
        )
        .bind(directory_id)
        .bind(key)
        .fetch_one(db)
        .await?;
        if exists > 0 {
            continue;
        }
        sqlx::query(
            "INSERT INTO ad_zones (name, zone_key, width, height, price_monthly, directory_id, status) \
             VALUES ($1, $2, $3, $4, $5::numeric, $6, 'available')",
        )
        .bind(label)
        .bind(key)
        .bind(width)
        .bind(height)
        .bind(price)
        .bind(directory_id)
        .execute(db)
        .await?;
        created += 1;
    }
    Ok(created)
}

/// Seed the starter system email templates for a STANDALONE directory (network_id NULL).
/// A city inside a network deliberately inherits its network's templates, so it is skipped
/// here. Idempotent per (directory_id, event_key): an edited template is never overwritten.
pub async fn seed_system_email_templates(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    directory_name: &str,
) -> Result<u64, sqlx::Error> {
    let mut created = 0u64;
    for (event_key, name, subject, body_html, body_text) in SYSTEM_EMAIL_TEMPLATES {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM email_templates WHERE directory_id = $1 AND event_key = $2",
        )
        .bind(directory_id)
        .bind(event_key)
        .fetch_one(db)
        .await?;
        if exists > 0 {
            continue;
        }
        sqlx::query(
            "INSERT INTO email_templates \
             (name, subject, body, body_text, variables, category, directory_id, event_key, is_active) \
             VALUES ($1, $2, $3, $4, $5, 'system', $6, $7, true)",
        )
        .bind(name)
        .bind(subject)
        .bind(body_html)
        .bind(body_text)
        .bind(vec!["site_name", "name"])
        .bind(directory_id)
        .bind(event_key)
        .execute(db)
        .await?;
        created += 1;
    }
    // The directory's display name is the mail "site_name" fallback; keep it discoverable
    // without inventing a second source of truth.
    let _ = directory_name;
    Ok(created)
}

/// Outcome of a provisioning pass, so a caller (create or the back-fill endpoint) can
/// report exactly what was created.
#[derive(Debug, serde::Serialize)]
pub struct ProvisionReport {
    pub ad_zones_created: u64,
    pub email_templates_created: u64,
}

/// Idempotently fill every PER-DIRECTORY default a new directory needs. Never fails the
/// caller on a seeding error — a directory that exists but is missing a default is still
/// usable, and the failure is logged (fail gracefully, never panic and never 500 a create).
pub async fn provision_directory_defaults(
    db: &sqlx::PgPool,
    directory_id: Uuid,
) -> ProvisionReport {
    let ad_zones_created = match seed_standard_ad_zones(db, directory_id).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[directory] ad-zone provisioning failed for {directory_id}: {e}");
            0
        }
    };

    // A standalone directory (not attached to a network) owns its own mail templates;
    // a network city inherits its network's.
    let network_id: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .flatten();

    let email_templates_created = if network_id.is_none() {
        let name: String = sqlx::query_scalar("SELECT name FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_optional(db)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        match seed_system_email_templates(db, directory_id, &name).await {
            Ok(n) => n,
            Err(e) => {
                eprintln!("[directory] email-template provisioning failed for {directory_id}: {e}");
                0
            }
        }
    } else {
        0
    };

    ProvisionReport {
        ad_zones_created,
        email_templates_created,
    }
}

/// POST /api/v1/directories/:slug/provision-defaults — back-fill the per-directory
/// defaults on an EXISTING directory (the same routine create_directory runs), so an
/// existing city can be brought up to the standard without SQL. Idempotent and safe to
/// press twice. Platform operator or the directory's own tenant only.
pub async fn provision_defaults(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let claims = tenant_scope::claims_from_headers(&headers, &s.config.jwt_secret)?;
    tenant_scope::assert_directory_admin_by_slug(&s.db, &claims, &slug).await?;

    let directory = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let report = provision_directory_defaults(&s.db, directory.id).await;

    Ok(Json(json!({
        "status": "ok",
        "directory": directory.slug,
        "ad_zones_created": report.ad_zones_created,
        "email_templates_created": report.email_templates_created,
    })))
}
