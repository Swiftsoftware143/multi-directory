//! B2B Marketplace handlers — BL23
//! Suppliers list products. Businesses search, browse, and connect.
//! Distinct from regular business listings with supplier-specific features.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

#[derive(Debug, Deserialize)]
pub struct ProductQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    pub business_id: Option<Uuid>,
    pub delivery_area: Option<String>,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductRequest {
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub price: Option<f64>,
    pub unit: Option<String>,
    pub min_order: Option<i32>,
    pub delivery_areas: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub price: Option<f64>,
    pub unit: Option<String>,
    pub min_order: Option<i32>,
    pub delivery_areas: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SupplierProduct {
    pub id: Uuid,
    pub business_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub price: Option<rust_decimal::Decimal>,
    pub unit: Option<String>,
    pub min_order: Option<i32>,
    pub delivery_areas: Option<Vec<String>>,
    pub is_active: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// POST /api/v1/b2b/products — supplier adds a product
pub async fn create_product(
    State(s): State<AppState>,
    Json(req): Json<CreateProductRequest>,
) -> ApiResult<impl IntoResponse> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO supplier_products (id, business_id, name, description, category, price, unit, min_order, delivery_areas)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
    )
    .bind(id)
    .bind(req.name)
    .bind(req.description)
    .bind(req.category)
    .bind(req.price)
    .bind(req.unit)
    .bind(req.min_order)
    .bind(&req.delivery_areas)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"id": id, "status": "created"})))
}

/// GET /api/v1/b2b/products — search products (supplier marketplace search)
pub async fn search_products(
    State(s): State<AppState>,
    Query(qs): Query<ProductQuery>,
) -> ApiResult<impl IntoResponse> {
    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let mut wheres = vec!["sp.is_active = true".to_string()];
    let mut param_count = 0i32;
    let mut next_param = || { param_count += 1; param_count };

    if let Some(ref q) = qs.q { if !q.is_empty() { let p = next_param(); wheres.push(format!("(sp.name ILIKE '%' || ${p} || '%' OR COALESCE(sp.description,'') ILIKE '%' || ${p} || '%')", p = p)); } }
    if let Some(ref cat) = qs.category { if !cat.is_empty() { let p = next_param(); wheres.push(format!("sp.category = ${p}", p = p)); } }
    if let Some(bid) = qs.business_id { let p = next_param(); wheres.push(format!("sp.business_id = ${p}", p = p)); }
    if let Some(ref area) = qs.delivery_area { if !area.is_empty() { let p = next_param(); wheres.push(format!("${p} = ANY(sp.delivery_areas)", p = p)); } }
    if let Some(mp) = qs.max_price { let p = next_param(); wheres.push(format!("COALESCE(sp.price, 0) <= ${p}", p = p)); }

    let where_clause = if wheres.is_empty() { String::new() } else { format!("WHERE {}", wheres.join(" AND ")) };

    let count_sql = format!("SELECT COUNT(*) FROM supplier_products sp {}", where_clause);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(ref q) = qs.q { if !q.is_empty() { count_q = count_q.bind(q); } }
    if let Some(ref cat) = qs.category { if !cat.is_empty() { count_q = count_q.bind(cat); } }
    if let Some(bid) = qs.business_id { count_q = count_q.bind(bid); }
    if let Some(ref area) = qs.delivery_area { if !area.is_empty() { count_q = count_q.bind(area); } }
    if let Some(mp) = qs.max_price { count_q = count_q.bind(mp); }

    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    let data_sql = format!(
        "SELECT sp.*, b.name as business_name, b.city, b.state \
         FROM supplier_products sp \
         LEFT JOIN businesses b ON b.id = sp.business_id \
         {} ORDER BY sp.name ASC LIMIT ${} OFFSET ${}",
        where_clause, next_param(), next_param()
    );

    let mut data_q = sqlx::query_as::<_, (Uuid, Uuid, String, Option<String>, Option<String>, Option<rust_decimal::Decimal>, Option<String>, Option<i32>, Option<Vec<String>>, Option<bool>, Option<chrono::DateTime<chrono::Utc>>, String, Option<String>, Option<String>)>(&data_sql);
    if let Some(ref q) = qs.q { if !q.is_empty() { data_q = data_q.bind(q); } }
    if let Some(ref cat) = qs.category { if !cat.is_empty() { data_q = data_q.bind(cat); } }
    if let Some(bid) = qs.business_id { data_q = data_q.bind(bid); }
    if let Some(ref area) = qs.delivery_area { if !area.is_empty() { data_q = data_q.bind(area); } }
    if let Some(mp) = qs.max_price { data_q = data_q.bind(mp); }
    data_q = data_q.bind(per_page);
    data_q = data_q.bind(offset);

    let rows = data_q.fetch_all(&s.db).await?;
    let results: Vec<serde_json::Value> = rows.into_iter().map(|r| json!({
        "id": r.0, "business_id": r.1, "name": r.2, "description": r.3,
        "category": r.4, "price": r.5, "unit": r.6, "min_order": r.7,
        "delivery_areas": r.8, "is_active": r.9, "created_at": r.10,
        "business_name": r.11, "city": r.12, "state": r.13
    })).collect();

    Ok(Json(json!({"products": results, "total": total, "page": page, "per_page": per_page})))
}

/// GET /api/v1/b2b/products/:id — get single product
pub async fn get_product(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let product = sqlx::query_as::<_, SupplierProduct>(
        "SELECT sp.*, b.name as business_name FROM supplier_products sp \
         LEFT JOIN businesses b ON b.id = sp.business_id WHERE sp.id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Product not found".into()))?;

    Ok(Json(json!(product)))
}

/// PUT /api/v1/b2b/products/:id — update product
pub async fn update_product(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProductRequest>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE supplier_products SET name=COALESCE($1,name), description=COALESCE($2,description), \
         category=COALESCE($3,category), price=COALESCE($4,price), unit=COALESCE($5,unit), \
         min_order=COALESCE($6,min_order), delivery_areas=COALESCE($7,delivery_areas), \
         is_active=COALESCE($8,is_active), updated_at=NOW() WHERE id=$9"
    )
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.category)
    .bind(req.price)
    .bind(&req.unit)
    .bind(req.min_order)
    .bind(&req.delivery_areas)
    .bind(req.is_active)
    .bind(id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "updated"})))
}

/// DELETE /api/v1/b2b/products/:id — delete product
pub async fn delete_product(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query("DELETE FROM supplier_products WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({"status": "deleted"})))
}

/// GET /api/v1/b2b/suppliers — list all supplier-type businesses
pub async fn list_suppliers(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let suppliers = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, name, city, state, phone, website FROM businesses \
         WHERE business_type IN ('supplier','distributor','wholesaler','farm','association') AND COALESCE(is_active, true) = true \
         ORDER BY name ASC"
    )
    .fetch_all(&s.db)
    .await?;

    let result: Vec<serde_json::Value> = suppliers.into_iter().map(|s| json!({
        "id": s.0, "name": s.1, "city": s.2, "state": s.3, "phone": s.4, "website": s.5
    })).collect();

    Ok(Json(json!({"suppliers": result, "total": result.len()})))
}
