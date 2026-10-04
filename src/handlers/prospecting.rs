//! Supplier PROSPECTING — card B80 (operator-only, never public).
//!
//! David (2026-09-30): *"I don't pre-populate the suppliers ... But I do want to have a way to
//! search for them so I can reach out to them."* This module is that internal tool.
//!
//! It reuses the card-B81 lifecycle instead of a parallel system: a PROSPECT is an existing
//! `businesses` row created with `status='prospect'` (and `is_active=false`), so it is excluded
//! from every public surface by construction. The outreach state (pipeline status, notes, source,
//! append-only log) lives in `supplier_prospects` / `supplier_prospect_outreach` (migration 145).
//!
//! Endpoints (all behind `operator_guard`):
//!   * `GET  /admin/prospecting/search`                     — free-text candidate search (free source)
//!   * `GET  /admin/prospecting/prospects`                  — the saved prospect pipeline
//!   * `POST /admin/prospecting/prospects`                  — save one candidate as a prospect
//!   * `PUT  /admin/prospecting/prospects/:business_id`     — edit status / notes
//!   * `POST /admin/prospecting/prospects/:business_id/outreach` — append an outreach entry
//!   * `POST /admin/prospecting/prospects/:business_id/convert`  — publish the SAME row as a supplier
//!
//! Candidate search uses the free/open enrichment source (OpenStreetMap/Nominatim, card B79) — the
//! same source as the listing pipeline's zero-cost fallback — so it works with nothing configured.

use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// Business types that make a business a SUPPLIER (the second front). A prospect is always one of
/// these; `local` is widened to `supplier` on save and on convert. Kept in code (not user input)
/// so a typo'd parameter can never widen the population.
const SUPPLIER_TYPES: &[&str] = &[
    "supplier",
    "distributor",
    "wholesaler",
    "farm",
    "association",
    "manufacturer",
];

/// The prospect pipeline statuses. Mirrors the CHECK in migration 145.
const PROSPECT_STATUSES: &[&str] = &[
    "new",
    "contacted",
    "replied",
    "interested",
    "onboarded",
    "declined",
];

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

// ── GET /admin/prospecting/search ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CandidateQuery {
    /// Free-text query (e.g. "organic farm", "wholesaler").
    pub q: Option<String>,
    /// Optional supplier business_type hint, appended to the query for precision.
    #[serde(rename = "type")]
    pub type_hint: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub limit: Option<i64>,
}

pub async fn search_candidates(
    State(_s): State<AppState>,
    Query(q): Query<CandidateQuery>,
) -> ApiResult<impl IntoResponse> {
    let base = q.q.as_deref().map(str::trim).unwrap_or("");
    let type_hint = q
        .type_hint
        .as_deref()
        .map(str::trim)
        .filter(|t| SUPPLIER_TYPES.contains(t));
    if base.is_empty() && type_hint.is_none() {
        return Err(AppError::Validation(
            "Enter what to search for (e.g. 'organic farm', 'wholesaler').".to_string(),
        ));
    }
    let mut parts: Vec<&str> = Vec::new();
    if let Some(t) = type_hint {
        parts.push(t);
    }
    if !base.is_empty() {
        parts.push(base);
    }
    if let Some(c) = q.city.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(c);
    }
    if let Some(st) = q.state.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(st);
    }
    let query = parts.join(" ");
    let limit = q.limit.unwrap_or(20).clamp(1, 50);

    let candidates = crate::handlers::enrichment::search_external_candidates(&query, limit)
        .await
        .map_err(|e| {
            AppError::BadRequest(format!("The search source could not be reached: {e}"))
        })?;

    Ok(Json(json!({
        "query": query,
        "source": "openstreetmap",
        "count": candidates.len(),
        "candidates": candidates,
    })))
}

// ── GET /admin/prospecting/prospects ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ProspectsQuery {
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
}

const LIST_PROSPECTS_SQL: &str = r#"
SELECT b.id::text AS business_id,
       b.name AS name,
       COALESCE(b.business_type, 'supplier') AS business_type,
       b.address AS address, b.city AS city, b.state AS state, b.zip AS zip,
       b.phone AS phone, b.email AS email, b.website AS website,
       b.directory_id::text AS directory_id,
       d.name AS directory_name, d.slug AS directory_slug,
       sp.status AS prospect_status, sp.notes AS notes, sp.source AS source,
       sp.created_at::text AS created_at, sp.updated_at::text AS updated_at,
       (SELECT count(*) FROM supplier_prospect_outreach o WHERE o.business_id = b.id) AS outreach_count,
       (SELECT max(o.created_at)::text FROM supplier_prospect_outreach o WHERE o.business_id = b.id) AS last_outreach
FROM supplier_prospects sp
JOIN businesses b ON b.id = sp.business_id
LEFT JOIN directories d ON d.id = b.directory_id
WHERE ($1::uuid IS NULL OR d.network_id = $1)
  AND ($2::uuid IS NULL OR b.directory_id = $2)
  AND ($3::text IS NULL OR sp.status = $3)
ORDER BY sp.updated_at DESC
"#;

pub async fn list_prospects(
    State(s): State<AppState>,
    Query(q): Query<ProspectsQuery>,
) -> ApiResult<impl IntoResponse> {
    let status = q
        .status
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "all");
    if let Some(st) = status {
        if !PROSPECT_STATUSES.contains(&st) {
            return Err(AppError::Validation(format!(
                "status must be one of {} (got '{st}')",
                PROSPECT_STATUSES.join(", ")
            )));
        }
    }

    let rows = sqlx::query(LIST_PROSPECTS_SQL)
        .bind(q.network_id)
        .bind(q.directory_id)
        .bind(status)
        .fetch_all(&s.db)
        .await?;

    let prospects: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "business_id": r.try_get::<String, _>("business_id").unwrap_or_default(),
                "name": r.try_get::<String, _>("name").unwrap_or_default(),
                "business_type": r.try_get::<String, _>("business_type").unwrap_or_default(),
                "address": r.try_get::<Option<String>, _>("address").unwrap_or(None),
                "city": r.try_get::<Option<String>, _>("city").unwrap_or(None),
                "state": r.try_get::<Option<String>, _>("state").unwrap_or(None),
                "zip": r.try_get::<Option<String>, _>("zip").unwrap_or(None),
                "phone": r.try_get::<Option<String>, _>("phone").unwrap_or(None),
                "email": r.try_get::<Option<String>, _>("email").unwrap_or(None),
                "website": r.try_get::<Option<String>, _>("website").unwrap_or(None),
                "directory_id": r.try_get::<Option<String>, _>("directory_id").unwrap_or(None),
                "directory_name": r.try_get::<Option<String>, _>("directory_name").unwrap_or(None),
                "directory_slug": r.try_get::<Option<String>, _>("directory_slug").unwrap_or(None),
                "prospect_status": r.try_get::<String, _>("prospect_status").unwrap_or_default(),
                "notes": r.try_get::<Option<String>, _>("notes").unwrap_or(None),
                "source": r.try_get::<Option<String>, _>("source").unwrap_or(None),
                "created_at": r.try_get::<Option<String>, _>("created_at").unwrap_or(None),
                "updated_at": r.try_get::<Option<String>, _>("updated_at").unwrap_or(None),
                "outreach_count": r.try_get::<i64, _>("outreach_count").unwrap_or(0),
                "last_outreach": r.try_get::<Option<String>, _>("last_outreach").unwrap_or(None),
            })
        })
        .collect();

    Ok(Json(json!({
        "network_id": q.network_id,
        "directory_id": q.directory_id,
        "total": prospects.len(),
        "prospects": prospects,
    })))
}

// ── POST /admin/prospecting/prospects ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SaveProspectRequest {
    pub directory_id: Uuid,
    pub name: String,
    #[serde(rename = "type")]
    pub business_type: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub notes: Option<String>,
    pub source: Option<String>,
}

pub async fn save_prospect(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<SaveProspectRequest>,
) -> ApiResult<impl IntoResponse> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "The prospect needs a name.".to_string(),
        ));
    }
    let btype = req
        .business_type
        .as_deref()
        .map(str::trim)
        .filter(|t| SUPPLIER_TYPES.contains(t))
        .unwrap_or("supplier");

    // The directory must exist — the prospect belongs to a city so it is scoped like everything else.
    let dir_ok: Option<Uuid> = sqlx::query_scalar("SELECT id FROM directories WHERE id = $1")
        .bind(req.directory_id)
        .fetch_optional(&s.db)
        .await?;
    if dir_ok.is_none() {
        return Err(AppError::Validation(
            "Pick a directory for this prospect.".to_string(),
        ));
    }

    let created_by = Uuid::parse_str(&claims.sub).ok();
    let mut slug_base = slugify(name);
    if slug_base.is_empty() {
        slug_base = "prospect".to_string();
    }
    if slug_base.len() > 200 {
        slug_base.truncate(200);
    }
    let slug = format!(
        "{}-{}",
        slug_base,
        &Uuid::new_v4().simple().to_string()[..6]
    );

    let mut tx = s.db.begin().await?;

    let business_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO businesses
           (directory_id, name, slug, business_type, address, city, state, zip,
            phone, email, website, latitude, longitude, status, is_active)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                   'prospect', false)
           RETURNING id"#,
    )
    .bind(req.directory_id)
    .bind(name)
    .bind(&slug)
    .bind(btype)
    .bind(req.address.as_deref())
    .bind(req.city.as_deref())
    .bind(req.state.as_deref())
    .bind(req.zip.as_deref())
    .bind(req.phone.as_deref())
    .bind(req.email.as_deref())
    .bind(req.website.as_deref())
    .bind(req.latitude)
    .bind(req.longitude)
    .fetch_one(&mut *tx)
    .await?;

    let source = req
        .source
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("openstreetmap");
    sqlx::query(
        "INSERT INTO supplier_prospects (business_id, status, notes, source, created_by) \
         VALUES ($1, 'new', $2, $3, $4)",
    )
    .bind(business_id)
    .bind(req.notes.as_deref().unwrap_or(""))
    .bind(source)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "business_id": business_id.to_string(),
            "slug": slug,
            "prospect_status": "new",
            "message": "Saved as an internal prospect — it is hidden from the public directory.",
        })),
    ))
}

// ── PUT /admin/prospecting/prospects/:business_id ───────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateProspectRequest {
    pub status: Option<String>,
    pub notes: Option<String>,
}

pub async fn update_prospect(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
    Json(req): Json<UpdateProspectRequest>,
) -> ApiResult<impl IntoResponse> {
    if let Some(st) = req.status.as_deref() {
        if !PROSPECT_STATUSES.contains(&st) {
            return Err(AppError::Validation(format!(
                "status must be one of {} (got '{st}')",
                PROSPECT_STATUSES.join(", ")
            )));
        }
    }

    let row = sqlx::query(
        "UPDATE supplier_prospects SET \
             status = COALESCE($2, status), \
             notes = COALESCE($3, notes), \
             updated_at = now() \
         WHERE business_id = $1 \
         RETURNING status, notes",
    )
    .bind(business_id)
    .bind(req.status.as_deref())
    .bind(req.notes.as_deref())
    .fetch_optional(&s.db)
    .await?;

    let Some(row) = row else {
        return Err(AppError::NotFound(format!(
            "No prospect with id {business_id}"
        )));
    };

    Ok(Json(json!({
        "business_id": business_id.to_string(),
        "prospect_status": row.try_get::<String, _>("status")?,
        "notes": row.try_get::<Option<String>, _>("notes")?,
    })))
}

// ── POST /admin/prospecting/prospects/:business_id/outreach ─────────────────

#[derive(Debug, Deserialize)]
pub struct OutreachRequest {
    pub channel: Option<String>,
    pub note: Option<String>,
    /// Optionally move the pipeline forward in the same action.
    pub status: Option<String>,
}

const OUTREACH_CHANNELS: &[&str] = &["note", "email", "phone", "other"];

pub async fn add_outreach(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(business_id): Path<Uuid>,
    Json(req): Json<OutreachRequest>,
) -> ApiResult<impl IntoResponse> {
    let channel = req
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|c| OUTREACH_CHANNELS.contains(c))
        .unwrap_or("note");
    if let Some(st) = req.status.as_deref() {
        if !PROSPECT_STATUSES.contains(&st) {
            return Err(AppError::Validation(format!(
                "status must be one of {} (got '{st}')",
                PROSPECT_STATUSES.join(", ")
            )));
        }
    }

    // The prospect must exist before we log against it.
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT business_id FROM supplier_prospects WHERE business_id = $1")
            .bind(business_id)
            .fetch_optional(&s.db)
            .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(format!(
            "No prospect with id {business_id}"
        )));
    }

    let created_by = Uuid::parse_str(&claims.sub).ok();
    let mut tx = s.db.begin().await?;

    sqlx::query(
        "INSERT INTO supplier_prospect_outreach (business_id, channel, note, created_by) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(business_id)
    .bind(channel)
    .bind(req.note.as_deref().unwrap_or(""))
    .bind(created_by)
    .execute(&mut *tx)
    .await?;

    // Advance the pipeline: an explicit status wins; otherwise the first real contact moves a
    // brand-new prospect to 'contacted'. A note-only entry never changes the status.
    let new_status: String = if let Some(st) = req.status.as_deref() {
        sqlx::query_scalar(
            "UPDATE supplier_prospects SET status = $2, updated_at = now() \
             WHERE business_id = $1 RETURNING status",
        )
        .bind(business_id)
        .bind(st)
        .fetch_one(&mut *tx)
        .await?
    } else if channel == "email" || channel == "phone" {
        sqlx::query_scalar(
            "UPDATE supplier_prospects SET status = 'contacted', updated_at = now() \
             WHERE business_id = $1 AND status = 'new' RETURNING status",
        )
        .bind(business_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_else(|| "unchanged".to_string())
    } else {
        "unchanged".to_string()
    };

    tx.commit().await?;

    Ok(Json(json!({
        "business_id": business_id.to_string(),
        "logged": true,
        "prospect_status": new_status,
    })))
}

// ── POST /admin/prospecting/prospects/:business_id/convert ──────────────────

/// Explicit conversion: PROMOTE the same `businesses` row to a real, visible supplier. No second
/// record is created (mirrors the card-B82 adoption principle). The prospect's supplier type is
/// preserved, or widened from 'local' to 'supplier'.
pub async fn convert_prospect(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let mut tx = s.db.begin().await?;

    let promoted: Option<String> = sqlx::query_scalar(
        r#"UPDATE businesses b
              SET status = 'active',
                  is_active = true,
                  business_type = CASE
                      WHEN COALESCE(b.business_type, 'local') = 'local' THEN 'supplier'
                      ELSE b.business_type END,
                  updated_at = now()
            WHERE b.id = $1
              AND EXISTS (SELECT 1 FROM supplier_prospects sp WHERE sp.business_id = b.id)
            RETURNING b.business_type"#,
    )
    .bind(business_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(business_type) = promoted else {
        return Err(AppError::NotFound(format!(
            "No prospect with id {business_id}"
        )));
    };

    sqlx::query(
        "UPDATE supplier_prospects SET status = 'onboarded', updated_at = now() \
         WHERE business_id = $1",
    )
    .bind(business_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(json!({
        "business_id": business_id.to_string(),
        "prospect_status": "onboarded",
        "published": true,
        "business_type": business_type,
        "message": "Converted to a live supplier — the same record is now visible publicly.",
    })))
}
