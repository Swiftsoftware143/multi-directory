//! Industry Dashboard Handlers
//! Manages user industry dashboard selections, synced with template_categories.

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct UserIndustryDashboard {
    pub id: Uuid,
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub industry_slug: String,
    pub dashboard_name: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SetIndustryRequest {
    pub industry_slug: String,
    pub dashboard_name: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IndustryOption {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<i32>,
}

/// GET /api/v1/industries/available
/// The industries/verticals a directory can be built on. Source of truth is this app's own
/// `template_categories` table (migration 108) — a real, admin-editable table. The old code
/// SELECTed a `template_categories` that only existed in the workflowswift database, so the
/// query always failed and a hardcoded list was served; errors now propagate instead.
pub async fn list_available_industries(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let industries = sqlx::query_as::<_, IndustryOption>(
        "SELECT slug, name, description, icon, sort_order::int FROM template_categories \
         WHERE is_active = true ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(industries)))
}

/// GET /api/v1/admin/industries
/// Lists the user's active industry dashboards
pub async fn list_user_industries(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let dashboards = sqlx::query_as::<_, UserIndustryDashboard>(
        "SELECT * FROM user_industry_dashboards WHERE user_id = $1 AND tenant_id = $2 ORDER BY created_at ASC"
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(dashboards)))
}

/// POST /api/v1/admin/industries
/// Sets/activates an industry dashboard for the current user
pub async fn set_user_industry(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<SetIndustryRequest>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    if req.industry_slug.is_empty() {
        return Err(AppError::Validation(
            "industry_slug is required".to_string(),
        ));
    }

    // Count current industries to check plan limit
    let current_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_industry_dashboards WHERE user_id = $1 AND is_active = true",
    )
    .bind(user_id)
    .fetch_one(&s.db)
    .await?;

    // Get plan limit (from plan_tiers.max_industries via the user's subscription)
    let max_industries: i32 = {
        // Default to 1 if we can't determine
        let mut limit = 1;

        // Try to get the user's plan tier via business_subscriptions
        // For now, pull from plan_tiers default
        if let Ok(Some((max_val,))) = sqlx::query_as::<_, (Option<i32>,)>(
            "SELECT pt.max_industries FROM plan_tiers pt
             INNER JOIN business_subscriptions bs ON bs.tier_id = pt.id
             INNER JOIN businesses b ON b.id = bs.business_id
             WHERE bs.status = 'active' AND b.owner_id IS NOT NULL
             LIMIT 1",
        )
        .fetch_optional(&s.db)
        .await
        {
            if let Some(val) = max_val {
                limit = val;
            }
        }

        limit
    };

    // Check if we're adding a new one (upsert flow)
    let existing = sqlx::query_as::<_, UserIndustryDashboard>(
        "SELECT * FROM user_industry_dashboards WHERE user_id = $1 AND industry_slug = $2",
    )
    .bind(user_id)
    .bind(&req.industry_slug)
    .fetch_optional(&s.db)
    .await?;

    if existing.is_none() && current_count.0 >= max_industries as i64 && max_industries >= 0 {
        return Err(AppError::Validation(format!(
            "Industry dashboard limit reached ({}/{})",
            current_count.0, max_industries
        )));
    }

    let dashboard_name = req
        .dashboard_name
        .unwrap_or_else(|| format!("{} Dashboard", req.industry_slug.replace('-', " ")));

    // Upsert: insert or activate
    let dashboard = if let Some(existing) = existing {
        sqlx::query_as::<_, UserIndustryDashboard>(
            "UPDATE user_industry_dashboards SET is_active = true, dashboard_name = $1, updated_at = NOW() WHERE id = $2 RETURNING *"
        )
        .bind(&dashboard_name)
        .bind(existing.id)
        .fetch_one(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, UserIndustryDashboard>(
            "INSERT INTO user_industry_dashboards (user_id, tenant_id, industry_slug, dashboard_name) VALUES ($1, $2, $3, $4) RETURNING *"
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(&req.industry_slug)
        .bind(&dashboard_name)
        .fetch_one(&s.db)
        .await?
    };

    // Also update the tenant's default industry
    sqlx::query("UPDATE tenants SET industry_slug = $1 WHERE id = $2")
        .bind(&req.industry_slug)
        .bind(tenant_id)
        .execute(&s.db)
        .await?;

    Ok((StatusCode::CREATED, Json(json!(dashboard))))
}

/// DELETE /api/v1/admin/industries/:slug
/// Deactivates an industry dashboard
pub async fn remove_user_industry(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(slug): axum::extract::Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let result = sqlx::query("UPDATE user_industry_dashboards SET is_active = false, updated_at = NOW() WHERE user_id = $1 AND industry_slug = $2")
        .bind(user_id)
        .bind(&slug)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(
            "Industry dashboard not found".to_string(),
        ));
    }

    Ok(Json(json!({"message": "Industry dashboard deactivated"})))
}

/// GET /api/v1/admin/industries/limit
/// Returns the user's plan industry limit and current usage
pub async fn get_industry_limit(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let current_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_industry_dashboards WHERE user_id = $1 AND is_active = true",
    )
    .bind(user_id)
    .fetch_one(&s.db)
    .await?;

    let max_industries: i32 = sqlx::query_scalar(
        "SELECT COALESCE(pt.max_industries, 1) FROM plan_tiers pt
         INNER JOIN business_subscriptions bs ON bs.tier_id = pt.id
         INNER JOIN businesses b ON b.id = bs.business_id
         WHERE bs.status = 'active' AND (b.owner_id = $1 OR b.id IN (
            SELECT business_id FROM business_subscriptions WHERE status = 'active'
         ))
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or(1);

    Ok(Json(json!({
        "current": current_count.0,
        "max": max_industries,
        "remaining": if max_industries < 0 { -1 } else { max_industries as i64 - current_count.0 }
    })))
}
