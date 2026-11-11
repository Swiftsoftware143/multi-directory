//! Satellite town approval (W30).
//!
//! Town pages publish only for towns an admin has approved, one town at a time, and only while
//! the city's "include satellite towns" switch is on. This module is the console side of that
//! gate: it lists the towns a city actually holds (with their live listing counts), creates the
//! missing rows as drafts, and flips a single town on or off. The public gate lives in
//! `subfolder::local_town_entries`, which reads the same rows — so what the console shows and
//! what the site publishes can never disagree.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::handlers::subfolder::{local_town_entries, satellite_approval};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ApproveRequest {
    pub approved: bool,
}

#[derive(Debug, Deserialize)]
pub struct IncludeRequest {
    pub include_satellite_towns: bool,
}

/// GET /api/v1/seo/satellite-towns/:directory_id
///
/// Every town of this city with its own live listing count and its effective status:
/// `draft` (not approved — publishes nothing), `approved` (approved but currently below the
/// three-listing floor, so no page yet) or `live` (approved and publishing). Missing rows are
/// created as drafts on the way through, so a newly appearing town shows up here without
/// anybody having to seed it.
pub async fn list_satellite_towns(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let dir: Option<(String,)> = sqlx::query_as("SELECT city FROM directories WHERE id = $1")
        .bind(directory_id)
        .fetch_optional(&s.db)
        .await?;

    let (city,) = dir.ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let towns = local_town_entries(&s.db, directory_id, &city).await;
    let (include, approved) = satellite_approval(&s.db, directory_id).await;

    // Persist a draft row for every town the city actually holds, so the console and the gate
    // read the same set on the next request. A missing table (install mid-upgrade) is tolerated:
    // the gate itself fails open, and the list still renders.
    for t in &towns {
        let _ = sqlx::query(
            "INSERT INTO satellite_towns (directory_id, town_key, town_label) VALUES ($1, $2, $3) \
             ON CONFLICT (directory_id, town_key) DO NOTHING",
        )
        .bind(directory_id)
        .bind(&t.key)
        .bind(&t.label)
        .execute(&s.db)
        .await;
    }

    let rows: Vec<serde_json::Value> = towns
        .iter()
        .map(|t| {
            let is_approved = approved.get(&t.key).copied().unwrap_or(false);
            let status = if !is_approved {
                "draft"
            } else if t.published {
                "live"
            } else {
                "approved"
            };
            json!({
                "town_key": t.key,
                "town_label": t.label,
                "slug": t.slug,
                "count": t.count,
                "is_home": t.is_home,
                "approved": is_approved,
                "status": status,
            })
        })
        .collect();

    Ok(Json(json!({
        "directory_id": directory_id,
        "city": city,
        "include_satellite_towns": include,
        "min_listings": 3,
        "towns": rows,
    })))
}

/// PUT /api/v1/seo/satellite-towns/:directory_id/:town_key — approve or revoke ONE town.
pub async fn set_town_approval(
    State(s): State<AppState>,
    Path((directory_id, town_key)): Path<(Uuid, String)>,
    Json(body): Json<ApproveRequest>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let key = town_key.trim().to_lowercase();
    if key.is_empty() {
        return Err(AppError::BadRequest("town_key is required".to_string()));
    }

    let dir: Option<(String,)> = sqlx::query_as("SELECT city FROM directories WHERE id = $1")
        .bind(directory_id)
        .fetch_optional(&s.db)
        .await?;
    let (city,) = dir.ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    // Only a town this city actually holds inside its own local area can be approved — never a
    // free-text name that would publish an empty page.
    let towns = local_town_entries(&s.db, directory_id, &city).await;
    let town = towns
        .iter()
        .find(|t| t.key == key)
        .ok_or_else(|| AppError::BadRequest("That town is not part of this city".to_string()))?;
    let label = town.label.clone();

    sqlx::query(
        "INSERT INTO satellite_towns (directory_id, town_key, town_label, approved, approved_at) \
         VALUES ($1, $2, $3, $4, CASE WHEN $4 THEN now() ELSE NULL END) \
         ON CONFLICT (directory_id, town_key) DO UPDATE SET \
           approved = EXCLUDED.approved, \
           approved_at = EXCLUDED.approved_at, \
           town_label = EXCLUDED.town_label, \
           updated_at = now()",
    )
    .bind(directory_id)
    .bind(&key)
    .bind(&label)
    .bind(body.approved)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "town_key": key,
        "town_label": label,
        "approved": body.approved,
        "status": if body.approved { "approved" } else { "draft" },
    })))
}

/// PUT /api/v1/seo/satellite-settings/:directory_id — the city's "include satellite towns"
/// switch. Unchecking unpublishes every town page of that city (approvals are kept).
pub async fn set_satellite_include(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(body): Json<IncludeRequest>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let updated = sqlx::query(
        "UPDATE directories SET include_satellite_towns = $2, updated_at = now() WHERE id = $1",
    )
    .bind(directory_id)
    .bind(body.include_satellite_towns)
    .execute(&s.db)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    Ok(Json(json!({
        "directory_id": directory_id,
        "include_satellite_towns": body.include_satellite_towns,
    })))
}
