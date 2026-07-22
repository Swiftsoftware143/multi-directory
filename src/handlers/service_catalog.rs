//! Business Service Catalog — stage 5 booking services.
//!
//! Each business can list services/products that visitors can browse and book.
//! These are the per-business services (not the directory-level service_prices).

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::AppState;
use crate::auth::models::Claims;
use crate::auth::middleware::{is_admin, is_business_owner};
use crate::error::{AppError, ApiResult};

#[derive(Debug, Deserialize)]
pub struct ServicesQuery {
    pub business_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub category: Option<String>,
    pub active_only: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct BusinessServiceRow {
    pub id: Uuid,
    pub business_id: Uuid,
    pub directory_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub price: Option<rust_decimal::Decimal>,
    pub currency: String,
    pub duration_minutes: Option<i32>,
    pub category: Option<String>,
    pub is_active: bool,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub business_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub price: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub duration_minutes: Option<i32>,
    pub category: Option<String>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServiceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<rust_decimal::Decimal>,
    pub currency: Option<String>,
    pub duration_minutes: Option<i32>,
    pub category: Option<String>,
    pub is_active: Option<bool>,
    pub sort_order: Option<i32>,
}

/// GET /api/v1/services — list services for a business
///
/// Query params:
/// - business_id (optional): filter by business
/// - directory_id (optional): filter by directory
/// - category (optional): filter by category
/// - active_only (optional, default true): only active services
pub async fn list_services(
    State(s): State<AppState>,
    Query(query): Query<ServicesQuery>,
) -> ApiResult<impl IntoResponse> {
    // Build query dynamically
    let mut conditions = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if let Some(biz_id) = query.business_id {
        conditions.push(format!("business_id = ${}", param_idx));
        params.push(biz_id.to_string());
        param_idx += 1;
    }
    if let Some(dir_id) = query.directory_id {
        conditions.push(format!("directory_id = ${}", param_idx));
        params.push(dir_id.to_string());
        param_idx += 1;
    }
    if let Some(ref cat) = query.category {
        conditions.push(format!("category = ${}", param_idx));
        params.push(cat.clone());
        param_idx += 1;
    }
    let active_only = query.active_only.unwrap_or(true);
    if active_only {
        conditions.push(format!("is_active = true"));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT id, business_id, directory_id, name, description, price, currency, \
         duration_minutes, category, is_active, sort_order, created_at, updated_at \
         FROM business_services {} ORDER BY sort_order, name",
        where_clause
    );

    let mut query_builder = sqlx::query_as::<_, BusinessServiceRow>(&sql);
    for p in &params {
        query_builder = query_builder.bind(p.clone());
    }

    let services = query_builder.fetch_all(&s.db).await?;

    Ok(Json(json!({
        "success": true,
        "services": services,
    })))
}

/// GET /api/v1/services/:id — get a single service
pub async fn get_service(
    State(s): State<AppState>,
    Path(service_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let service = sqlx::query_as::<_, BusinessServiceRow>(
        "SELECT id, business_id, directory_id, name, description, price, currency, \
         duration_minutes, category, is_active, sort_order, created_at, updated_at \
         FROM business_services WHERE id = $1"
    )
    .bind(service_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Service not found".to_string()))?;

    Ok(Json(json!({ "success": true, "service": service })))
}

/// POST /api/v1/services — create a new service for a business
///
/// Requires business_owner or admin role.
pub async fn create_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateServiceRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.trim().is_empty() {
        return Err(AppError::Validation("name is required".to_string()));
    }

    // Verify authorization
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    let is_authorized = if is_admin(&claims) {
        true
    } else if is_business_owner(&claims) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM claimed_businesses WHERE business_id = $1 AND user_id = $2 AND is_active = true"
        )
        .bind(req.business_id)
        .bind(user_id)
        .fetch_one(&s.db)
        .await?;
        count > 0
    } else {
        false
    };

    if !is_authorized {
        return Err(AppError::Forbidden("Not authorized to manage this business's services".to_string()));
    }

    // Get directory_id from business
    let directory_id: Uuid = sqlx::query_scalar(
        "SELECT directory_id FROM businesses WHERE id = $1"
    )
    .bind(req.business_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Business not found".to_string()))?;

    let row = sqlx::query_as::<_, BusinessServiceRow>(
        r#"INSERT INTO business_services (business_id, directory_id, name, description, price, currency, duration_minutes, category, sort_order)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id, business_id, directory_id, name, description, price, currency,
                     duration_minutes, category, is_active, sort_order, created_at, updated_at"#
    )
    .bind(req.business_id)
    .bind(directory_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price)
    .bind(req.currency.unwrap_or_else(|| "USD".to_string()))
    .bind(req.duration_minutes)
    .bind(&req.category)
    .bind(req.sort_order.unwrap_or(0))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!({ "success": true, "service": row }))))
}

/// PUT /api/v1/services/:id — update a service
pub async fn update_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(service_id): Path<Uuid>,
    Json(req): Json<UpdateServiceRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Get business_id for verification
    let biz_id: Option<(Uuid,)> = sqlx::query_as(
        "SELECT business_id FROM business_services WHERE id = $1"
    )
    .bind(service_id)
    .fetch_optional(&s.db)
    .await?;

    let (business_id,) = biz_id.ok_or(AppError::NotFound("Service not found".to_string()))?;

    let is_authorized = if is_admin(&claims) {
        true
    } else if is_business_owner(&claims) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM claimed_businesses WHERE business_id = $1 AND user_id = $2 AND is_active = true"
        )
        .bind(business_id)
        .bind(user_id)
        .fetch_one(&s.db)
        .await?;
        count > 0
    } else {
        false
    };

    if !is_authorized {
        return Err(AppError::Forbidden("Not authorized to update this service".to_string()));
    }

    sqlx::query(
        r#"UPDATE business_services
           SET name = COALESCE($1, name),
               description = COALESCE($2, description),
               price = COALESCE($3, price),
               currency = COALESCE($4, currency),
               duration_minutes = COALESCE($5, duration_minutes),
               category = COALESCE($6, category),
               is_active = COALESCE($7, is_active),
               sort_order = COALESCE($8, sort_order)
           WHERE id = $9"#
    )
    .bind(&req.name)
    .bind(&req.description)
    .bind(req.price)
    .bind(&req.currency)
    .bind(req.duration_minutes)
    .bind(&req.category)
    .bind(req.is_active)
    .bind(req.sort_order)
    .bind(service_id)
    .execute(&s.db)
    .await?;

    let updated = sqlx::query_as::<_, BusinessServiceRow>(
        "SELECT id, business_id, directory_id, name, description, price, currency, \
         duration_minutes, category, is_active, sort_order, created_at, updated_at \
         FROM business_services WHERE id = $1"
    )
    .bind(service_id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({ "success": true, "service": updated })))
}

/// DELETE /api/v1/services/:id — soft-delete (set inactive) a service
pub async fn delete_service(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(service_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    let biz_id: Option<(Uuid,)> = sqlx::query_as(
        "SELECT business_id FROM business_services WHERE id = $1"
    )
    .bind(service_id)
    .fetch_optional(&s.db)
    .await?;

    let (business_id,) = biz_id.ok_or(AppError::NotFound("Service not found".to_string()))?;

    let is_authorized = if is_admin(&claims) {
        true
    } else if is_business_owner(&claims) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM claimed_businesses WHERE business_id = $1 AND user_id = $2 AND is_active = true"
        )
        .bind(business_id)
        .bind(user_id)
        .fetch_one(&s.db)
        .await?;
        count > 0
    } else {
        false
    };

    if !is_authorized {
        return Err(AppError::Forbidden("Not authorized to delete this service".to_string()));
    }

    sqlx::query("UPDATE business_services SET is_active = false WHERE id = $1")
        .bind(service_id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({ "success": true, "message": "Service deleted" })))
}

/// GET /api/v1/businesses/:business_id/services — list services for a business
///
/// Public endpoint (no auth required). Returns only active services.
pub async fn list_services_for_business(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let services = sqlx::query_as::<_, BusinessServiceRow>(
        r#"SELECT id, business_id, directory_id, name, description, price, currency,
                  duration_minutes, category, is_active, sort_order, created_at, updated_at
           FROM business_services
           WHERE business_id = $1 AND is_active = true
           ORDER BY sort_order, name"#
    )
    .bind(business_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "success": true,
        "business_id": business_id,
        "services": services,
    })))
}
