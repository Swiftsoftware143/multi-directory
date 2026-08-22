//! Supplier Portal handlers — back office for distributors/wholesalers/farms/associations
//! Separate from the business owner portal. Manages products, delivery zones, orders.
//!
//! All handlers require auth (JWT via auth_guard middleware). Each handler resolves
//! the supplier's claimed businesses and scopes operations to their own records.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::{middleware::verify_token, models::Claims};
use crate::error::{ApiResult, AppError};
use crate::handlers::b2b::{get_b2b_config, get_first_directory_id, require_b2b_feature};
use crate::utils;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct UpdateSupplierProfileRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
    /// Standardized CTA type — one of the 13 predefined values, or None.
    pub cta_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDeliverySettingsRequest {
    pub delivery_areas: Option<Vec<String>>,
    pub min_order: Option<f64>,
}

/// GET /api/v1/supplier/profile — get the authenticated supplier's business profile
pub async fn get_supplier_profile(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;

    // Resolve the supplier business this user has claimed
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let profile = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"SELECT b.id, b.name, b.email, b.phone, b.website, b.description
           FROM businesses b
           WHERE b.id = $1
             AND b.business_type IN ('supplier','distributor','wholesaler','farm','association')
           LIMIT 1"#,
    )
    .bind(biz_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("No supplier profile found for your account".into()))?;

    let (id, name, email, phone, website, desc) = profile;

    // Fetch delivery_areas and min_order from supplier_fields jsonb
    let supplier_fields: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT supplier_fields FROM businesses WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .flatten();

    let delivery_areas = supplier_fields
        .as_ref()
        .and_then(|v| v.get("delivery_areas"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let min_order = supplier_fields
        .as_ref()
        .and_then(|v| v.get("min_order"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    Ok(Json(json!({
        "business_id": id,
        "name": name,
        "email": email,
        "phone": phone,
        "website": website,
        "description": desc,
        "delivery_areas": delivery_areas,
        "min_order": min_order
    })))
}

/// PUT /api/v1/supplier/profile — update supplier business profile
pub async fn update_supplier_profile(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateSupplierProfileRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    // Validate cta_type if provided
    if let Some(ref cta) = req.cta_type {
        if !utils::is_valid_cta_type(cta) {
            return Err(AppError::BadRequest(format!(
                "Invalid CTA type '{}'. Must be one of: {}",
                cta,
                utils::VALID_CTA_TYPES.join(", ")
            )));
        }
    }

    sqlx::query(
        "UPDATE businesses SET name=COALESCE($1,name), email=COALESCE($2,email), \
         phone=COALESCE($3,phone), website=COALESCE($4,website), description=COALESCE($5,description), \
         updated_at=NOW() WHERE id = $6 \
         AND business_type IN ('supplier','distributor','wholesaler','farm','association')"
    )
    .bind(&req.name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.website)
    .bind(&req.description)
    .bind(biz_id)
    .execute(&s.db)
    .await?;

    // Upsert cta_type into business_meta.meta_data
    if req.cta_type.is_some() {
        let cta_value = req.cta_type.as_deref().unwrap_or("none");
        let meta_patch = serde_json::json!({"cta_type": cta_value});
        sqlx::query(
            r#"INSERT INTO business_meta (business_id, template, meta_data)
               VALUES ($1, $2, $3::jsonb)
               ON CONFLICT (business_id, template)
               DO UPDATE SET meta_data = business_meta.meta_data || $3::jsonb,
                             updated_at = NOW()"#,
        )
        .bind(biz_id)
        .bind(crate::template_engine::TEMPLATE_BUSINESS_DETAIL)
        .bind(&meta_patch)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({"status": "updated"})))
}

/// PUT /api/v1/supplier/delivery — update delivery zones and min order
pub async fn update_delivery_settings(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateDeliverySettingsRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    if let Some(delivery_areas) = &req.delivery_areas {
        let delivery_areas_json = serde_json::to_value(delivery_areas).unwrap_or_default();
        sqlx::query(
            "UPDATE businesses SET supplier_fields = jsonb_set(COALESCE(supplier_fields,'{}'::jsonb), '{delivery_areas}', $1, true), \
             updated_at=NOW() WHERE id = $2 \
             AND business_type IN ('supplier','distributor','wholesaler','farm','association')"
        )
        .bind(&delivery_areas_json)
        .bind(biz_id)
        .execute(&s.db)
        .await?;
    }

    if let Some(mo) = req.min_order {
        let mo_json = serde_json::json!(mo);
        sqlx::query(
            "UPDATE businesses SET supplier_fields = jsonb_set(COALESCE(supplier_fields,'{}'::jsonb), '{min_order}', $1, true), \
             updated_at=NOW() WHERE id = $2 \
             AND business_type IN ('supplier','distributor','wholesaler','farm','association')"
        )
        .bind(&mo_json)
        .bind(biz_id)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({"status": "updated"})))
}

/// PUT /api/v1/supplier/featured-product — set featured product for a supplier
#[derive(Debug, Deserialize)]
pub struct FeaturedProductRequest {
    pub product_id: Option<Uuid>,
    pub cta_text: Option<String>,
}

pub async fn set_featured_product(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<FeaturedProductRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    sqlx::query(
        "UPDATE businesses SET featured_product_id = $1, \
         featured_product_cta = COALESCE($2, featured_product_cta, 'Featured Product'), \
         updated_at = NOW() WHERE id = $3 \
         AND business_type IN ('supplier','distributor','wholesaler','farm','association')",
    )
    .bind(req.product_id)
    .bind(&req.cta_text)
    .bind(biz_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "updated"})))
}

/// GET /api/v1/supplier/stats — supplier order analytics
pub async fn supplier_stats(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let row = sqlx::query_as::<
        _,
        (
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            rust_decimal::Decimal,
            rust_decimal::Decimal,
        ),
    >(
        "SELECT total_orders, pending_orders, confirmed_orders, shipped_orders, \
                delivered_orders, cancelled_orders, total_revenue, avg_rating \
         FROM supplier_order_stats WHERE supplier_business_id = $1",
    )
    .bind(biz_id)
    .fetch_optional(&s.db)
    .await?;

    if let Some((total, pending, confirmed, shipped, delivered, cancelled, revenue, avg)) = row {
        Ok(Json(json!({
            "business_id": biz_id,
            "total_orders": total,
            "pending_orders": pending,
            "confirmed_orders": confirmed,
            "shipped_orders": shipped,
            "delivered_orders": delivered,
            "cancelled_orders": cancelled,
            "total_revenue": revenue,
            "avg_rating": avg
        })))
    } else {
        // No orders yet — return zeros
        Ok(Json(json!({
            "business_id": biz_id,
            "total_orders": 0,
            "pending_orders": 0,
            "confirmed_orders": 0,
            "shipped_orders": 0,
            "delivered_orders": 0,
            "cancelled_orders": 0,
            "total_revenue": 0.0,
            "avg_rating": 0.0
        })))
    }
}

#[derive(Debug, Deserialize)]
pub struct FulfillOrderRequest {
    pub tracking_number: Option<String>,
    pub carrier: Option<String>,
    pub estimated_delivery: Option<String>,
}

/// PUT /api/v1/b2b/orders/:id/fulfill — add tracking info to an order
pub async fn fulfill_order(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<FulfillOrderRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    // Verify the order belongs to this supplier's business
    let order = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT supplier_business_id, status FROM b2b_orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Order not found".into()))?;

    if order.0 != biz_id {
        return Err(AppError::Forbidden(
            "Only the supplier can fulfill this order".into(),
        ));
    }

    // Parse estimated_delivery as Option<chrono::NaiveDate> if provided
    let est_delivery = req
        .estimated_delivery
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());

    sqlx::query(
        "UPDATE b2b_orders \
         SET tracking_number = COALESCE($1, tracking_number), \
             carrier = COALESCE($2, carrier), \
             estimated_delivery = COALESCE($3, estimated_delivery), \
             updated_at = NOW() \
         WHERE id = $4",
    )
    .bind(&req.tracking_number)
    .bind(&req.carrier)
    .bind(est_delivery)
    .bind(order_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "status": "fulfilled",
        "order_id": order_id,
        "tracking_number": req.tracking_number,
        "carrier": req.carrier,
        "estimated_delivery": req.estimated_delivery
    })))
}

#[derive(Debug, Deserialize)]
pub struct ReviewBuyerRequest {
    pub rating: i32,
    pub review: Option<String>,
}

/// POST /api/v1/b2b/orders/:id/review — supplier reviews the buyer
pub async fn supplier_review_buyer(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(order_id): Path<Uuid>,
    Json(req): Json<ReviewBuyerRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id_for_supplier(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    // Validate rating range
    if req.rating < 1 || req.rating > 5 {
        return Err(AppError::Validation(
            "Rating must be between 1 and 5".into(),
        ));
    }

    // Verify the order belongs to this supplier and is delivered
    let order = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT supplier_business_id, status FROM b2b_orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Order not found".into()))?;

    if order.0 != biz_id {
        return Err(AppError::Forbidden(
            "Only the supplier can review this order".into(),
        ));
    }

    if order.1 != "delivered" {
        return Err(AppError::Validation(
            "Can only review delivered orders".into(),
        ));
    }

    sqlx::query(
        "UPDATE b2b_orders \
         SET supplier_rating = $1, \
             supplier_review = $2, \
             updated_at = NOW() \
         WHERE id = $3",
    )
    .bind(req.rating)
    .bind(&req.review)
    .bind(order_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "status": "reviewed",
        "order_id": order_id,
        "rating": req.rating
    })))
}

// Extract user_id from Authorization header (used by handlers outside auth_guard)
fn extract_user_id_for_supplier(headers: &HeaderMap, state: &AppState) -> ApiResult<Uuid> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;
    let claims =
        verify_token(token, &state.config.jwt_secret).map_err(|_| AppError::Unauthorized)?;
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

// Re-use the shared resolve_supplier_business from b2b module
use crate::handlers::b2b::resolve_supplier_business;
