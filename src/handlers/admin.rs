//! Admin handlers: dashboard, admin listings, portfolio sync.

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use crate::error::{ApiResult, AppError};
use crate::models::*;
use crate::AppState;

/// GET /api/v1/admin/dashboard/stats
pub async fn dashboard_stats(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let total_directories = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories ")
        .fetch_one(&s.db)
        .await?;

    let total_businesses = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM businesses ")
        .fetch_one(&s.db)
        .await?;

    let total_reviews = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reviews ")
        .fetch_one(&s.db)
        .await?;

    let total_domains = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM domain_mappings ")
        .fetch_one(&s.db)
        .await?;

    // B91 completeness fix: a directory's lifecycle value for "published/live" is 'active'
    // ('draft' / 'prospect' are the hidden states — see models/directory.rs). 'published' is
    // not a directory status, so both counters below read 0 while all 10 live cities were
    // 'active'. Count the real value.
    let active_directories =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE status = 'active'")
            .fetch_one(&s.db)
            .await?;

    let published_directories =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE status = 'active'")
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
    pub member_type: String, // visitor, business_owner, supplier
    pub business_type: Option<String>,
    pub directory_slug: Option<String>,
    pub signed_up_at: Option<String>,
    pub survey_completed: bool,
    pub loyalty_enrolled: bool,
    pub interests: Option<Vec<String>>,
}

pub async fn admin_members(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    #[derive(sqlx::FromRow)]
    struct MemberRecord {
        id: uuid::Uuid,
        email: String,
        name: Option<String>,
        business_type: Option<String>,
        directory_id: Option<uuid::Uuid>,
        created_at: Option<chrono::DateTime<chrono::Utc>>,
        survey_answered_at: Option<chrono::DateTime<chrono::Utc>>,
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
           LIMIT 500"#,
    )
    .fetch_all(&s.db)
    .await?;

    // Resolve directory slugs in one batch. `= ANY($1)` with the id list bound as one uuid[]
    // parameter: the statement text is a compile-time literal and only the VALUES travel as a
    // bind, so no SQL is assembled at run time (class-14 paydown, kanban t_faba8c76).
    let dir_ids: Vec<uuid::Uuid> = members.iter().filter_map(|m| m.directory_id).collect();
    let dir_slugs: std::collections::HashMap<uuid::Uuid, String> = if !dir_ids.is_empty() {
        sqlx::query_as::<_, (uuid::Uuid, String)>(
            "SELECT id, slug FROM directories WHERE id = ANY($1)",
        )
        .bind(&dir_ids)
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .collect()
    } else {
        std::collections::HashMap::new()
    };

    // Check native Multi-Directory loyalty enrollment (loyalty_members) via email lookup.
    // This used to query the IncentiveSwift database directly; loyalty is native MD code
    // (ZaarCash) and there is no second pool any more (kanban t_20e0bcd5).
    let emails: Vec<&str> = members.iter().map(|m| m.email.as_str()).collect();
    let loyalty_emails: std::collections::HashSet<String> = if !emails.is_empty() {
        match check_loyalty_enrollment(&s, &emails).await {
            Ok(set) => set,
            Err(_) => std::collections::HashSet::new(),
        }
    } else {
        std::collections::HashSet::new()
    };

    let rows: Vec<MemberRow> = members
        .into_iter()
        .map(|m| {
            let member_type = match m.business_type.as_deref() {
                Some("supplier") | Some("farm") | Some("wholesaler") | Some("distributor") => {
                    "supplier"
                }
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
        })
        .collect();

    Ok(Json(json!({
        "total": rows.len(),
        "members": rows,
    })))
}

/// Batch-check which emails are enrolled in Multi-Directory's native loyalty programme.
/// Reads the app's own `loyalty_members` joined to `visitor_accounts` — loyalty is native
/// ZaarCash code, never another app's database (kanban t_20e0bcd5).
async fn check_loyalty_enrollment(
    s: &AppState,
    emails: &[&str],
) -> Result<std::collections::HashSet<String>, AppError> {
    if emails.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    // `= ANY($1)` with the email list bound as one text[] parameter: statement text is a
    // compile-time literal, only the VALUES travel as a bind (class-14 paydown, kanban t_faba8c76).
    let emails_owned: Vec<String> = emails.iter().map(|e| e.to_string()).collect();
    let results = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT va.email
           FROM loyalty_members lm
           JOIN visitor_accounts va ON va.id = lm.visitor_account_id
           WHERE va.email = ANY($1)"#,
    )
    .bind(&emails_owned)
    .fetch_all(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("loyalty lookup failed: {}", e)))?;
    Ok(results.into_iter().collect())
}

/// POST /api/v1/admin/portfolio-sync
pub async fn portfolio_sync(State(_s): State<AppState>) -> ApiResult<impl IntoResponse> {
    // This endpoint can be called from other Swift apps to sync portfolio companies
    // Actual implementation would pull from the workflowswift portfolio_companies table
    // For now, return acknowledgement
    tracing::info!("Portfolio sync triggered");

    Ok(Json(json!({
        "message": "Portfolio sync initiated",
        "status": "processing "
    })))
}
