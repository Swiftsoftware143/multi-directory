//! Import/Export handlers for Multi-Directory API.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;
use sqlx::Row;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::models::*;

// ── Import ────────────────────────────────────────────────────────────────

pub async fn import_data(
    State(state): State<AppState>,
    Json(req): Json<ImportDataRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.data.is_empty() {
        return Err(AppError::Validation("No data provided for import".to_string()));
    }
    let valid_entities = ["businesses", "reviews", "contacts", "deals"];
    if !valid_entities.contains(&req.entity_type.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid entity_type: {}. Must be one of: {}",
            req.entity_type, valid_entities.join(", ")
        )));
    }
    let log_entry = sqlx::query_as::<_, ImportLog>(
        "INSERT INTO import_logs (entity_type, filename, rows_total, status) VALUES ($1, $2, $3, 'processing') RETURNING *"
    )
    .bind(&req.entity_type)
    .bind(format!("import-{}.json", req.entity_type))
    .bind(req.data.len() as i32)
    .fetch_one(&state.db)
    .await?;
    let mut success = 0i32;
    let mut failed = 0i32;
    let mut errors: Vec<serde_json::Value> = Vec::new();
    for (idx, row) in req.data.iter().enumerate() {
        match import_single_row(&state.db, &req.entity_type, row, req.directory_id).await {
            Ok(_) => success += 1,
            Err(e) => {
                failed += 1;
                errors.push(json!({"row": idx, "error": e.to_string(), "data": row}));
            }
        }
    }
    let final_status = if failed == 0 { "completed" } else if success == 0 { "failed" } else { "completed" };
    sqlx::query(
        "UPDATE import_logs SET rows_success = $1, rows_failed = $2, errors = $3::jsonb, status = $4 WHERE id = $5"
    )
    .bind(success).bind(failed)
    .bind(serde_json::to_value(&errors).unwrap_or(json!([])))
    .bind(final_status).bind(log_entry.id)
    .execute(&state.db).await?;
    Ok(Json(json!(ImportResult {
        import_log_id: log_entry.id,
        rows_total: req.data.len() as i32,
        rows_success: success,
        rows_failed: failed,
        errors,
        status: final_status.to_string(),
    })))
}

async fn import_single_row(
    db: &sqlx::PgPool, entity_type: &str,
    row: &serde_json::Value, directory_id: Option<Uuid>,
) -> Result<(), AppError> {
    match entity_type {
        "businesses" => import_business_row(db, row, directory_id).await,
        "reviews" => import_review_row(db, row, directory_id).await,
        "contacts" => import_contact_row(db, row, directory_id).await,
        "deals" => import_deal_row(db, row, directory_id).await,
        _ => Err(AppError::Internal(format!("Unknown entity type: {}", entity_type))),
    }
}

async fn import_business_row(
    db: &sqlx::PgPool, row: &serde_json::Value, dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let name = row.get("name").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("Missing 'name' field".to_string()))?;
    let slug = row.get("slug").and_then(|v| v.as_str()).unwrap_or(name);
    let directory_id = row.get("directory_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or(dir_id)
        .ok_or_else(|| AppError::Validation("Missing 'directory_id' field".to_string()))?;
    sqlx::query(
        "INSERT INTO businesses (name, slug, directory_id) VALUES ($1, $2, $3)"
    )
    .bind(name).bind(slug).bind(directory_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn import_review_row(
    db: &sqlx::PgPool, row: &serde_json::Value, _dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let business_id = row.get("business_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| AppError::Validation("Missing 'business_id' field".to_string()))?;
    let rating = row.get("rating").and_then(|v| v.as_i64()).unwrap_or(5) as i32;
    let content = row.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let reviewer_name = row.get("reviewer_name").or_else(|| row.get("author")).and_then(|v| v.as_str()).unwrap_or("Anonymous");
    sqlx::query(
        "INSERT INTO reviews (business_id, rating, content, reviewer_name) VALUES ($1, $2, $3, $4)"
    )
    .bind(business_id).bind(rating).bind(content).bind(reviewer_name)
    .execute(db).await?;
    Ok(())
}

async fn import_contact_row(
    db: &sqlx::PgPool, row: &serde_json::Value, dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let first_name = row.get("first_name").and_then(|v| v.as_str()).or_else(|| row.get("name").and_then(|v| v.as_str())).unwrap_or("Unknown");
    let last_name = row.get("last_name").and_then(|v| v.as_str()).unwrap_or("");
    let email = row.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let phone = row.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let directory_id = row.get("directory_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or(dir_id);
    sqlx::query(
        "INSERT INTO crm_contacts (first_name, last_name, email, phone, directory_id) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(first_name).bind(last_name).bind(email).bind(phone).bind(directory_id)
    .execute(db).await?;
    Ok(())
}

async fn import_deal_row(
    db: &sqlx::PgPool, row: &serde_json::Value, dir_id: Option<Uuid>,
) -> Result<(), AppError> {
    let title = row.get("title").and_then(|v| v.as_str()).ok_or_else(|| AppError::Validation("Missing 'title' field".to_string()))?;
    let description = row.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let directory_id = row.get("directory_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or(dir_id)
        .ok_or_else(|| AppError::Validation("Missing 'directory_id' field".to_string()))?;
    sqlx::query(
        "INSERT INTO deals (title, description, directory_id) VALUES ($1, $2, $3)"
    )
    .bind(title).bind(description).bind(directory_id)
    .execute(db).await?;
    Ok(())
}

// ── Import Logs ───────────────────────────────────────────────────────────

pub async fn list_import_logs(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let logs = sqlx::query_as::<_, ImportLog>(
        "SELECT * FROM import_logs ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(logs)))
}

pub async fn get_import_log(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let log = sqlx::query_as::<_, ImportLog>(
        "SELECT * FROM import_logs WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Import log not found".to_string()))?;
    Ok(Json(json!(log)))
}

// ── Export ────────────────────────────────────────────────────────────────

pub async fn export_businesses(
    State(state): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let businesses = sqlx::query_as::<_, Business>(
        "SELECT * FROM businesses WHERE directory_id = $1 ORDER BY name"
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(businesses)))
}

pub async fn export_reviews(
    State(state): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let reviews = sqlx::query_as::<_, Review>(
        "SELECT r.* FROM reviews r JOIN businesses b ON r.business_id = b.id WHERE b.directory_id = $1 ORDER BY r.created_at"
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(reviews)))
}

pub async fn export_contacts(
    State(state): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
    struct CrmContactExport {
        id: Uuid,
        first_name: Option<String>,
        last_name: Option<String>,
        email: Option<String>,
        phone: Option<String>,
        company: Option<String>,
        directory_id: Option<Uuid>,
        created_at: Option<chrono::DateTime<chrono::Utc>>,
    }
    let contacts = sqlx::query_as::<_, CrmContactExport>(
        "SELECT id, first_name, last_name, email, phone, company, directory_id, created_at FROM crm_contacts WHERE directory_id = $1 ORDER BY first_name"
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(contacts)))
}

// ── Export Templates ──────────────────────────────────────────────────────

pub async fn list_export_templates(
    State(state): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let templates = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates ORDER BY name"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!(templates)))
}

pub async fn create_export_template(
    State(state): State<AppState>,
    Json(req): Json<CreateExportTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, ExportTemplate>(
        "INSERT INTO export_templates (name, entity_type, fields, directory_id, delimiter, include_header) VALUES ($1, $2, $3::jsonb, $4, $5, $6) RETURNING *"
    )
    .bind(&req.name)
    .bind(&req.entity_type)
    .bind(serde_json::to_value(&req.fields).map_err(|e| AppError::Internal(e.to_string()))?)
    .bind(req.directory_id)
    .bind(req.delimiter.unwrap_or_else(|| ",".to_string()))
    .bind(req.include_header.unwrap_or(true))
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(json!(template))))
}

pub async fn get_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Export template not found".to_string()))?;
    Ok(Json(json!(template)))
}

pub async fn update_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateExportTemplateRequest>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE export_templates SET name = $1, entity_type = $2, fields = $3::jsonb, directory_id = $4, delimiter = $5, include_header = $6 WHERE id = $7"
    )
    .bind(&req.name)
    .bind(&req.entity_type)
    .bind(serde_json::to_value(&req.fields).map_err(|e| AppError::Internal(e.to_string()))?)
    .bind(req.directory_id)
    .bind(req.delimiter.unwrap_or_else(|| ",".to_string()))
    .bind(req.include_header.unwrap_or(true))
    .bind(id)
    .execute(&state.db)
    .await?;
    let template = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!(template)))
}

pub async fn delete_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM export_templates WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Export template not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_export_template(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let template = sqlx::query_as::<_, ExportTemplate>(
        "SELECT * FROM export_templates WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Export template not found".to_string()))?;
    let data: Vec<serde_json::Value> = match template.entity_type.as_str() {
        "businesses" => {
            let rows = if let Some(did) = template.directory_id {
                sqlx::query("SELECT row_to_json(b.*)::text as j FROM businesses b WHERE b.directory_id = $1 ORDER BY b.name")
                    .bind(did).fetch_all(&state.db).await?
            } else {
                sqlx::query("SELECT row_to_json(b.*)::text as j FROM businesses b ORDER BY b.name")
                    .fetch_all(&state.db).await?
            };
            rows.iter().filter_map(|r| {
                let s: String = r.get("j");
                serde_json::from_str(&s).ok()
            }).collect()
        }
        "reviews" => {
            let rows = sqlx::query("SELECT row_to_json(r.*)::text as j FROM reviews r ORDER BY r.created_at")
                .fetch_all(&state.db).await?;
            rows.iter().filter_map(|r| {
                let s: String = r.get("j");
                serde_json::from_str(&s).ok()
            }).collect()
        }
        "contacts" => {
            let rows = if let Some(did) = template.directory_id {
                sqlx::query("SELECT row_to_json(c.*)::text as j FROM crm_contacts c WHERE c.directory_id = $1 ORDER BY c.first_name")
                    .bind(did).fetch_all(&state.db).await?
            } else {
                sqlx::query("SELECT row_to_json(c.*)::text as j FROM crm_contacts c ORDER BY c.first_name")
                    .fetch_all(&state.db).await?
            };
            rows.iter().filter_map(|r| {
                let s: String = r.get("j");
                serde_json::from_str(&s).ok()
            }).collect()
        }
        _ => return Err(AppError::Validation(format!("Unsupported entity type: {}", template.entity_type))),
    };
    Ok(Json(json!({
        "template": template,
        "rows": data.len(),
        "data": data,
    })))
}
