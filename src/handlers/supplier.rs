//! Supplier Portal handlers — back office for distributors/wholesalers/farms/associations
//! Separate from the business owner portal. Manages products, delivery zones, orders.

use axum::{extract::State, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};

#[derive(Debug, Deserialize)]
pub struct UpdateSupplierProfileRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDeliverySettingsRequest {
    pub delivery_areas: Option<Vec<String>>,
    pub min_order: Option<f64>,
}

/// GET /api/v1/supplier/profile — get the authenticated supplier's business profile
pub async fn get_supplier_profile(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    // Get the business associated with the authenticated user
    let user_id = get_user_id_from_jwt(); // This would come from middleware — for now return the business context

    // For now, return a basic profile shape
    // The actual implementation will get the business linked to the user's claimed_businesses
    let profile = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
        r#"SELECT b.id, b.name, b.email, b.phone, b.website, b.description
           FROM businesses b
           WHERE b.business_type IN ('supplier','distributor','wholesaler','farm','association')
           LIMIT 1"#
    )
    .fetch_optional(&s.db)
    .await?;

    match profile {
        Some((id, name, email, phone, website, desc)) => {
            Ok(Json(json!({
                "business_id": id,
                "name": name,
                "email": email,
                "phone": phone,
                "website": website,
                "description": desc,
                "delivery_areas": [],
                "min_order": 0
            })))
        }
        None => Err(AppError::NotFound("No supplier profile found".into()))
    }
}

/// PUT /api/v1/supplier/profile — update supplier business profile
pub async fn update_supplier_profile(
    State(s): State<AppState>,
    Json(req): Json<UpdateSupplierProfileRequest>,
) -> ApiResult<impl IntoResponse> {
    sqlx::query(
        "UPDATE businesses SET name=COALESCE($1,name), email=COALESCE($2,email), \
         phone=COALESCE($3,phone), website=COALESCE($4,website), description=COALESCE($5,description), \
         updated_at=NOW() WHERE business_type IN ('supplier','distributor','wholesaler','farm','association')"
    )
    .bind(&req.name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.website)
    .bind(&req.description)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "updated"})))
}

/// PUT /api/v1/supplier/delivery — update delivery zones and min order
pub async fn update_delivery_settings(
    State(s): State<AppState>,
    Json(req): Json<UpdateDeliverySettingsRequest>,
) -> ApiResult<impl IntoResponse> {
    let delivery_areas_json = req.delivery_areas.map(|v| serde_json::to_value(v).unwrap_or_default());

    sqlx::query(
        "UPDATE businesses SET supplier_fields = jsonb_set(COALESCE(supplier_fields,'{}'::jsonb), '{delivery_areas}', $1, true), \
         updated_at=NOW() WHERE business_type IN ('supplier','distributor','wholesaler','farm','association')"
    )
    .bind(&delivery_areas_json)
    .execute(&s.db)
    .await?;

    // Also update min_order in supplier_fields
    if let Some(mo) = req.min_order {
        let mo_json = serde_json::json!(mo);
        sqlx::query(
            "UPDATE businesses SET supplier_fields = jsonb_set(COALESCE(supplier_fields,'{}'::jsonb), '{min_order}', $1, true), \
             updated_at=NOW() WHERE business_type IN ('supplier','distributor','wholesaler','farm','association')"
        )
        .bind(&mo_json)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({"status": "updated"})))
}

fn get_user_id_from_jwt() -> Option<Uuid> {
    // Placeholder — real implementation extracts from the JWT claims set by auth middleware
    None
}
