//! Handlers: Directory Services & Locations

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .collect::<String>()
        .trim()
        .replace(' ', "-")
        .replace("--", "-")
}

// ── Services ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DirectoryService {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub is_active: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServiceReq { pub name: String, pub slug: Option<String>, pub description: Option<String> }
#[derive(Debug, Deserialize)]
pub struct UpdateServiceReq { pub name: Option<String>, pub slug: Option<String>, pub description: Option<String>, pub is_active: Option<bool> }

pub async fn list_services(State(s): State<AppState>, Path(dir_id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    Ok(Json(sqlx::query_as::<_, DirectoryService>(
        "SELECT * FROM directory_services WHERE directory_id = $1 ORDER BY name"
    ).bind(dir_id).fetch_all(&s.db).await?))
}

pub async fn create_service(State(s): State<AppState>, Path(dir_id): Path<Uuid>, Json(req): Json<CreateServiceReq>) -> ApiResult<impl IntoResponse> {
    let slug = req.slug.unwrap_or_else(|| slugify(&req.name));
    let svc = sqlx::query_as::<_, DirectoryService>(
        "INSERT INTO directory_services (directory_id,name,slug,description) VALUES($1,$2,$3,$4) RETURNING *"
    ).bind(dir_id).bind(&req.name).bind(&slug).bind(&req.description)
    .fetch_one(&s.db).await?;
    Ok((StatusCode::CREATED, Json(svc)))
}

pub async fn update_service(State(s): State<AppState>, Path((dir_id, svc_id)): Path<(Uuid, Uuid)>, Json(req): Json<UpdateServiceReq>) -> ApiResult<impl IntoResponse> {
    sqlx::query_as::<_, DirectoryService>("SELECT * FROM directory_services WHERE id=$1 AND directory_id=$2")
        .bind(svc_id).bind(dir_id).fetch_optional(&s.db).await?.ok_or(AppError::NotFound("Service".into()))?;
    let svc = sqlx::query_as::<_, DirectoryService>(
        "UPDATE directory_services SET name=COALESCE($1,name),slug=COALESCE($2,slug),description=COALESCE($3,description),is_active=COALESCE($4,is_active),updated_at=NOW() WHERE id=$5 AND directory_id=$6 RETURNING *"
    ).bind(&req.name).bind(&req.slug).bind(&req.description).bind(req.is_active).bind(svc_id).bind(dir_id)
    .fetch_one(&s.db).await?;
    Ok(Json(svc))
}

pub async fn delete_service(State(s): State<AppState>, Path((dir_id, svc_id)): Path<(Uuid, Uuid)>) -> ApiResult<impl IntoResponse> {
    let r = sqlx::query("DELETE FROM directory_services WHERE id=$1 AND directory_id=$2").bind(svc_id).bind(dir_id).execute(&s.db).await?;
    if r.rows_affected() == 0 { return Err(AppError::NotFound("Service".into())); }
    Ok(Json(json!({"ok":true})))
}

// ── Locations ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DirectoryLocation {
    pub id: Uuid, pub directory_id: Uuid, pub name: String, pub slug: String,
    pub state: Option<String>, pub region: Option<String>, pub is_active: Option<bool>,
    pub created_at: Option<DateTime<Utc>>, pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLocationReq { pub name: String, pub slug: Option<String>, pub state: Option<String>, pub region: Option<String> }
#[derive(Debug, Deserialize)]
pub struct UpdateLocationReq { pub name: Option<String>, pub slug: Option<String>, pub state: Option<String>, pub region: Option<String>, pub is_active: Option<bool> }

pub async fn list_locations(State(s): State<AppState>, Path(dir_id): Path<Uuid>) -> ApiResult<impl IntoResponse> {
    Ok(Json(sqlx::query_as::<_, DirectoryLocation>(
        "SELECT * FROM directory_locations WHERE directory_id=$1 ORDER BY name"
    ).bind(dir_id).fetch_all(&s.db).await?))
}

pub async fn create_location(State(s): State<AppState>, Path(dir_id): Path<Uuid>, Json(req): Json<CreateLocationReq>) -> ApiResult<impl IntoResponse> {
    let slug = req.slug.unwrap_or_else(|| slugify(&req.name));
    let loc = sqlx::query_as::<_, DirectoryLocation>(
        "INSERT INTO directory_locations (directory_id,name,slug,state,region) VALUES($1,$2,$3,$4,$5) RETURNING *"
    ).bind(dir_id).bind(&req.name).bind(&slug).bind(&req.state).bind(&req.region)
    .fetch_one(&s.db).await?;
    Ok((StatusCode::CREATED, Json(loc)))
}

pub async fn update_location(State(s): State<AppState>, Path((dir_id, loc_id)): Path<(Uuid, Uuid)>, Json(req): Json<UpdateLocationReq>) -> ApiResult<impl IntoResponse> {
    sqlx::query_as::<_, DirectoryLocation>("SELECT * FROM directory_locations WHERE id=$1 AND directory_id=$2")
        .bind(loc_id).bind(dir_id).fetch_optional(&s.db).await?.ok_or(AppError::NotFound("Location".into()))?;
    let loc = sqlx::query_as::<_, DirectoryLocation>(
        "UPDATE directory_locations SET name=COALESCE($1,name),slug=COALESCE($2,slug),state=COALESCE($3,state),region=COALESCE($4,region),is_active=COALESCE($5,is_active),updated_at=NOW() WHERE id=$6 AND directory_id=$7 RETURNING *"
    ).bind(&req.name).bind(&req.slug).bind(&req.state).bind(&req.region).bind(req.is_active).bind(loc_id).bind(dir_id)
    .fetch_one(&s.db).await?;
    Ok(Json(loc))
}

pub async fn delete_location(State(s): State<AppState>, Path((dir_id, loc_id)): Path<(Uuid, Uuid)>) -> ApiResult<impl IntoResponse> {
    let r = sqlx::query("DELETE FROM directory_locations WHERE id=$1 AND directory_id=$2").bind(loc_id).bind(dir_id).execute(&s.db).await?;
    if r.rows_affected() == 0 { return Err(AppError::NotFound("Location".into())); }
    Ok(Json(json!({"ok":true})))
}

// ── Bulk CSV Import for Services/Locations ──

#[derive(Debug, Deserialize)]
pub struct CsvImportReq {
    pub rows: Vec<Vec<String>>, // each row: [name, slug?, description/state?, region?]
    pub target: String, // "services" or "locations"
}

pub async fn csv_import(State(s): State<AppState>, Path(dir_id): Path<Uuid>, Json(req): Json<CsvImportReq>) -> ApiResult<impl IntoResponse> {
    let mut created = 0;
    let mut skipped = 0;
    for row in &req.rows {
        if row.is_empty() { continue; }
        let name = &row[0];
        let slug = row.get(1).filter(|x| !x.is_empty()).map(|x| slugify(x)).unwrap_or_else(|| slugify(name));
        if req.target == "services" {
            let desc = row.get(2);
            let ex: Option<(i64,)> = sqlx::query_as("SELECT COUNT(*) FROM directory_services WHERE directory_id=$1 AND slug=$2")
                .bind(dir_id).bind(&slug).fetch_optional(&s.db).await?;
            if ex.map(|e| e.0).unwrap_or(0) > 0 { skipped += 1; continue; }
            sqlx::query("INSERT INTO directory_services (directory_id,name,slug,description) VALUES($1,$2,$3,$4)")
                .bind(dir_id).bind(name).bind(&slug).bind(desc).execute(&s.db).await?;
            created += 1;
        } else {
            let state = row.get(2);
            let region = row.get(3);
            let ex: Option<(i64,)> = sqlx::query_as("SELECT COUNT(*) FROM directory_locations WHERE directory_id=$1 AND slug=$2")
                .bind(dir_id).bind(&slug).fetch_optional(&s.db).await?;
            if ex.map(|e| e.0).unwrap_or(0) > 0 { skipped += 1; continue; }
            sqlx::query("INSERT INTO directory_locations (directory_id,name,slug,state,region) VALUES($1,$2,$3,$4,$5)")
                .bind(dir_id).bind(name).bind(&slug).bind(state).bind(region).execute(&s.db).await?;
            created += 1;
        }
    }
    Ok(Json(json!({"created": created, "skipped": skipped})))
}
