//! Industry Dashboard Handlers
//! Manages user industry dashboard selections, synced with template_categories.

use axum::{
    extract::{State, Extension},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::auth::models::Claims;

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

/// GET /api/v1/admin/industries/available
/// Lists industries available (from workflowswift template_categories via hardcoded sync,
/// or from a dedicated industries table). For now we return the canonical list.
/// This syncs with template_categories in workflowswift DB.
pub async fn list_available_industries(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    // Try to fetch from workflowswift DB first (cross-database query)
    let industries = sqlx::query_as::<_, IndustryOption>(
        "SELECT slug, name, description, icon, sort_order::int FROM template_categories WHERE is_active = true ORDER BY sort_order ASC"
    )
    .fetch_all(&s.db)
    .await
    .unwrap_or_else(|_| {
        // Fallback: return hardcoded list matching template_categories
        vec![
            IndustryOption { slug: "sales-lead-gen".into(), name: "Sales & Lead Generation".into(), description: Some("Lead capture, nurturing, and sales pipeline automation".into()), icon: Some("💼".into()), sort_order: Some(0) },
            IndustryOption { slug: "service-businesses".into(), name: "Service Businesses".into(), description: Some("Estimate, schedule, invoice workflows".into()), icon: Some("🔧".into()), sort_order: Some(1) },
            IndustryOption { slug: "recruitment-staffing".into(), name: "Recruitment & Staffing".into(), description: Some("Resume screening, interview coordination, placements".into()), icon: Some("👥".into()), sort_order: Some(2) },
            IndustryOption { slug: "marketing-agencies".into(), name: "Marketing Agencies".into(), description: Some("Content calendars, ad campaigns, reporting".into()), icon: Some("📣".into()), sort_order: Some(3) },
            IndustryOption { slug: "professional-services".into(), name: "Professional Services".into(), description: Some("Tax, legal, consulting workflows".into()), icon: Some("⚖️".into()), sort_order: Some(4) },
            IndustryOption { slug: "ecommerce-retail".into(), name: "Ecommerce & Retail".into(), description: Some("Order fulfillment, inventory, dropshipping".into()), icon: Some("🛒".into()), sort_order: Some(5) },
            IndustryOption { slug: "healthcare-wellness".into(), name: "Healthcare & Wellness".into(), description: Some("Patient intake, appointments, treatment planning".into()), icon: Some("🏥".into()), sort_order: Some(6) },
            IndustryOption { slug: "construction-development".into(), name: "Construction & Development".into(), description: Some("Permit management, subcontractor bidding, development".into()), icon: Some("🏗️".into()), sort_order: Some(7) },
            IndustryOption { slug: "grant-funding".into(), name: "Grant & Funding".into(), description: Some("Grant writing, research, submission tracking".into()), icon: Some("💰".into()), sort_order: Some(8) },
            IndustryOption { slug: "education-training".into(), name: "Education & Training".into(), description: Some("Course creation, enrollment, certificates".into()), icon: Some("📚".into()), sort_order: Some(9) },
            IndustryOption { slug: "publishing-media".into(), name: "Publishing & Media".into(), description: Some("Content approval, newsletters, editorial calendars".into()), icon: Some("📰".into()), sort_order: Some(10) },
            IndustryOption { slug: "site-flipping".into(), name: "Site Flipping".into(), description: Some("Website flipping, marketplace listings, TinyBrander funnel".into()), icon: Some("🔄".into()), sort_order: Some(11) },
            IndustryOption { slug: "government-contracting".into(), name: "Government Contracting".into(), description: Some("Opportunity discovery, bidding, contract management".into()), icon: Some("🏛️".into()), sort_order: Some(12) },
            IndustryOption { slug: "content-creation".into(), name: "Content Creation".into(), description: Some("AI video, images, voiceover workflows".into()), icon: Some("🎬".into()), sort_order: Some(13) },
            IndustryOption { slug: "newsletter".into(), name: "Newsletter".into(), description: Some("Email newsletter creation and management".into()), icon: Some("📧".into()), sort_order: Some(14) },
        ]
    });

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
        return Err(AppError::Validation("industry_slug is required".to_string()));
    }

    // Count current industries to check plan limit
    let current_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_industry_dashboards WHERE user_id = $1 AND is_active = true"
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
             LIMIT 1"
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
        "SELECT * FROM user_industry_dashboards WHERE user_id = $1 AND industry_slug = $2"
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

    let dashboard_name = req.dashboard_name
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
        return Err(AppError::NotFound("Industry dashboard not found".to_string()));
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
        "SELECT COUNT(*) FROM user_industry_dashboards WHERE user_id = $1 AND is_active = true"
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
         LIMIT 1"
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
