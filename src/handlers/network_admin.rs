//! Network admin — ONE management surface for a whole network (kanban B96).
//!
//! David's binding rule (2026-10-01): a NETWORK is one management surface. Anything that
//! belongs to any of its cities is managed in ONE place, with the CITY visible and
//! filterable on every record, so an operator running a 100-city network can answer
//! "which cities do these businesses belong to?" at a glance — without visiting each
//! city's admin separately and without SQL.
//!
//! Two operator-guarded read endpoints back the panel's "Network Management" card:
//!   * `GET /admin/network/counts`   — per-directory counts for every per-city dataset
//!   * `GET /admin/network/entities` — the rows for one dataset, each carrying its city
//!
//! Both take an optional `network_id` and/or `directory_id` scope (the panel passes the
//! scope chosen in the persistent context bar). Neither is required: with no scope the
//! view is the whole platform, which is what the operator's "All networks" scope means.
//!
//! Every statement is a compile-time literal and every filter is a bind, so nothing is
//! assembled at run time (class-14). A `count(*) OVER()` column rides each row so the
//! page total comes back in the same query, and the rows are aggregated to JSON in the
//! database — one round trip per request.

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
pub struct ScopeQuery {
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct EntityQuery {
    pub kind: Option<String>,
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub q: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// Per-directory counts for every per-city dataset, for the whole scope. One row per
/// directory so the panel can render a city column, counts per city, and a filter.
const COUNTS_SQL: &str = r#"
SELECT COALESCE(json_agg(t), '[]'::json) AS rows
FROM (
    SELECT d.id::text                          AS directory_id,
           d.name                              AS directory_name,
           d.slug                              AS directory_slug,
           COALESCE(d.city, d.name)            AS city,
           d.state                             AS state,
           d.is_primary                        AS is_primary,
           n.id::text                          AS network_id,
           COALESCE(n.name, 'Standalone')      AS network_name,
           (SELECT count(*) FROM businesses b
              WHERE b.directory_id = d.id
                AND COALESCE(b.business_type, 'local') = 'local'
                AND COALESCE(b.is_franchise, false) = false)                 AS businesses,
           (SELECT count(*) FROM businesses b
              WHERE b.directory_id = d.id
                AND COALESCE(b.business_type, 'local') <> 'local')           AS suppliers,
           (SELECT count(*) FROM claimed_businesses c
              JOIN businesses b ON b.id = c.business_id
              WHERE b.directory_id = d.id AND c.is_active)                   AS claims,
           (SELECT count(*) FROM shared_leads l
              JOIN businesses b ON b.id = l.poster_business_id
              WHERE b.directory_id = d.id)                                   AS leads,
           (SELECT count(*) FROM deals x WHERE x.directory_id = d.id)        AS deals,
           (SELECT count(*) FROM community_events e WHERE e.directory_id = d.id) AS events,
           (SELECT count(*) FROM business_articles a WHERE a.directory_id = d.id) AS articles,
           (SELECT count(*) FROM newsletter_subscribers s WHERE s.directory_id = d.id) AS subscribers
    FROM directories d
    LEFT JOIN networks n ON n.id = d.network_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
    ORDER BY COALESCE(n.name, 'zzz'), d.is_primary DESC, d.name
) t
"#;

// ── Entity rows, one compile-time literal per dataset ───────────────────────────
// Each returns `rows` (JSON array) + `total` (page total), bound $1 network, $2
// directory, $3 search text, $4 limit, $5 offset. Column names are kept identical
// across kinds so one table renders them all: id, name, city, state, directory_name,
// directory_slug, directory_id, type, status, created_at.

const BUSINESSES_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT b.id::text AS id, b.name AS name, COALESCE(b.city, d.city) AS city, b.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(b.business_type, 'local') AS type,
           CASE WHEN b.claimed THEN 'claimed' WHEN b.verified THEN 'verified' ELSE 'listed' END AS status,
           b.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM businesses b
    JOIN directories d ON d.id = b.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND COALESCE(b.business_type, 'local') = 'local'
      AND COALESCE(b.is_franchise, false) = false
      AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%')
    ORDER BY d.name, b.name
    LIMIT $4 OFFSET $5
) t
"#;

const SUPPLIERS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT b.id::text AS id, b.name AS name, COALESCE(b.city, d.city) AS city, b.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(b.business_type, 'supplier') AS type,
           CASE WHEN b.claimed THEN 'claimed' WHEN b.verified THEN 'verified' ELSE 'listed' END AS status,
           b.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM businesses b
    JOIN directories d ON d.id = b.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND COALESCE(b.business_type, 'local') <> 'local'
      AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%')
    ORDER BY d.name, b.name
    LIMIT $4 OFFSET $5
) t
"#;

const LEADS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT l.id::text AS id, l.title AS name, COALESCE(b.city, d.city) AS city, b.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(l.category, 'lead') AS type, l.status AS status,
           l.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM shared_leads l
    JOIN businesses b ON b.id = l.poster_business_id
    JOIN directories d ON d.id = b.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR l.title ILIKE '%' || $3 || '%'
                            OR COALESCE(l.category, '') ILIKE '%' || $3 || '%')
    ORDER BY d.name, l.created_at DESC
    LIMIT $4 OFFSET $5
) t
"#;

const CLAIMS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT c.id::text AS id, b.name AS name, COALESCE(b.city, d.city) AS city, b.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           c.owner_email AS type,
           CASE WHEN c.verified_at IS NOT NULL THEN 'verified' ELSE 'pending' END AS status,
           c.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM claimed_businesses c
    JOIN businesses b ON b.id = c.business_id
    JOIN directories d ON d.id = b.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR b.name ILIKE '%' || $3 || '%'
                            OR COALESCE(c.owner_email, '') ILIKE '%' || $3 || '%')
    ORDER BY d.name, c.created_at DESC
    LIMIT $4 OFFSET $5
) t
"#;

const DEALS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT x.id::text AS id, x.title AS name, COALESCE(b.city, d.city) AS city, d.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(x.status, 'draft') AS type, x.status AS status,
           x.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM deals x
    JOIN directories d ON d.id = x.directory_id
    LEFT JOIN businesses b ON b.id = x.business_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR x.title ILIKE '%' || $3 || '%')
    ORDER BY d.name, x.created_at DESC
    LIMIT $4 OFFSET $5
) t
"#;

const EVENTS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT e.id::text AS id, e.title AS name, COALESCE(d.city, d.name) AS city, d.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(e.status, 'draft') AS type, e.status AS status,
           e.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM community_events e
    JOIN directories d ON d.id = e.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR e.title ILIKE '%' || $3 || '%'
                            OR COALESCE(e.location, '') ILIKE '%' || $3 || '%')
    ORDER BY d.name, e.created_at DESC
    LIMIT $4 OFFSET $5
) t
"#;

const ARTICLES_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT a.id::text AS id, a.title AS name, COALESCE(d.city, d.name) AS city, d.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(a.status, 'draft') AS type, a.status AS status,
           a.created_at::text AS created_at,
           count(*) OVER() AS total
    FROM business_articles a
    JOIN directories d ON d.id = a.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR a.title ILIKE '%' || $3 || '%')
    ORDER BY d.name, a.created_at DESC
    LIMIT $4 OFFSET $5
) t
"#;

const SUBSCRIBERS_SQL: &str = r#"
SELECT COALESCE(jsonb_agg(row_to_json(t)::jsonb - 'total'), '[]'::jsonb) AS rows,
       COALESCE(max(t.total), 0) AS total
FROM (
    SELECT s.id::text AS id, COALESCE(s.name, s.email) AS name,
           COALESCE(d.city, d.name) AS city, d.state AS state,
           d.name AS directory_name, d.slug AS directory_slug, d.id::text AS directory_id,
           COALESCE(s.status, 'active') AS type, s.status AS status,
           COALESCE(s.subscribed_at, s.created_at)::text AS created_at,
           count(*) OVER() AS total
    FROM newsletter_subscribers s
    JOIN directories d ON d.id = s.directory_id
    WHERE ($1::uuid IS NULL OR d.network_id = $1)
      AND ($2::uuid IS NULL OR d.id = $2)
      AND ($3::text IS NULL OR COALESCE(s.name, '') ILIKE '%' || $3 || '%'
                            OR s.email ILIKE '%' || $3 || '%')
    ORDER BY d.name, COALESCE(s.subscribed_at, s.created_at) DESC
    LIMIT $4 OFFSET $5
) t
"#;

/// GET /admin/network/counts — per-city counts for the whole scope (operator-guarded).
pub async fn network_counts(
    State(s): State<AppState>,
    Query(q): Query<ScopeQuery>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query(COUNTS_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .fetch_one(&s.db)
        .await?;
    let rows: serde_json::Value = row.try_get("rows")?;

    Ok(Json(json!({
        "network_id": q.network_id,
        "directory_id": q.directory_id,
        "rows": rows,
    })))
}

/// GET /admin/network/entities?kind=... — the rows for one dataset, city shown per row
/// (operator-guarded). Accepts the kind allowlist; an unknown kind is a 422.
pub async fn network_entities(
    State(s): State<AppState>,
    Query(q): Query<EntityQuery>,
) -> ApiResult<impl IntoResponse> {
    let kind = q
        .kind
        .unwrap_or_else(|| "businesses".to_string())
        .to_ascii_lowercase();

    let sql = match kind.as_str() {
        "businesses" => BUSINESSES_SQL,
        "suppliers" => SUPPLIERS_SQL,
        "leads" => LEADS_SQL,
        "claims" => CLAIMS_SQL,
        "deals" => DEALS_SQL,
        "events" => EVENTS_SQL,
        "articles" => ARTICLES_SQL,
        "subscribers" => SUBSCRIBERS_SQL,
        other => {
            return Err(AppError::Validation(format!(
                "Unknown kind '{other}'. Expected one of: businesses, suppliers, leads, claims, deals, events, articles, subscribers."
            )))
        }
    };

    let (page, per_page) = validate_pagination(q.page, q.per_page);
    let offset = (page - 1) * per_page;
    let search = q.q.as_deref().map(str::trim).filter(|v| !v.is_empty());

    let row = sqlx::query(sql)
        .bind(q.network_id)
        .bind(q.directory_id)
        .bind(search)
        .bind(per_page)
        .bind(offset)
        .fetch_one(&s.db)
        .await?;

    let rows: serde_json::Value = row.try_get("rows")?;
    let total: i64 = row.try_get("total")?;

    Ok(Json(json!({
        "kind": kind,
        "network_id": q.network_id,
        "directory_id": q.directory_id,
        "q": search,
        "page": page,
        "per_page": per_page,
        "total": total,
        "rows": rows,
    })))
}
