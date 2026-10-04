//! Admin "Businesses" tab — directory listings AND claimed businesses in ONE place
//! (kanban B73).
//!
//! David's ask (2026-09-29): "in the admin panel I should have a list of businesses
//! that claim their business and obviously there should be business listings — I
//! should be able to have it in one tab and I should also have it set up so it can
//! go into my CRM once I connect it."
//!
//! One operator-guarded endpoint backs the panel's "Businesses" card:
//!   * `GET /admin/businesses` — every business in scope with its city, whether it is
//!     claimed or merely listed, the linked account/owner, rating/review count and its
//!     CoreSwift sync state, filterable by claim state and searchable by name/owner.
//!
//! Scope is the same as the rest of the operator surface: an optional `network_id`
//! and/or `directory_id` (the panel passes the persistent context-bar scope). No
//! scope means the whole platform, which is what "All networks and directories" means.
//!
//! Every statement is a compile-time literal and every filter is a bind (class-14).

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{validate_pagination, ApiResult, AppError};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct BusinessesQuery {
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub q: Option<String>,
    /// `claimed` / `unclaimed` / `all` (default) — which population to show.
    pub claim: Option<String>,
    /// Lifecycle filter (card B81): `active` (published), `draft`, `prospect`,
    /// `hidden` (draft+prospect) or `all` (default) — which records to show.
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// Rows for one page. `$1` network, `$2` directory, `$3` search, `$4` claim filter
/// (`claimed`/`unclaimed`/NULL = all), `$5` lifecycle filter, `$6` limit, `$7` offset.
///
/// `count(*) OVER()` carries the filtered total in the same round trip.
const ROWS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT b.id::text AS id,
           b.name AS name,
           b.slug AS slug,
           COALESCE(b.city, d.city) AS city,
           COALESCE(b.state, d.state) AS state,
           d.name AS directory_name,
           d.slug AS directory_slug,
           d.id::text AS directory_id,
           COALESCE(b.business_type, 'local') AS type,
           CASE WHEN b.claimed THEN 'claimed'
                WHEN b.verified THEN 'verified'
                ELSE 'listed' END AS status,
           COALESCE(b.status, 'active') AS lifecycle,
           COALESCE(b.is_active, true) AS is_active,
           b.claimed,
           b.verified,
           b.licensed,
           b.insured,
           (c.id IS NOT NULL) AS has_account,
           c.owner_email AS owner_email,
           c.owner_name AS owner_name,
           COALESCE(b.rating, 0) AS rating,
           COALESCE(b.review_count, 0) AS review_count,
           COALESCE(cs.status, 'not_synced') AS sync_status,
           cs.last_pushed_at::text AS sync_last_pushed_at,
           b.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM businesses b
    JOIN directories d ON d.id = b.directory_id
    LEFT JOIN claimed_businesses c ON c.business_id = b.id
    LEFT JOIN coreswift_sync_state cs
           ON cs.entity_kind = 'business' AND cs.entity_id = b.id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%'
                            OR COALESCE(c.owner_email, '') ILIKE '%' || $3 || '%'
                            OR COALESCE(c.owner_name, '') ILIKE '%' || $3 || '%')
      AND ($4::text IS NULL
           OR ($4 = 'claimed' AND b.claimed)
           OR ($4 = 'unclaimed' AND NOT b.claimed))
      AND ($5::text IS NULL
           OR ($5 = 'active' AND COALESCE(b.status, 'active') = 'active')
           OR ($5 = 'draft' AND b.status = 'draft')
           OR ($5 = 'prospect' AND b.status = 'prospect')
           OR ($5 = 'hidden' AND b.status IN ('draft', 'prospect')))
    ORDER BY b.claimed DESC, d.name, b.name
    LIMIT $6 OFFSET $7
) t
"#;

/// Filtered counts for each population, in the same scope (search applied, claim not).
const COUNTS_SQL: &str = r#"
SELECT count(*) FILTER (WHERE b.claimed) AS claimed,
       count(*) FILTER (WHERE NOT b.claimed) AS unclaimed,
       count(*) FILTER (WHERE COALESCE(b.status, 'active') = 'active') AS published,
       count(*) FILTER (WHERE b.status = 'draft') AS drafts,
       count(*) FILTER (WHERE b.status = 'prospect') AS prospects,
       count(*) FILTER (WHERE b.status IN ('draft', 'prospect')) AS hidden,
       count(*) AS all_rows
FROM businesses b
JOIN directories d ON d.id = b.directory_id
LEFT JOIN claimed_businesses c ON c.business_id = b.id
WHERE ($1::uuid IS NULL OR d.network_id = $1)
  AND ($2::uuid IS NULL OR d.id = $2)
  AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%'
                        OR COALESCE(c.owner_email, '') ILIKE '%' || $3 || '%'
                        OR COALESCE(c.owner_name, '') ILIKE '%' || $3 || '%')
"#;

/// True when any directory in scope has a CoreSwift tenant, directly or via its
/// network — the same source the connection endpoints store.
const CONNECTED_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1 FROM directories d
    LEFT JOIN networks n ON n.id = d.network_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND COALESCE(d.coreswift_tenant_id, n.coreswift_tenant_id) IS NOT NULL
) AS connected
"#;

/// GET /admin/businesses — listings + claimed businesses in one list (operator-guarded).
pub async fn list_businesses(
    State(s): State<AppState>,
    Query(q): Query<BusinessesQuery>,
) -> ApiResult<impl IntoResponse> {
    let claim = q
        .claim
        .as_deref()
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| !c.is_empty() && c != "all");
    if let Some(ref c) = claim {
        if c != "claimed" && c != "unclaimed" {
            return Err(AppError::Validation(format!(
                "claim must be 'all', 'claimed' or 'unclaimed' (got '{c}')"
            )));
        }
    }

    let search = q.q.as_deref().map(str::trim).filter(|v| !v.is_empty());

    // Lifecycle filter (card B81): 'all' clears it; anything else must be a known bucket.
    let lifecycle = q
        .status
        .as_deref()
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| !c.is_empty() && c != "all");
    if let Some(ref c) = lifecycle {
        if !matches!(c.as_str(), "active" | "draft" | "prospect" | "hidden") {
            return Err(AppError::Validation(format!(
                "status must be 'all', 'active', 'draft', 'prospect' or 'hidden' (got '{c}')"
            )));
        }
    }

    let (page, per_page) = validate_pagination(q.page, q.per_page);
    let offset = (page - 1) * per_page;

    let row = sqlx::query(ROWS_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .bind(search)
        .bind(claim.as_deref())
        .bind(lifecycle.as_deref())
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
        .fetch_one(&s.db)
        .await?;

    let connected: bool = sqlx::query(CONNECTED_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .fetch_one(&s.db)
        .await?
        .try_get("connected")?;

    Ok(Json(json!({
        "network_id": q.network_id,
        "directory_id": q.directory_id,
        "q": search,
        "claim": claim.clone().unwrap_or_else(|| "all".to_string()),
        "status": lifecycle.clone().unwrap_or_else(|| "all".to_string()),
        "page": page,
        "per_page": per_page,
        "total": total,
        "counts": {
            "all": counts.try_get::<i64, _>("all_rows").unwrap_or(0),
            "claimed": counts.try_get::<i64, _>("claimed").unwrap_or(0),
            "unclaimed": counts.try_get::<i64, _>("unclaimed").unwrap_or(0),
            "published": counts.try_get::<i64, _>("published").unwrap_or(0),
            "drafts": counts.try_get::<i64, _>("drafts").unwrap_or(0),
            "prospects": counts.try_get::<i64, _>("prospects").unwrap_or(0),
            "hidden": counts.try_get::<i64, _>("hidden").unwrap_or(0),
        },
        "coreswift_connected": connected,
        "rows": rows,
    })))
}
