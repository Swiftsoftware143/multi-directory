//! Admin handlers: dashboard, admin listings, portfolio sync.

use axum::{
    extract::{State, Extension},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::models::*;

/// GET /api/v1/admin/directories — list all directories (admin)
pub async fn admin_list_directories(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let directories: Vec<Directory> = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories ORDER BY created_at DESC "
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(directories)))
}

/// GET /api/v1/admin/dashboard/stats
pub async fn dashboard_stats(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let total_directories = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories "
    )
    .fetch_one(&s.db)
    .await?;

    let total_businesses = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM businesses "
    )
    .fetch_one(&s.db)
    .await?;

    let total_reviews = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reviews "
    )
    .fetch_one(&s.db)
    .await?;

    let total_domains = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM domain_mappings "
    )
    .fetch_one(&s.db)
    .await?;

    let active_directories = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE status = 'published' AND status IS NOT NULL "
    )
    .fetch_one(&s.db)
    .await?;

    let published_directories = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE status = 'published'"
    )
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(DashboardStats {
        total_directories,
        total_businesses,
        total_reviews,
        total_domains,
        active_directories,
        published_directories,
    })))
}

/// POST /api/v1/admin/portfolio-sync
pub async fn portfolio_sync(
    State(_s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    // This endpoint can be called from other Swift apps to sync portfolio companies
    // Actual implementation would pull from the workflowswift portfolio_companies table
    // For now, return acknowledgement
    tracing::info!("Portfolio sync triggered");

    Ok(Json(json!({
        "message": "Portfolio sync initiated",
        "status": "processing "
    })))
}
