//! Pricing engine handlers: service prices, bundles, grandfathered pricing.
//! BL29: Admin-configurable pricing for ZaarHub.

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::auth::middleware::is_admin;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// ── Models ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ServicePrice {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
    pub service_key: String,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub price_one_time: Option<rust_decimal::Decimal>,
    pub currency: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct PriceBundle {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub is_active: bool,
    pub sort_order: i32,
    pub is_featured: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct BundleService {
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub service_key: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct GrandfatheredPricing {
    pub id: Uuid,
    pub business_id: Uuid,
    pub service_key: String,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub price_one_time: Option<rust_decimal::Decimal>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

// ── Request / Response DTOs ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListServicesQuery {
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServicePriceRequest {
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub price_one_time: Option<rust_decimal::Decimal>,
    pub is_active: Option<bool>,
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBundleRequest {
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub is_active: Option<bool>,
    pub sort_order: Option<i32>,
    pub is_featured: Option<bool>,
    pub services: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBundleRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub is_active: Option<bool>,
    pub sort_order: Option<i32>,
    pub is_featured: Option<bool>,
    pub services: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ListBundlesQuery {
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SetGrandfatheredRequest {
    pub business_id: Uuid,
    pub service_key: String,
    pub price_monthly: Option<rust_decimal::Decimal>,
    pub price_yearly: Option<rust_decimal::Decimal>,
    pub price_one_time: Option<rust_decimal::Decimal>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct PublicPricingQuery {
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/v1/pricing/services
/// Returns service prices, optionally scoped to directory or network.
pub async fn list_services(
    State(s): State<AppState>,
    Query(q): Query<ListServicesQuery>,
) -> ApiResult<impl IntoResponse> {
    let services = if let Some(dir_id) = q.directory_id {
        sqlx::query_as::<_, ServicePrice>(
            r#"SELECT * FROM service_prices
               WHERE directory_id = $1 OR (directory_id IS NULL AND network_id IS NULL)
               ORDER BY service_key"#
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?
    } else if let Some(net_id) = q.network_id {
        sqlx::query_as::<_, ServicePrice>(
            r#"SELECT * FROM service_prices
               WHERE network_id = $1 OR (network_id IS NULL AND directory_id IS NULL)
               ORDER BY service_key"#
        )
        .bind(net_id)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, ServicePrice>(
            "SELECT * FROM service_prices WHERE directory_id IS NULL AND network_id IS NULL ORDER BY service_key"
        )
        .fetch_all(&s.db)
        .await?
    };

    Ok(Json(json!({ "services": services })))
}

/// PUT /api/v1/pricing/services/:service_key
/// Admin-only — update price for a service.
pub async fn update_service_price(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(service_key): Path<String>,
    Json(req): Json<UpdateServicePriceRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Determine scope: default to global if neither provided
    let dir_id: Option<Uuid> = req.directory_id;
    let net_id: Option<Uuid> = req.network_id;

    // Upsert: try insert, on conflict update
    let result = sqlx::query_as::<_, ServicePrice>(
        r#"INSERT INTO service_prices (directory_id, network_id, service_key, price_monthly, price_yearly, price_one_time, is_active)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (directory_id, network_id, service_key)
           DO UPDATE SET
               price_monthly = COALESCE($4, service_prices.price_monthly),
               price_yearly = COALESCE($5, service_prices.price_yearly),
               price_one_time = COALESCE($6, service_prices.price_one_time),
               is_active = COALESCE($7, service_prices.is_active),
               updated_at = NOW()
           RETURNING *"#
    )
    .bind(dir_id)
    .bind(net_id)
    .bind(&service_key)
    .bind(req.price_monthly)
    .bind(req.price_yearly)
    .bind(req.price_one_time)
    .bind(req.is_active)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({ "service": result })))
}

/// POST /api/v1/pricing/bundles
/// Admin-only — create a new bundle.
pub async fn create_bundle(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateBundleRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Check slug uniqueness
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM price_bundles WHERE slug = $1"
    )
    .bind(&req.slug)
    .fetch_one(&s.db)
    .await?;

    if existing > 0 {
        return Err(AppError::Duplicate(format!("Bundle slug '{}' already exists", req.slug)));
    }

    // Insert the bundle
    let bundle = sqlx::query_as::<_, PriceBundle>(
        r#"INSERT INTO price_bundles (directory_id, network_id, name, slug, description, price_monthly, price_yearly, is_active, sort_order, is_featured)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING *"#
    )
    .bind(req.directory_id)
    .bind(req.network_id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.price_monthly)
    .bind(req.price_yearly)
    .bind(req.is_active.unwrap_or(true))
    .bind(req.sort_order.unwrap_or(0))
    .bind(req.is_featured.unwrap_or(false))
    .fetch_one(&s.db)
    .await?;

    // Insert bundle services
    for svc_key in &req.services {
        sqlx::query(
            "INSERT INTO bundle_services (bundle_id, service_key) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        )
        .bind(bundle.id)
        .bind(svc_key)
        .execute(&s.db)
        .await?;
    }

    // Fetch services for the response
    let services = sqlx::query_as::<_, BundleService>(
        "SELECT * FROM bundle_services WHERE bundle_id = $1"
    )
    .bind(bundle.id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "bundle": bundle,
        "services": services.iter().map(|bs| &bs.service_key).collect::<Vec<_>>()
    })))
}

/// GET /api/v1/pricing/bundles
/// Returns bundles, optionally filtered by directory or network.
pub async fn list_bundles(
    State(s): State<AppState>,
    Query(q): Query<ListBundlesQuery>,
) -> ApiResult<impl IntoResponse> {
    let bundles = if let Some(dir_id) = q.directory_id {
        sqlx::query_as::<_, PriceBundle>(
            r#"SELECT * FROM price_bundles
               WHERE directory_id = $1 OR (directory_id IS NULL AND network_id IS NULL)
               ORDER BY sort_order, name"#
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?
    } else if let Some(net_id) = q.network_id {
        sqlx::query_as::<_, PriceBundle>(
            r#"SELECT * FROM price_bundles
               WHERE network_id = $1 OR (network_id IS NULL AND directory_id IS NULL)
               ORDER BY sort_order, name"#
        )
        .bind(net_id)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, PriceBundle>(
            "SELECT * FROM price_bundles ORDER BY sort_order, name"
        )
        .fetch_all(&s.db)
        .await?
    };

    // Enrich each bundle with its services
    let mut result = Vec::new();
    for bundle in &bundles {
        let services: Vec<BundleService> = sqlx::query_as(
            "SELECT * FROM bundle_services WHERE bundle_id = $1"
        )
        .bind(bundle.id)
        .fetch_all(&s.db)
        .await?;

        result.push(json!({
            "bundle": bundle,
            "services": services.iter().map(|bs| &bs.service_key).collect::<Vec<_>>()
        }));
    }

    Ok(Json(json!({ "bundles": result })))
}

/// GET /api/v1/pricing/bundles/:id
/// Returns a single bundle with its services.
pub async fn get_bundle(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let bundle = sqlx::query_as::<_, PriceBundle>(
        "SELECT * FROM price_bundles WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Bundle not found".to_string()))?;

    let services: Vec<BundleService> = sqlx::query_as(
        "SELECT * FROM bundle_services WHERE bundle_id = $1"
    )
    .bind(bundle.id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "bundle": bundle,
        "services": services.iter().map(|bs| &bs.service_key).collect::<Vec<_>>()
    })))
}

/// PUT /api/v1/pricing/bundles/:id
/// Admin-only — update a bundle.
pub async fn update_bundle(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateBundleRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Check bundle exists
    let existing = sqlx::query_as::<_, PriceBundle>(
        "SELECT * FROM price_bundles WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Bundle not found".to_string()))?;

    // If slug changed, check uniqueness
    if let Some(ref new_slug) = req.slug {
        if new_slug != &existing.slug {
            let slug_count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM price_bundles WHERE slug = $1 AND id != $2"
            )
            .bind(new_slug)
            .bind(id)
            .fetch_one(&s.db)
            .await?;

            if slug_count > 0 {
                return Err(AppError::Duplicate(format!("Bundle slug '{}' already exists", new_slug)));
            }
        }
    }

    // Update the bundle
    let bundle = sqlx::query_as::<_, PriceBundle>(
        r#"UPDATE price_bundles SET
               name = COALESCE($2, name),
               slug = COALESCE($3, slug),
               description = COALESCE($4, description),
               price_monthly = COALESCE($5, price_monthly),
               price_yearly = COALESCE($6, price_yearly),
               is_active = COALESCE($7, is_active),
               sort_order = COALESCE($8, sort_order),
               is_featured = COALESCE($9, is_featured),
               updated_at = NOW()
           WHERE id = $1
           RETURNING *"#
    )
    .bind(id)
    .bind(&req.name)
    .bind(&req.slug)
    .bind(&req.description)
    .bind(req.price_monthly)
    .bind(req.price_yearly)
    .bind(req.is_active)
    .bind(req.sort_order)
    .bind(req.is_featured)
    .fetch_one(&s.db)
    .await?;

    // Update bundle services if provided
    if let Some(ref services) = req.services {
        // Clear existing, re-insert
        sqlx::query("DELETE FROM bundle_services WHERE bundle_id = $1")
            .bind(id)
            .execute(&s.db)
            .await?;

        for svc_key in services {
            sqlx::query(
                "INSERT INTO bundle_services (bundle_id, service_key) VALUES ($1, $2) ON CONFLICT DO NOTHING"
            )
            .bind(id)
            .bind(svc_key)
            .execute(&s.db)
            .await?;
        }
    }

    // Fetch current services
    let services: Vec<BundleService> = sqlx::query_as(
        "SELECT * FROM bundle_services WHERE bundle_id = $1"
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "bundle": bundle,
        "services": services.iter().map(|bs| &bs.service_key).collect::<Vec<_>>()
    })))
}

/// DELETE /api/v1/pricing/bundles/:id
/// Admin-only — delete a bundle (cascade deletes bundle_services).
pub async fn delete_bundle(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let deleted = sqlx::query("DELETE FROM price_bundles WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?
        .rows_affected();

    if deleted == 0 {
        return Err(AppError::NotFound("Bundle not found".to_string()));
    }

    Ok(Json(json!({ "deleted": true })))
}

/// POST /api/v1/pricing/grandfather
/// Admin-only — set grandfathered pricing for a business + service.
pub async fn set_grandfathered(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<SetGrandfatheredRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let result = sqlx::query_as::<_, GrandfatheredPricing>(
        r#"INSERT INTO grandfathered_pricing (business_id, service_key, price_monthly, price_yearly, price_one_time, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (business_id, service_key)
           DO UPDATE SET
               price_monthly = COALESCE($3, grandfathered_pricing.price_monthly),
               price_yearly = COALESCE($4, grandfathered_pricing.price_yearly),
               price_one_time = COALESCE($5, grandfathered_pricing.price_one_time),
               expires_at = COALESCE($6, grandfathered_pricing.expires_at)
           RETURNING *"#
    )
    .bind(req.business_id)
    .bind(&req.service_key)
    .bind(req.price_monthly)
    .bind(req.price_yearly)
    .bind(req.price_one_time)
    .bind(req.expires_at)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({ "grandfathered": result })))
}

/// GET /api/v1/pricing/grandfather/:business_id
/// Admin-only — get grandfathered pricing for a business.
pub async fn get_grandfathered(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    let rows = sqlx::query_as::<_, GrandfatheredPricing>(
        "SELECT * FROM grandfathered_pricing WHERE business_id = $1 ORDER BY service_key"
    )
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({ "grandfathered": rows })))
}

/// GET /api/v1/pricing/public
/// Public endpoint — returns pricing for a directory/network (for frontend display).
pub async fn public_pricing(
    State(s): State<AppState>,
    Query(q): Query<PublicPricingQuery>,
) -> ApiResult<impl IntoResponse> {
    // Resolve scope: prefer directory_id, then network_id, then global
    let services = if let Some(dir_id) = q.directory_id {
        // Get directory-specific prices, falling back to global defaults
        // We do this in Rust rather than a complex UNION query for clarity
        let dir_prices = sqlx::query_as::<_, ServicePrice>(
            "SELECT * FROM service_prices WHERE directory_id = $1 AND is_active = true"
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?;

        let global_prices = sqlx::query_as::<_, ServicePrice>(
            "SELECT * FROM service_prices WHERE directory_id IS NULL AND network_id IS NULL AND is_active = true"
        )
        .fetch_all(&s.db)
        .await?;

        // Merge: directory-specific overrides global
        merge_prices(dir_prices, global_prices)
    } else if let Some(net_id) = q.network_id {
        let net_prices = sqlx::query_as::<_, ServicePrice>(
            "SELECT * FROM service_prices WHERE network_id = $1 AND is_active = true"
        )
        .bind(net_id)
        .fetch_all(&s.db)
        .await?;

        let global_prices = sqlx::query_as::<_, ServicePrice>(
            "SELECT * FROM service_prices WHERE directory_id IS NULL AND network_id IS NULL AND is_active = true"
        )
        .fetch_all(&s.db)
        .await?;

        merge_prices(net_prices, global_prices)
    } else {
        sqlx::query_as::<_, ServicePrice>(
            "SELECT * FROM service_prices WHERE directory_id IS NULL AND network_id IS NULL AND is_active = true ORDER BY service_key"
        )
        .fetch_all(&s.db)
        .await?
    };

    // Get bundles
    let bundles = if let Some(dir_id) = q.directory_id {
        sqlx::query_as::<_, PriceBundle>(
            "SELECT * FROM price_bundles WHERE (directory_id = $1 OR directory_id IS NULL) AND is_active = true ORDER BY sort_order"
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?
    } else if let Some(net_id) = q.network_id {
        sqlx::query_as::<_, PriceBundle>(
            "SELECT * FROM price_bundles WHERE (network_id = $1 OR network_id IS NULL) AND is_active = true ORDER BY sort_order"
        )
        .bind(net_id)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, PriceBundle>(
            "SELECT * FROM price_bundles WHERE (directory_id IS NULL AND network_id IS NULL) AND is_active = true ORDER BY sort_order"
        )
        .fetch_all(&s.db)
        .await?
    };

    // Enrich bundles with services
    let mut enriched_bundles = Vec::new();
    for bundle in &bundles {
        let bs: Vec<BundleService> = sqlx::query_as(
            "SELECT * FROM bundle_services WHERE bundle_id = $1"
        )
        .bind(bundle.id)
        .fetch_all(&s.db)
        .await?;

        enriched_bundles.push(json!({
            "id": bundle.id,
            "name": bundle.name,
            "slug": bundle.slug,
            "description": bundle.description,
            "price_monthly": bundle.price_monthly,
            "price_yearly": bundle.price_yearly,
            "sort_order": bundle.sort_order,
            "is_featured": bundle.is_featured,
            "services": bs.iter().map(|s| &s.service_key).collect::<Vec<_>>()
        }));
    }

    Ok(Json(json!({
        "services": services,
        "bundles": enriched_bundles
    })))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Merge scoped prices over global defaults. Scoped entries override matching global ones.
fn merge_prices(
    scoped: Vec<ServicePrice>,
    global: Vec<ServicePrice>,
) -> Vec<ServicePrice> {
    let mut map: std::collections::HashMap<String, ServicePrice> = std::collections::HashMap::new();

    for p in global {
        map.insert(p.service_key.clone(), p);
    }
    for p in scoped {
        map.insert(p.service_key.clone(), p);
    }

    let mut result: Vec<ServicePrice> = map.into_values().collect();
    result.sort_by(|a, b| a.service_key.cmp(&b.service_key));
    result
}
