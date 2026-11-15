//! Business facts: the first-hand details drafting must use.
//!
//! A draft that only knows the city and the keyword reads like every other post. Facts give it
//! the business's own services, areas, prices, jobs and quotes. They are per directory, and
//! optionally per business (a directory-wide fact applies to every draft in that directory).

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::handlers::tenant_scope;
use crate::AppState;

pub const KINDS: [&str; 8] = [
    "service",
    "area",
    "price",
    "job",
    "quote",
    "photo",
    "credential",
    "differentiator",
];

#[derive(Debug, Serialize)]
pub struct Fact {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub business_id: Option<Uuid>,
    pub kind: String,
    pub content: String,
    pub source: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct FactRequest {
    pub kind: String,
    pub content: String,
    pub source: Option<String>,
    pub business_id: Option<Uuid>,
    pub is_active: Option<bool>,
}

/// The facts a draft for this directory is told to use, most specific first (business-level
/// facts before directory-level ones, newest first inside each group).
pub async fn for_prompt(
    db: &PgPool,
    directory_id: Uuid,
    business_id: Option<Uuid>,
    limit: i64,
) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT content FROM business_facts \
         WHERE directory_id = $1 AND is_active \
           AND ($2::uuid IS NULL OR business_id IS NULL OR business_id = $2) \
         ORDER BY (business_id IS NOT NULL) DESC, created_at DESC LIMIT $3",
    )
    .bind(directory_id)
    .bind(business_id)
    .bind(limit)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|(c,)| c).collect()
}

/// How many facts the text actually uses, matched on a distinctive fragment of each fact.
pub async fn usage(db: &PgPool, directory_id: Uuid, text_lower: &str) -> (usize, usize) {
    let facts: Vec<(String,)> = sqlx::query_as(
        "SELECT content FROM business_facts WHERE directory_id = $1 AND is_active LIMIT 200",
    )
    .bind(directory_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let total = facts.len();
    let used = facts
        .iter()
        .filter(|(c,)| {
            let probe: String = c.to_lowercase().chars().take(40).collect();
            probe.trim().len() >= 12 && text_lower.contains(probe.trim())
        })
        .count();
    (used, total)
}

async fn assert_writer(s: &AppState, headers: &HeaderMap, dir: Uuid) -> Result<(), AppError> {
    let claims = crate::handlers::blog::caller(s, headers)?;
    tenant_scope::assert_directory_content_writer(&s.db, &claims, dir).await
}

fn validate_kind(kind: &str) -> Result<String, AppError> {
    let k = kind.trim().to_lowercase();
    if !KINDS.contains(&k.as_str()) {
        return Err(AppError::Validation(format!(
            "kind must be one of: {}",
            KINDS.join(", ")
        )));
    }
    Ok(k)
}

/// GET /api/v1/admin/directories/:id/business-facts
pub async fn list_facts(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    assert_writer(&s, &headers, id).await?;
    let business_id = q.get("business_id").and_then(|v| Uuid::parse_str(v).ok());
    let rows = sqlx::query(
        "SELECT id, directory_id, business_id, kind, content, source, is_active, created_at \
         FROM business_facts WHERE directory_id = $1 \
           AND ($2::uuid IS NULL OR business_id = $2) \
         ORDER BY created_at DESC LIMIT 500",
    )
    .bind(id)
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;
    let items: Vec<Fact> = rows
        .into_iter()
        .map(|r| Fact {
            id: r.get("id"),
            directory_id: r.get("directory_id"),
            business_id: r.get("business_id"),
            kind: r.get("kind"),
            content: r.get("content"),
            source: r.get("source"),
            is_active: r.get("is_active"),
            created_at: r.get("created_at"),
        })
        .collect();
    Ok(Json(json!({"items": items, "kinds": KINDS})))
}

/// POST /api/v1/admin/directories/:id/business-facts
pub async fn create_fact(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<FactRequest>,
) -> ApiResult<impl IntoResponse> {
    assert_writer(&s, &headers, id).await?;
    let kind = validate_kind(&req.kind)?;
    let content = req.content.trim();
    if content.len() < 8 {
        return Err(AppError::Validation(
            "a fact needs at least 8 characters".into(),
        ));
    }
    let row = sqlx::query(
        "INSERT INTO business_facts (directory_id, business_id, kind, content, source) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(id)
    .bind(req.business_id)
    .bind(kind)
    .bind(content)
    .bind(
        req.source
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    )
    .fetch_one(&s.db)
    .await?;
    let fact_id: Uuid = row.get("id");
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({"id": fact_id})),
    ))
}

/// PUT /api/v1/admin/business-facts/:id
pub async fn update_fact(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(fact_id): Path<Uuid>,
    Json(req): Json<FactRequest>,
) -> ApiResult<impl IntoResponse> {
    let dir: Option<(Uuid,)> =
        sqlx::query_as("SELECT directory_id FROM business_facts WHERE id = $1")
            .bind(fact_id)
            .fetch_optional(&s.db)
            .await?;
    let dir_id = dir
        .ok_or_else(|| AppError::NotFound("Fact not found".into()))?
        .0;
    assert_writer(&s, &headers, dir_id).await?;
    let kind = validate_kind(&req.kind)?;
    sqlx::query(
        "UPDATE business_facts SET kind = $2, content = $3, source = $4, \
                business_id = COALESCE($5, business_id), \
                is_active = COALESCE($6, is_active), updated_at = now() \
         WHERE id = $1",
    )
    .bind(fact_id)
    .bind(kind)
    .bind(req.content.trim())
    .bind(
        req.source
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    )
    .bind(req.business_id)
    .bind(req.is_active)
    .execute(&s.db)
    .await?;
    Ok(Json(json!({"status": "updated"})))
}

/// DELETE /api/v1/admin/business-facts/:id
pub async fn delete_fact(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(fact_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let dir: Option<(Uuid,)> =
        sqlx::query_as("SELECT directory_id FROM business_facts WHERE id = $1")
            .bind(fact_id)
            .fetch_optional(&s.db)
            .await?;
    let dir_id = dir
        .ok_or_else(|| AppError::NotFound("Fact not found".into()))?
        .0;
    assert_writer(&s, &headers, dir_id).await?;
    sqlx::query("DELETE FROM business_facts WHERE id = $1")
        .bind(fact_id)
        .execute(&s.db)
        .await?;
    Ok(Json(json!({"status": "deleted"})))
}
