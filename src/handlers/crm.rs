//! CRM (Contacts, Pipelines, Deals) CRUD handlers for Multi-Directory API.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc, NaiveDate};

use serde_json::json;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Contact ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrmContact {
    pub id: Uuid,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub position: Option<String>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub last_contacted_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateContactRequest {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub position: Option<String>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub last_contacted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateContactRequest {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub position: Option<String>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub source: Option<String>,
    pub assigned_to: Option<String>,
    pub last_contacted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ContactSearchQuery {
    pub q: Option<String>,
}

/// GET /api/v1/crm/contacts
pub async fn list_contacts(
    State(s): State<AppState>,
) -> ApiResult<Json<Vec<CrmContact>>> {
    let contacts = sqlx::query_as::<_, CrmContact>(
        "SELECT id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at FROM crm_contacts ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(contacts))
}

/// GET /api/v1/crm/contacts/search?q=
pub async fn search_contacts(
    State(s): State<AppState>,
    Query(q): Query<ContactSearchQuery>,
) -> ApiResult<impl IntoResponse> {
    let search_term = q.q.unwrap_or_default();
    let contacts = if search_term.trim().is_empty() {
        sqlx::query_as::<_, CrmContact>(
            "SELECT id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at FROM crm_contacts ORDER BY created_at DESC "
        )
        .fetch_all(&s.db)
        .await?
    } else {
        let pattern = format!("%{}%", search_term);
        sqlx::query_as::<_, CrmContact>(
            "SELECT id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at FROM crm_contacts WHERE first_name ILIKE \x241 OR last_name ILIKE \x241 OR email ILIKE \x241 OR phone ILIKE \x241 OR company ILIKE \x241 ORDER BY created_at DESC "
        )
        .bind(&pattern)
        .fetch_all(&s.db)
        .await?
    };

    Ok(Json(contacts))
}

/// POST /api/v1/crm/contacts
pub async fn create_contact(
    State(s): State<AppState>,
    Json(req): Json<CreateContactRequest>,
) -> ApiResult<impl IntoResponse> {
    let contact = sqlx::query_as::<_, CrmContact>(
        "INSERT INTO crm_contacts (first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413) RETURNING id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at "
    )
    .bind(&req.first_name)
    .bind(&req.last_name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.company)
    .bind(&req.position)
    .bind(req.directory_id)
    .bind(req.status.as_deref().unwrap_or("lead"))
    .bind(&req.tags)
    .bind(&req.notes)
    .bind(req.source.as_deref().unwrap_or("manual"))
    .bind(&req.assigned_to)
    .bind(req.last_contacted_at)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(contact)))
}

/// GET /api/v1/crm/contacts/:id
pub async fn get_contact(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let contact = sqlx::query_as::<_, CrmContact>(
        "SELECT id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at FROM crm_contacts WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Contact not found".into()))?;

    Ok(Json(contact))
}

/// PUT /api/v1/crm/contacts/:id
pub async fn update_contact(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateContactRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, CrmContact>(
        "SELECT id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at FROM crm_contacts WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Contact not found".into()))?;

    let contact = sqlx::query_as::<_, CrmContact>(
        "UPDATE crm_contacts SET first_name = \x241, last_name = \x242, email = \x243, phone = \x244, company = \x245, position = \x246, directory_id = \x247, status = \x248, tags = \x249, notes = \x2410, source = \x2411, assigned_to = \x2412, last_contacted_at = \x2413, updated_at = NOW() WHERE id = \x2414 RETURNING id, first_name, last_name, email, phone, company, position, directory_id, status, tags, notes, source, assigned_to, last_contacted_at, created_at, updated_at "
    )
    .bind(req.first_name.or(existing.first_name))
    .bind(req.last_name.or(existing.last_name))
    .bind(req.email.or(existing.email))
    .bind(req.phone.or(existing.phone))
    .bind(req.company.or(existing.company))
    .bind(req.position.or(existing.position))
    .bind(req.directory_id.or(existing.directory_id))
    .bind(req.status.as_deref().unwrap_or(existing.status.as_deref().unwrap_or("lead")))
    .bind(req.tags.or(existing.tags))
    .bind(req.notes.or(existing.notes))
    .bind(req.source.as_deref().unwrap_or(existing.source.as_deref().unwrap_or("manual")))
    .bind(req.assigned_to.or(existing.assigned_to))
    .bind(req.last_contacted_at.or(existing.last_contacted_at))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(contact))
}

/// DELETE /api/v1/crm/contacts/:id
pub async fn delete_contact(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM crm_contacts WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Contact not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Pipeline ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrmPipeline {
    pub id: Uuid,
    pub name: String,
    pub stages: Option<serde_json::Value>,
    pub directory_id: Option<Uuid>,
    pub default_pipeline: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePipelineRequest {
    pub name: String,
    pub stages: Option<serde_json::Value>,
    pub directory_id: Option<Uuid>,
    pub default_pipeline: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePipelineRequest {
    pub name: Option<String>,
    pub stages: Option<serde_json::Value>,
    pub directory_id: Option<Uuid>,
    pub default_pipeline: Option<bool>,
}

/// GET /api/v1/crm/pipelines
pub async fn list_pipelines(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let pipelines = sqlx::query_as::<_, CrmPipeline>(
        "SELECT id, name, stages, directory_id, default_pipeline, created_at FROM crm_pipelines ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(pipelines))
}

/// POST /api/v1/crm/pipelines
pub async fn create_pipeline(
    State(s): State<AppState>,
    Json(req): Json<CreatePipelineRequest>,
) -> ApiResult<impl IntoResponse> {
    let stages = req.stages.unwrap_or_else(|| serde_json::json!(["Lead", "Contacted", "Qualified", "Negotiation", "Closed"]));

    let pipeline = sqlx::query_as::<_, CrmPipeline>(
        "INSERT INTO crm_pipelines (name, stages, directory_id, default_pipeline) VALUES (\x241, \x242, \x243, \x244) RETURNING id, name, stages, directory_id, default_pipeline, created_at "
    )
    .bind(&req.name)
    .bind(&stages)
    .bind(req.directory_id)
    .bind(req.default_pipeline.unwrap_or(false))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(pipeline)))
}

/// GET /api/v1/crm/pipelines/:id
pub async fn get_pipeline(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let pipeline = sqlx::query_as::<_, CrmPipeline>(
        "SELECT id, name, stages, directory_id, default_pipeline, created_at FROM crm_pipelines WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Pipeline not found".into()))?;

    Ok(Json(pipeline))
}

/// PUT /api/v1/crm/pipelines/:id
pub async fn update_pipeline(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePipelineRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, CrmPipeline>(
        "SELECT id, name, stages, directory_id, default_pipeline, created_at FROM crm_pipelines WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Pipeline not found".into()))?;

    let pipeline = sqlx::query_as::<_, CrmPipeline>(
        "UPDATE crm_pipelines SET name = \x241, stages = \x242, directory_id = \x243, default_pipeline = \x244 WHERE id = \x245 RETURNING id, name, stages, directory_id, default_pipeline, created_at "
    )
    .bind(req.name.unwrap_or(existing.name))
    .bind(req.stages.or(existing.stages))
    .bind(req.directory_id.or(existing.directory_id))
    .bind(req.default_pipeline.unwrap_or(existing.default_pipeline.unwrap_or(false)))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(pipeline))
}

/// DELETE /api/v1/crm/pipelines/:id
pub async fn delete_pipeline(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM crm_pipelines WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Pipeline not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Deal Records ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrmDeal {
    pub id: Uuid,
    pub title: String,
    pub contact_id: Option<Uuid>,
    pub value: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub pipeline_id: Option<Uuid>,
    pub stage: Option<String>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub expected_close_date: Option<NaiveDate>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDealRequest {
    pub title: String,
    pub contact_id: Option<Uuid>,
    pub value: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub pipeline_id: Option<Uuid>,
    pub stage: Option<String>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub expected_close_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDealRequest {
    pub title: Option<String>,
    pub contact_id: Option<Uuid>,
    pub value: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub pipeline_id: Option<Uuid>,
    pub stage: Option<String>,
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
    pub expected_close_date: Option<NaiveDate>,
}

/// GET /api/v1/crm/deals
pub async fn list_deals(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let deals = sqlx::query_as::<_, CrmDeal>(
        "SELECT id, title, contact_id, value, currency, pipeline_id, stage, status, directory_id, expected_close_date, created_at, updated_at FROM crm_deal_records ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(deals))
}

/// POST /api/v1/crm/deals
pub async fn create_deal(
    State(s): State<AppState>,
    Json(req): Json<CreateDealRequest>,
) -> ApiResult<impl IntoResponse> {
    let deal = sqlx::query_as::<_, CrmDeal>(
        "INSERT INTO crm_deal_records (title, contact_id, value, currency, pipeline_id, stage, status, directory_id, expected_close_date) VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249) RETURNING id, title, contact_id, value, currency, pipeline_id, stage, status, directory_id, expected_close_date, created_at, updated_at "
    )
    .bind(&req.title)
    .bind(req.contact_id)
    .bind(req.value)
    .bind(req.currency.as_deref().unwrap_or("USD"))
    .bind(req.pipeline_id)
    .bind(&req.stage)
    .bind(req.status.as_deref().unwrap_or("open"))
    .bind(req.directory_id)
    .bind(req.expected_close_date)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(deal)))
}

/// GET /api/v1/crm/deals/:id
pub async fn get_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let deal = sqlx::query_as::<_, CrmDeal>(
        "SELECT id, title, contact_id, value, currency, pipeline_id, stage, status, directory_id, expected_close_date, created_at, updated_at FROM crm_deal_records WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Deal not found".into()))?;

    Ok(Json(deal))
}

/// PUT /api/v1/crm/deals/:id
pub async fn update_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDealRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, CrmDeal>(
        "SELECT id, title, contact_id, value, currency, pipeline_id, stage, status, directory_id, expected_close_date, created_at, updated_at FROM crm_deal_records WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Deal not found".into()))?;

    let deal = sqlx::query_as::<_, CrmDeal>(
        "UPDATE crm_deal_records SET title = \x241, contact_id = \x242, value = \x243, currency = \x244, pipeline_id = \x245, stage = \x246, status = \x247, directory_id = \x248, expected_close_date = \x249, updated_at = NOW() WHERE id = \x2410 RETURNING id, title, contact_id, value, currency, pipeline_id, stage, status, directory_id, expected_close_date, created_at, updated_at "
    )
    .bind(req.title.unwrap_or(existing.title))
    .bind(req.contact_id.or(existing.contact_id))
    .bind(req.value.or(existing.value))
    .bind(req.currency.as_deref().unwrap_or(existing.currency.as_deref().unwrap_or("USD")))
    .bind(req.pipeline_id.or(existing.pipeline_id))
    .bind(req.stage.or(existing.stage))
    .bind(req.status.as_deref().unwrap_or(existing.status.as_deref().unwrap_or("open")))
    .bind(req.directory_id.or(existing.directory_id))
    .bind(req.expected_close_date.or(existing.expected_close_date))
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(deal))
}

/// DELETE /api/v1/crm/deals/:id
pub async fn delete_deal(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM crm_deal_records WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Deal not found".into()));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ── Directory CRM Stats ───────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DirectoryCrmStats {
    pub directory_id: Uuid,
    pub directory_name: String,
    pub total_contacts: i64,
    pub total_deals: i64,
    pub deals_won: i64,
    pub deals_lost: i64,
    pub deals_open: i64,
    pub pipeline_value: Option<rust_decimal::Decimal>,
    pub contacts_by_status: Vec<StatusCount>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StatusCount {
    pub status: Option<String>,
    pub count: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DirectorySlugId {
    pub id: Uuid,
    pub name: String,
}

/// GET /api/v1/directories/:slug/crm/stats
pub async fn directory_crm_stats(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, DirectorySlugId>(
        "SELECT id, name FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let total_contacts = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM crm_contacts WHERE directory_id = \x241 "
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let total_deals = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM crm_deal_records WHERE directory_id = \x241 "
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let deals_won = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM crm_deal_records WHERE directory_id = \x241 AND status = 'won'"
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let deals_lost = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM crm_deal_records WHERE directory_id = \x241 AND status = 'lost'"
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let deals_open = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM crm_deal_records WHERE directory_id = \x241 AND status = 'open'"
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    let pipeline_value = sqlx::query_scalar::<_, rust_decimal::Decimal>(
        "SELECT COALESCE(SUM(value), 0) FROM crm_deal_records WHERE directory_id = \x241 AND status = 'open'"
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .ok();

    let contacts_by_status: Vec<StatusCount> = sqlx::query_as::<_, StatusCount>(
        "SELECT status, COUNT(*) as count FROM crm_contacts WHERE directory_id = \x241 GROUP BY status ORDER BY status "
    )
    .bind(dir.id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    Ok(Json(DirectoryCrmStats {
        directory_id: dir.id,
        directory_name: dir.name,
        total_contacts,
        total_deals,
        deals_won,
        deals_lost,
        deals_open,
        pipeline_value,
        contacts_by_status,
    }))
}
