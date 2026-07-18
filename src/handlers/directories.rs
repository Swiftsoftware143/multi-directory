//! Directory and category CRUD handlers.
//!
//! Updated with template engine support:
//! - Create/Update accept template + color_scheme
//! - GET /api/v1/directories/:slug/render returns HTML rendered with template
//! - GET /api/v1/directories/:slug/preview returns template preview

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult, validate_pagination};
use crate::models::*;
use crate::template_engine;
use crate::tracking_script;

/// GET /api/v1/directories
pub async fn list_directories(
    State(s): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
    let per_page = params.get("per_page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories")
        .fetch_one(&s.db)
        .await?;

    let directories = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 "
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

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
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    Ok(Json(json!(directory)))
}

/// POST /api/v1/directories
pub async fn create_directory(
    State(s): State<AppState>,
    Json(req): Json<CreateDirectoryRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.is_empty() {
        return Err(AppError::Validation("Directory name is required".to_string()));
    }

    // Auto-generate slug if not provided
    let slug = match &req.slug {
        Some(s) if !s.is_empty() => s.clone(),
        _ => req.name.to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
            .replace(' ', "-")
            .chars().take(80).collect::<String>(),
    };

    if slug.is_empty() {
        return Err(AppError::Validation("Slug is required (auto-generation failed)".to_string()));
    }

    // Check slug uniqueness
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!("Directory slug '{}' already exists", slug)));
    }

    let template = req.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let template = if template_engine::is_valid_template(template) { template } else { template_engine::TEMPLATE_LOCAL_BUSINESS };

    // Determine network mode
    let network_mode = req.network_mode.as_deref().unwrap_or("standalone");
    let (network_id, url_type, url_value, custom_domain) = resolve_network_config(&s, &req, &slug, network_mode).await?;

    let template_config = req.template_config.clone().unwrap_or_default();

    // Color scheme: use provided, inherit from network, or default
    let color_scheme = if let Some(cs) = req.color_scheme.clone() {
        cs
    } else if network_mode == "connect" {
        if let Some(nid) = network_id {
            let nb = sqlx::query_as::<_, crate::models::NetworkBranding>(
                "SELECT * FROM network_branding WHERE network_id = $1"
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
        r#"INSERT INTO directories (name, slug, description, status, template, color_scheme, network_id, url_type, url_value, custom_domain, city, template_config, head_injection, body_injection, footer_injection, email_signature_html, email_signature_text)
           VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10, $11, $12::jsonb, $13, $14, $15, $16, $17)
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
    .fetch_one(&s.db)
    .await?;

    // If network_mode="new_network", create the network and link it
    if network_mode == "new_network" {
        let network_slug = format!("network-{}", &slug);
        let network = sqlx::query_as::<_, crate::models::Network>(
            r#"INSERT INTO networks (name, slug, description, root_domain)
               VALUES ($1, $2, $3, $4)
               RETURNING *"#
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

        // Create default homepage hero section
        sqlx::query(
            r#"INSERT INTO homepage_sections (network_id, section_type, sort_order, title)
               VALUES ($1, 'hero', 0, $2)"#
        )
        .bind(network.id)
        .bind(&req.name)
        .execute(&s.db)
        .await?;

        // Link directory to the new network
        sqlx::query("UPDATE directories SET network_id = $1 WHERE id = $2")
            .bind(network.id)
            .bind(directory.id)
            .execute(&s.db)
            .await?;

        // Re-fetch directory to get updated network_id
        directory = sqlx::query_as::<_, Directory>(
            "SELECT * FROM directories WHERE id = $1"
        )
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
            match crate::coreswift::provision_tenant(&db2, dir_id2, &dir_name2, &dir_slug2, true).await {
                Ok(_) => tracing::info!("[directory] CoreSwift network tenant provisioned for {dir_slug2}"),
                Err(e) => tracing::warn!("[directory] CoreSwift network tenant provisioning failed: {e}"),
            }
            // Then provision directory resources (booking calendar, tags, etc.)
            match crate::coreswift::provision_directory_resources(&db2, dir_id2, &dir_slug2).await {
                Ok(prefix) => tracing::info!("[directory] CoreSwift resources provisioned for {dir_slug2} (prefix={prefix})"),
                Err(e) => tracing::warn!("[directory] CoreSwift resource provisioning failed for {dir_slug2}: {e}"),
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
            match crate::coreswift::provision_tenant(&db, dir_id, &dir_name, &dir_slug, false).await {
                Ok(_) => {
                    tracing::info!("[directory] CoreSwift tenant provisioned for {dir_slug}");
                    match crate::coreswift::provision_directory_resources(&db, dir_id, &dir_slug).await {
                        Ok(prefix) => tracing::info!("[directory] CoreSwift resources provisioned for {dir_slug} (prefix={prefix})"),
                        Err(e) => tracing::warn!("[directory] CoreSwift resource provisioning failed for {dir_slug}: {e}"),
                    }
                },
                Err(e) => tracing::warn!("[directory] CoreSwift provisioning failed for {dir_slug}: {e}"),
            }
        });
    }

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
            let network_id = req.parent_network_id
                .ok_or(AppError::Validation("parent_network_id is required when network_mode='connect'".to_string()))?;

            let network_exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM networks WHERE id = $1"
            )
            .bind(network_id)
            .fetch_one(&s.db)
            .await?;

            if network_exists == 0 {
                return Err(AppError::NotFound(format!("Network '{}' not found", network_id)));
            }

            let url_type = req.url_type.clone().unwrap_or_else(|| "subfolder".to_string());
            let url_value = req.url_value.clone().unwrap_or_else(|| slug.to_string());
            let custom_domain = req.custom_domain.clone();

            Ok((Some(network_id), Some(url_type), Some(url_value), custom_domain))
        }
        "new_network" => {
            let url_type = req.url_type.clone().unwrap_or_else(|| "standalone".to_string());
            let url_value = req.url_value.clone().or_else(|| Some(slug.to_string()));
            let custom_domain = req.custom_domain.clone();
            Ok((None, Some(url_type), url_value, custom_domain))
        }
        _ => {
            // Standalone
            Ok((None, Some("standalone".to_string()), Some(slug.to_string()), req.custom_domain.clone()))
        }
    }
}

/// PUT /api/v1/directories/:slug
pub async fn update_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<UpdateDirectoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let new_name = req.name.unwrap_or(existing.name);
    let new_slug = req.slug.unwrap_or(existing.slug.clone());
    let new_description = req.description.or(existing.description);
    let new_status = req.status.or(existing.status);
    let new_template = req.template.unwrap_or(existing.template.unwrap_or_else(|| template_engine::TEMPLATE_LOCAL_BUSINESS.to_string()));
    let new_color_scheme = req.color_scheme.or(existing.color_scheme);
    let new_network_id = req.network_id.or(existing.network_id);
    let new_url_type = req.url_type.or(existing.url_type);
    let new_url_value = req.url_value.or(existing.url_value);
    let new_custom_domain = req.custom_domain.or(existing.custom_domain);
    let new_city = req.city.or(existing.city);
    let new_head_injection = req.head_injection.clone().or(existing.head_injection.clone());
    let new_body_injection = req.body_injection.clone().or(existing.body_injection.clone());
    let new_footer_injection = req.footer_injection.clone().or(existing.footer_injection.clone());
    let new_template_config = req.template_config.clone().or(existing.template_config);
    let new_email_signature_html = req.email_signature_html.clone().or(existing.email_signature_html);
    let new_email_signature_text = req.email_signature_text.clone().or(existing.email_signature_text);

    if new_slug != slug {
        let slug_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM directories WHERE slug = $1 AND id != $2"
        )
        .bind(&new_slug)
        .bind(existing.id)
        .fetch_one(&s.db)
        .await?;

        if slug_exists > 0 {
            return Err(AppError::Duplicate(format!("Slug '{}' already in use", new_slug)));
        }
    }

    let directory = sqlx::query_as::<_, Directory>(
        "UPDATE directories SET name = $1, slug = $2, description = $3, status = $4, template = $5, color_scheme = $6::jsonb, network_id = $7, url_type = $8, url_value = $9, custom_domain = $10, city = $11, template_config = $12::jsonb, head_injection = $14, body_injection = $15, footer_injection = $16, email_signature_html = $17, email_signature_text = $18, updated_at = NOW() WHERE id = $13 RETURNING *"
    )
    .bind(&new_name)
    .bind(&new_slug)
    .bind(&new_description)
    .bind(&new_status)
    .bind(&new_template)
    .bind(&new_color_scheme.map(|v| v.to_string()).unwrap_or_default())
    .bind(&new_network_id)
    .bind(&new_url_type)
    .bind(&new_url_value)
    .bind(&new_custom_domain)
    .bind(&new_city)
    .bind(&new_template_config.map(|v| v.to_string()).unwrap_or_default())
    .bind(existing.id)
    .bind(&new_head_injection)
    .bind(&new_body_injection)
    .bind(&new_footer_injection)
    .bind(&new_email_signature_html)
    .bind(&new_email_signature_text)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(directory)))
}

/// DELETE /api/v1/directories/:slug
pub async fn delete_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM directories WHERE slug = \x241")
        .bind(&slug)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("Directory '{}' not found", slug)));
    }

    Ok((StatusCode::OK, Json(json!({"message": "Directory deleted successfully"}))))
}

/// GET /api/v1/directories/:slug/render — render directory page with template
pub async fn render_directory(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

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
            "SELECT * FROM business_meta WHERE business_id = \x241 AND template = \x242 "
        )
        .bind(biz.id)
        .bind(directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS))
        .fetch_optional(&s.db)
        .await
        {
            if let Some(m) = meta {
                meta_map.insert(biz.id, m.meta_data);
            }
        }
    }

    let template_id = directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let cats_val = serde_json::to_value(&categories).unwrap_or_default();
    let biz_val = serde_json::to_value(&businesses).unwrap_or_default();
    let ctx = template_engine::build_template_context(
        &dir_val,
        &biz_val,
        &cats_val,
        None,
        None,
    );
    let html = engine.render_directory_page(template_id, &ctx)
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
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

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
        return Err(AppError::Validation("Name and slug are required".to_string()));
    }

    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

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
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let existing = sqlx::query_as::<_, DirectoryCategory>(
        "SELECT * FROM directory_categories WHERE id = \x241 AND directory_id = \x242 "
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
        return Err(AppError::Validation("A category cannot be its own parent".to_string()));
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
    let _dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    // Check for existing businesses
    let business_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses WHERE category_id = \x241"
    )
    .bind(category_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Check for subcategories
    let subcategory_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM directory_categories WHERE parent_id = \x241"
    )
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
        if let Some(reassign_to) = params.get("reassign_to").and_then(|v| Uuid::parse_str(v).ok()) {
            // Reassign businesses to target category
            sqlx::query(
                "UPDATE businesses SET category_id = \x241 WHERE category_id = \x242"
            )
            .bind(reassign_to)
            .bind(category_id)
            .execute(&s.db)
            .await?;
        } else {
            // Delete all businesses in this category
            sqlx::query(
                "DELETE FROM businesses WHERE category_id = \x241"
            )
            .bind(category_id)
            .execute(&s.db)
            .await?;
        }
    }

    // If force and reassign_to for subcategories, move subcategories up to parent's parent
    if force && subcategory_count > 0 {
        let parent_of_deleted = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT parent_id FROM directory_categories WHERE id = \x241"
        )
        .bind(category_id)
        .fetch_optional(&s.db)
        .await?
        .flatten();

        sqlx::query(
            "UPDATE directory_categories SET parent_id = \x241 WHERE parent_id = \x242"
        )
        .bind(parent_of_deleted)
        .bind(category_id)
        .execute(&s.db)
        .await?;
    }

    // Clear category_id from visitor_events
    sqlx::query(
        "UPDATE visitor_events SET category_id = NULL WHERE category_id = \x241"
    )
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
    let _dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    if req.category_ids.is_empty() {
        return Err(AppError::Validation("No category IDs provided".to_string()));
    }

    let mut affected = 0usize;

    if req.move_businesses {
        let result = sqlx::query(
            "UPDATE businesses SET category_id = \x241 WHERE category_id = ANY(\x242)"
        )
        .bind(req.target_category_id)
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;
        affected += result.rows_affected() as usize;
    }

    if req.move_subcategories {
        let result = sqlx::query(
            "UPDATE directory_categories SET parent_id = \x241 WHERE id = ANY(\x242)"
        )
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
    let _dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    if req.category_ids.is_empty() {
        return Err(AppError::Validation("No category IDs provided".to_string()));
    }

    if let Some(reassign_to) = req.reassign_to {
        // Reassign businesses to target category
        sqlx::query(
            "UPDATE businesses SET category_id = \x241 WHERE category_id = ANY(\x242)"
        )
        .bind(reassign_to)
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;

        // Move subcategories up
        sqlx::query(
            "UPDATE directory_categories SET parent_id = NULL WHERE parent_id = ANY(\x241)"
        )
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;
    } else {
        // Delete all businesses in these categories
        sqlx::query(
            "DELETE FROM businesses WHERE category_id = ANY(\x241)"
        )
        .bind(&req.category_ids)
        .execute(&s.db)
        .await?;
    }

    // Clear category_id from visitor_events
    sqlx::query(
        "UPDATE visitor_events SET category_id = NULL WHERE category_id = ANY(\x241)"
    )
    .bind(&req.category_ids)
    .execute(&s.db)
    .await?;

    // Delete the categories
    sqlx::query(
        "DELETE FROM directory_categories WHERE id = ANY(\x241)"
    )
    .bind(&req.category_ids)
    .execute(&s.db)
    .await?;

    Ok(Json(json!(CategoryBulkResult {
        success: true,
        message: "Bulk delete completed".to_string(),
        affected_categories: req.category_ids.len(),
    })))
}
