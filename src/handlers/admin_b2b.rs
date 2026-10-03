//! Admin B2B / supplier section (kanban B51).
//!
//! FINDING (2026-09-22): suppliers exist and work publicly — 54 `/b2b/*` routes (marketplace,
//! discover, products, orders, messages, notifications, leads, co-op groups) — but the ZaarHub
//! admin panel had no section for any of it. An endpoint-only capability is a defect by David's
//! front-end parity rule: the operator must be able to SEE and MANAGE the second front.
//!
//! This module is the missing operator surface. It is explicitly NOT a sourcing engine and does
//! NOT scrape suppliers (David, 2026-09-22): suppliers are businesses tagged with `business_type`
//! (`farm`/`distributor`/`wholesaler`/`manufacturer`/`association`/`supplier` vs `local`), and they
//! arrive by self-registration. So this card adds only visibility + moderation over what already
//! exists:
//!
//!   * `GET  /admin/b2b/suppliers`               — the participating suppliers with product and
//!                                                 order volume, scope-aware and filterable.
//!   * `GET  /admin/b2b/suppliers/:id`           — one supplier with its products and orders.
//!   * `POST /admin/b2b/suppliers/:id/status`    — suspend / reinstate a supplier.
//!   * `POST /admin/b2b/products/:id/status`     — approve / remove (deactivate) a product.
//!   * `GET  /admin/b2b/leads`                   — the shared-lead queue.
//!
//! Scope is the same as every other operator surface: an optional `network_id` and/or
//! `directory_id` from the context bar; no scope means the whole platform.
//!
//! Every statement is a compile-time literal and every filter is a bind (gate rule 5d).

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{validate_pagination, ApiResult, AppError};
use crate::AppState;

/// Business types that make a business a SUPPLIER (the second front). `local` is a normal
/// directory listing; everything else tags the record as a participant in the B2B marketplace.
/// Kept as a compile-time allowlist so a typo'd query parameter can never widen the population.
const SUPPLIER_TYPES: &[&str] = &[
    "farm",
    "distributor",
    "wholesaler",
    "manufacturer",
    "association",
    "supplier",
];

#[derive(Debug, Deserialize)]
pub struct SuppliersQuery {
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub q: Option<String>,
    /// Business type filter (farm/distributor/wholesaler/manufacturer/association/supplier).
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    /// `all` (default) / `active` / `suspended`.
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// One page of suppliers with product/order aggregates in the same round trip.
/// `$1` network, `$2` directory, `$3` search, `$4` type, `$5` status, `$6` limit, `$7` offset.
const SUPPLIERS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT b.id::text AS id,
           b.name AS name,
           b.slug AS slug,
           COALESCE(b.business_type, 'local') AS type,
           COALESCE(b.city, d.city) AS city,
           COALESCE(b.state, d.state) AS state,
           d.id::text AS directory_id,
           d.name AS directory_name,
           d.slug AS directory_slug,
           COALESCE(b.is_active, true) AS is_active,
           COALESCE(b.rating, 0) AS rating,
           COALESCE(b.review_count, 0) AS review_count,
           COALESCE(p.product_count, 0) AS product_count,
           COALESCE(o.order_count, 0) AS order_count,
           COALESCE(o.order_volume, 0) AS order_volume,
           b.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM businesses b
    LEFT JOIN directories d ON d.id = b.directory_id
    LEFT JOIN (
        SELECT business_id, count(*) AS product_count
        FROM supplier_products WHERE is_active
        GROUP BY business_id
    ) p ON p.business_id = b.id
    LEFT JOIN (
        SELECT supplier_business_id, count(*) AS order_count,
               COALESCE(sum(total_amount), 0) AS order_volume
        FROM b2b_orders
        GROUP BY supplier_business_id
    ) o ON o.supplier_business_id = b.id
    WHERE COALESCE(b.business_type, 'local') <> 'local'
      AND ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%')
      AND ($4::text IS NULL OR COALESCE(b.business_type, 'local') = $4)
      AND ($5::text IS NULL
           OR ($5 = 'active' AND COALESCE(b.is_active, true))
           OR ($5 = 'suspended' AND NOT COALESCE(b.is_active, true)))
    ORDER BY b.name
    LIMIT $6 OFFSET $7
) t
"#;

/// Population counts in the same scope + search + type (status filter NOT applied, so the card
/// can label the tabs regardless of which tab is open).
/// `$1` network, `$2` directory, `$3` search, `$4` type.
const COUNTS_SQL: &str = r#"
SELECT count(*) AS all_rows,
       count(*) FILTER (WHERE COALESCE(b.is_active, true)) AS active,
       count(*) FILTER (WHERE NOT COALESCE(b.is_active, true)) AS suspended
FROM businesses b
LEFT JOIN directories d ON d.id = b.directory_id
WHERE COALESCE(b.business_type, 'local') <> 'local'
  AND ($1::uuid IS NULL OR d.network_id = $1)
  AND ($2::uuid IS NULL OR d.id = $2)
  AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%')
  AND ($4::text IS NULL OR COALESCE(b.business_type, 'local') = $4)
"#;

/// One supplier row, scoped so a stray id from another tenant cannot be read.
/// `$1` id, `$2` network, `$3` directory.
const SUPPLIER_SQL: &str = r#"
SELECT b.id::text AS id, b.name AS name, b.slug AS slug,
       COALESCE(b.business_type, 'local') AS type,
       COALESCE(b.city, d.city) AS city, COALESCE(b.state, d.state) AS state,
       COALESCE(b.description, '') AS description,
       b.address AS address, b.phone AS phone, b.email AS email, b.website AS website,
       d.id::text AS directory_id, d.name AS directory_name, d.slug AS directory_slug,
       COALESCE(b.is_active, true) AS is_active,
       COALESCE(b.verified, false) AS verified,
       COALESCE(b.rating, 0) AS rating,
       COALESCE(b.review_count, 0) AS review_count,
       b.created_at::text AS created_at,
       b.supplier_fields AS supplier_fields
FROM businesses b
LEFT JOIN directories d ON d.id = b.directory_id
WHERE b.id = $1
  AND COALESCE(b.business_type, 'local') <> 'local'
  AND ($2::uuid IS NULL OR d.network_id = $2)
  AND ($3::uuid IS NULL OR d.id = $3)
"#;

/// `$1` supplier id.
const PRODUCTS_SQL: &str = r#"
SELECT id::text AS id, name, description, category, price::text AS price, unit,
       min_order, is_active, created_at::text AS created_at, updated_at::text AS updated_at
FROM supplier_products
WHERE business_id = $1
ORDER BY is_active DESC, name
"#;

/// `$1` supplier id, `$2` limit.
const ORDERS_SQL: &str = r#"
SELECT o.id::text AS id, o.quantity, o.unit_price::text AS unit_price,
       o.total_amount::text AS total_amount, o.status,
       o.created_at::text AS created_at,
       COALESCE(p.name, '') AS product_name,
       COALESCE(bb.name, '') AS buyer_name
FROM b2b_orders o
LEFT JOIN supplier_products p ON p.id = o.product_id
LEFT JOIN businesses bb ON bb.id = o.buyer_business_id
WHERE o.supplier_business_id = $1
ORDER BY o.created_at DESC
LIMIT $2
"#;

/// Moderation: suspend/reinstate a supplier. `$1` id, `$2` is_active, `$3` network,
/// `$4` directory. A supplier may have no directory (B2B registers without a city), so the
/// scope is an EXISTS check rather than an inner join — no directory is in scope for the
/// platform-wide view but excluded once a network/directory scope is chosen.
const SUPPLIER_STATUS_SQL: &str = r#"
UPDATE businesses b
SET is_active = $2, updated_at = NOW()
WHERE b.id = $1
  AND COALESCE(b.business_type, 'local') <> 'local'
  AND ($3::uuid IS NULL OR EXISTS (
        SELECT 1 FROM directories d WHERE d.id = b.directory_id AND d.network_id = $3))
  AND ($4::uuid IS NULL OR b.directory_id = $4)
RETURNING b.id::text AS id, COALESCE(b.is_active, true) AS is_active
"#;

/// Moderation: approve/remove a product. `$1` product id, `$2` is_active, `$3` network,
/// `$4` directory. Scoped through the product's owning supplier directory (EXISTS, so a
/// supplier with no directory is reachable in the platform-wide view).
const PRODUCT_STATUS_SQL: &str = r#"
UPDATE supplier_products sp
SET is_active = $2, updated_at = NOW()
WHERE sp.id = $1
  AND EXISTS (
      SELECT 1 FROM businesses b
      WHERE b.id = sp.business_id
        AND COALESCE(b.business_type, 'local') <> 'local'
        AND ($3::uuid IS NULL OR EXISTS (
              SELECT 1 FROM directories d WHERE d.id = b.directory_id AND d.network_id = $3))
        AND ($4::uuid IS NULL OR b.directory_id = $4)
  )
RETURNING sp.id::text AS id, sp.is_active AS is_active
"#;

/// `$1` network, `$2` directory, `$3` status, `$4` limit, `$5` offset. The lead queue is scoped
/// through the POSTING business's directory (a lead belongs to whoever shared it).
const LEADS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT l.id::text AS id, l.title, l.description, l.category, l.location,
           l.estimated_value, l.status, l.created_at::text AS created_at,
           l.claimed_at::text AS claimed_at, l.expires_at::text AS expires_at,
           COALESCE(pb.name, '') AS poster_name,
           COALESCE(cb.name, '') AS claimed_by_name,
           count(*) OVER() AS total
    FROM shared_leads l
    LEFT JOIN businesses pb ON pb.id = l.poster_business_id
    LEFT JOIN businesses cb ON cb.id = l.claimed_by
    LEFT JOIN directories d ON d.id = pb.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR l.status = $3)
    ORDER BY l.created_at DESC
    LIMIT $4 OFFSET $5
) t
"#;

fn normalize_type(raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(t) => {
            let t = t.to_ascii_lowercase();
            if !SUPPLIER_TYPES.contains(&t.as_str()) {
                return Err(AppError::Validation(format!(
                    "type must be one of: {} (got '{t}')",
                    SUPPLIER_TYPES.join(", ")
                )));
            }
            Ok(Some(t))
        }
    }
}

fn normalize_status(raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw
        .map(str::trim)
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("all"))
    {
        None => Ok(None),
        Some(s) => {
            let s = s.to_ascii_lowercase();
            if s != "active" && s != "suspended" {
                return Err(AppError::Validation(format!(
                    "status must be 'all', 'active' or 'suspended' (got '{s}')"
                )));
            }
            Ok(Some(s))
        }
    }
}

/// GET /admin/b2b/suppliers — every participating supplier with product/order volume.
pub async fn list_suppliers(
    State(s): State<AppState>,
    Query(q): Query<SuppliersQuery>,
) -> ApiResult<impl IntoResponse> {
    let search = q.q.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let type_filter = normalize_type(q.type_filter.as_deref())?;
    let status = normalize_status(q.status.as_deref())?;
    let (page, per_page) = validate_pagination(q.page, q.per_page);
    let offset = (page - 1) * per_page;

    let row = sqlx::query(SUPPLIERS_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .bind(search)
        .bind(type_filter.as_deref())
        .bind(status.as_deref())
        .bind(per_page)
        .bind(offset)
        .fetch_one(&s.db)
        .await?;
    let rows: serde_json::Value = row.try_get("rows")?;
    let total: i64 = row.try_get("total")?;

    let counts = sqlx::query(COUNTS_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .bind(search)
        .bind(type_filter.as_deref())
        .fetch_one(&s.db)
        .await?;

    Ok(Json(json!({
        "network_id": q.network_id,
        "directory_id": q.directory_id,
        "q": search,
        "type": type_filter.clone().unwrap_or_else(|| "all".to_string()),
        "status": status.clone().unwrap_or_else(|| "all".to_string()),
        "page": page,
        "per_page": per_page,
        "total": total,
        "counts": {
            "all": counts.try_get::<i64, _>("all_rows").unwrap_or(0),
            "active": counts.try_get::<i64, _>("active").unwrap_or(0),
            "suspended": counts.try_get::<i64, _>("suspended").unwrap_or(0),
        },
        "rows": rows,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ScopeQuery {
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
}

/// GET /admin/b2b/suppliers/:id — one supplier with its products and recent orders.
pub async fn get_supplier(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Query(scope): Query<ScopeQuery>,
) -> ApiResult<impl IntoResponse> {
    let supplier = sqlx::query(SUPPLIER_SQL)
        .bind(id)
        .bind(scope.network_id)
        .bind(scope.directory_id)
        .fetch_optional(&s.db)
        .await?;
    let Some(supplier) = supplier else {
        return Err(AppError::NotFound("supplier not found in scope".into()));
    };

    let products = sqlx::query(PRODUCTS_SQL).bind(id).fetch_all(&s.db).await?;
    let product_rows: Vec<serde_json::Value> = products
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<String, _>("id").unwrap_or_default(),
                "name": r.try_get::<String, _>("name").unwrap_or_default(),
                "description": r.try_get::<Option<String>, _>("description").unwrap_or(None),
                "category": r.try_get::<Option<String>, _>("category").unwrap_or(None),
                "price": r.try_get::<Option<String>, _>("price").unwrap_or(None),
                "unit": r.try_get::<Option<String>, _>("unit").unwrap_or(None),
                "min_order": r.try_get::<Option<i32>, _>("min_order").unwrap_or(None),
                "is_active": r.try_get::<Option<bool>, _>("is_active").unwrap_or(Some(true)),
                "created_at": r.try_get::<Option<String>, _>("created_at").unwrap_or(None),
            })
        })
        .collect();

    let orders = sqlx::query(ORDERS_SQL)
        .bind(id)
        .bind(50_i64)
        .fetch_all(&s.db)
        .await?;
    let order_rows: Vec<serde_json::Value> = orders
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<String, _>("id").unwrap_or_default(),
                "quantity": r.try_get::<Option<i32>, _>("quantity").unwrap_or(None),
                "unit_price": r.try_get::<Option<String>, _>("unit_price").unwrap_or(None),
                "total_amount": r.try_get::<Option<String>, _>("total_amount").unwrap_or(None),
                "status": r.try_get::<String, _>("status").unwrap_or_default(),
                "created_at": r.try_get::<Option<String>, _>("created_at").unwrap_or(None),
                "product_name": r.try_get::<Option<String>, _>("product_name").unwrap_or(None),
                "buyer_name": r.try_get::<Option<String>, _>("buyer_name").unwrap_or(None),
            })
        })
        .collect();

    // Read the header row field by field (a generic Value decode is only valid for json/jsonb
    // columns, so the mixed row is mapped explicitly to keep every column's real type).
    let supplier_json = json!({
        "id": supplier.try_get::<String, _>("id").unwrap_or_default(),
        "name": supplier.try_get::<String, _>("name").unwrap_or_default(),
        "slug": supplier.try_get::<String, _>("slug").unwrap_or_default(),
        "type": supplier.try_get::<String, _>("type").unwrap_or_default(),
        "city": supplier.try_get::<Option<String>, _>("city").unwrap_or(None),
        "state": supplier.try_get::<Option<String>, _>("state").unwrap_or(None),
        "description": supplier.try_get::<String, _>("description").unwrap_or_default(),
        "address": supplier.try_get::<Option<String>, _>("address").unwrap_or(None),
        "phone": supplier.try_get::<Option<String>, _>("phone").unwrap_or(None),
        "email": supplier.try_get::<Option<String>, _>("email").unwrap_or(None),
        "website": supplier.try_get::<Option<String>, _>("website").unwrap_or(None),
        "directory_id": supplier.try_get::<String, _>("directory_id").unwrap_or_default(),
        "directory_name": supplier.try_get::<String, _>("directory_name").unwrap_or_default(),
        "directory_slug": supplier.try_get::<String, _>("directory_slug").unwrap_or_default(),
        "is_active": supplier.try_get::<bool, _>("is_active").unwrap_or(true),
        "verified": supplier.try_get::<bool, _>("verified").unwrap_or(false),
        "rating": supplier.try_get::<Option<f64>, _>("rating").unwrap_or(None),
        "review_count": supplier.try_get::<Option<i32>, _>("review_count").unwrap_or(None),
        "created_at": supplier.try_get::<Option<String>, _>("created_at").unwrap_or(None),
        "supplier_fields": supplier
            .try_get::<serde_json::Value, _>("supplier_fields")
            .unwrap_or(serde_json::Value::Null),
    });

    Ok(Json(json!({
        "supplier": supplier_json,
        "products": product_rows,
        "orders": order_rows,
        "product_count": product_rows.len(),
        "order_count": order_rows.len(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct StatusBody {
    pub is_active: bool,
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
}

/// POST /admin/b2b/suppliers/:id/status — suspend (is_active=false) or reinstate a supplier.
pub async fn set_supplier_status(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<StatusBody>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query(SUPPLIER_STATUS_SQL)
        .bind(id)
        .bind(body.is_active)
        .bind(body.network_id)
        .bind(body.directory_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("supplier not found in scope".into()))?;

    Ok(Json(json!({
        "id": row.try_get::<String, _>("id").unwrap_or_default(),
        "is_active": row.try_get::<bool, _>("is_active").unwrap_or(body.is_active),
        "action": if body.is_active { "reinstated" } else { "suspended" },
    })))
}

/// POST /admin/b2b/products/:id/status — approve (is_active=true) or remove a product.
pub async fn set_product_status(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<StatusBody>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query(PRODUCT_STATUS_SQL)
        .bind(id)
        .bind(body.is_active)
        .bind(body.network_id)
        .bind(body.directory_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("product not found in scope".into()))?;

    Ok(Json(json!({
        "id": row.try_get::<String, _>("id").unwrap_or_default(),
        "is_active": row.try_get::<bool, _>("is_active").unwrap_or(body.is_active),
        "action": if body.is_active { "approved" } else { "removed" },
    })))
}

#[derive(Debug, Deserialize)]
pub struct LeadsQuery {
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    /// `available` / `claimed` / `expired` / `all` (default).
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// GET /admin/b2b/leads — the shared-lead queue (visibility only; claiming stays in the portal).
pub async fn list_leads(
    State(s): State<AppState>,
    Query(q): Query<LeadsQuery>,
) -> ApiResult<impl IntoResponse> {
    let status = match q
        .status
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("all"))
    {
        None => None,
        Some(s) => {
            let s = s.to_ascii_lowercase();
            if !matches!(s.as_str(), "available" | "claimed" | "expired") {
                return Err(AppError::Validation(format!(
                    "status must be 'all', 'available', 'claimed' or 'expired' (got '{s}')"
                )));
            }
            Some(s)
        }
    };
    let (page, per_page) = validate_pagination(q.page, q.per_page);
    let offset = (page - 1) * per_page;

    let row = sqlx::query(LEADS_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .bind(status.as_deref())
        .bind(per_page)
        .bind(offset)
        .fetch_one(&s.db)
        .await?;
    let rows: serde_json::Value = row.try_get("rows")?;
    let total: i64 = row.try_get("total")?;

    Ok(Json(json!({
        "network_id": q.network_id,
        "directory_id": q.directory_id,
        "status": status.clone().unwrap_or_else(|| "all".to_string()),
        "page": page,
        "per_page": per_page,
        "total": total,
        "rows": rows,
    })))
}
