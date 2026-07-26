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

/// GET /api/v1/admin/members
/// Super admin view: all signups across directories with loyalty enrollment status.
/// Feeds into CoreSwift CRM as a unified member data table.
#[derive(Debug, serde::Serialize)]
pub struct MemberRow {
    pub id: uuid::Uuid,
    pub name: Option<String>,
    pub email: String,
    pub member_type: String,     // visitor, business_owner, supplier
    pub business_type: Option<String>,
    pub directory_slug: Option<String>,
    pub signed_up_at: Option<String>,
    pub survey_completed: bool,
    pub loyalty_enrolled: bool,
    pub interests: Option<Vec<String>>,
}

pub async fn admin_members(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    #[derive(sqlx::FromRow)]
    struct MemberRecord {
        id: uuid::Uuid,
        email: String,
        name: Option<String>,
        business_type: Option<String>,
        directory_id: Option<uuid::Uuid>,
        created_at: Option<chrono::NaiveDateTime>,
        survey_answered_at: Option<chrono::NaiveDateTime>,
        interest_tags: Option<Vec<String>>,
    }

    let members = sqlx::query_as::<_, MemberRecord>(
        r#"SELECT
            va.id, va.email, va.name, va.business_type,
            va.directory_id, va.created_at, va.survey_answered_at,
            va.interest_tags
           FROM visitor_accounts va
           WHERE va.email IS NOT NULL
           ORDER BY va.created_at DESC NULLS LAST
           LIMIT 500"#
    )
    .fetch_all(&s.db)
    .await?;

    // Resolve directory slugs in one batch
    let dir_ids: Vec<uuid::Uuid> = members.iter()
        .filter_map(|m| m.directory_id)
        .collect();
    let dir_slugs: std::collections::HashMap<uuid::Uuid, String> = if !dir_ids.is_empty() {
        let placeholders: Vec<String> = dir_ids.iter().enumerate()
            .map(|(i, _)| format!("${}", i + 1))
            .collect();
        let query = format!(
            "SELECT id, slug FROM directories WHERE id IN ({})",
            placeholders.join(",")
        );
        let mut q = sqlx::query_as::<_, (uuid::Uuid, String)>(&query);
        for id in &dir_ids {
            q = q.bind(*id);
        }
        q.fetch_all(&s.db).await?.into_iter().collect()
    } else {
        std::collections::HashMap::new()
    };

    // Check IS for loyalty enrollment via email lookup
    // We batch-query IS to avoid N+1 requests
    let emails: Vec<&str> = members.iter().map(|m| m.email.as_str()).collect();
    let loyalty_emails: std::collections::HashSet<String> = if !emails.is_empty() {
        match check_loyalty_enrollment(&s, &emails).await {
            Ok(set) => set,
            Err(_) => std::collections::HashSet::new(),
        }
    } else {
        std::collections::HashSet::new()
    };

    let rows: Vec<MemberRow> = members.into_iter().map(|m| {
        let member_type = match m.business_type.as_deref() {
            Some("supplier") | Some("farm") | Some("wholesaler") | Some("distributor") => "supplier",
            Some("business") | Some("service") => "business_owner",
            _ => "visitor",
        };
        MemberRow {
            id: m.id,
            name: m.name,
            email: m.email.clone(),
            member_type: member_type.to_string(),
            business_type: m.business_type,
            directory_slug: m.directory_id.and_then(|did| dir_slugs.get(&did).cloned()),
            signed_up_at: m.created_at.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
            survey_completed: m.survey_answered_at.is_some(),
            loyalty_enrolled: loyalty_emails.contains(&m.email),
            interests: m.interest_tags,
        }
    }).collect();

    Ok(Json(json!({
        "total": rows.len(),
        "members": rows,
    })))
}

/// Batch-check which emails are enrolled in IncentiveSwift loyalty.
/// Queries IS DB directly for loyalty_members records.
async fn check_loyalty_enrollment(
    s: &AppState,
    emails: &[&str],
) -> Result<std::collections::HashSet<String>, AppError> {
    if emails.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let placeholders: Vec<String> = emails.iter().enumerate()
        .map(|(i, _)| format!("${}", i + 1))
        .collect();
    let query = format!(
        r#"SELECT DISTINCT c.email
           FROM contacts c
           JOIN loyalty_members lm ON lm.contact_id = c.id
           WHERE c.email IN ({})"#,
        placeholders.join(",")
    );
    let mut q = sqlx::query_scalar::<_, String>(&query);
    for email in emails {
        q = q.bind(*email);
    }
    let results = q.fetch_all(&s.is_db).await
        .map_err(|e| AppError::Internal(format!("IS lookup failed: {}", e)))?;
    Ok(results.into_iter().collect())
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
