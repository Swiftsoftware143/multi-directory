//! Nightly sitemap regeneration + search-engine submission (B91 / B117 gap #10).
//!
//! The SEO engine *serves* sitemaps but nothing ever told a search engine that
//! content changed, and nothing regenerated on a schedule. This module adds the
//! missing submission loop behind the admin panel:
//!
//!   * ONE editable settings row (`seo_submission_settings`, migration 144) —
//!     `is_enabled`, `cadence_hours`, an optional `sitemap_url` override and a
//!     `ping_targets` JSON array of `{name,url,enabled}`. Nothing is hardwired:
//!     the default Google/Bing targets are seeded as DATA on first access and can
//!     be replaced from the panel with any endpoint (e.g. an IndexNow consumer).
//!   * `{sitemap}` in a target URL is substituted with the URL-encoded sitemap
//!     URL; `{sitemap_raw}` with the raw URL.
//!   * a background task checks every 5 minutes for a due global row and runs it;
//!     the same run is available on demand (`POST /seo/run-submission`) and each
//!     target hit is written to `seo_submission_log` (status + ok + detail) so the
//!     panel can show what the engine actually answered.
//!
//! A disabled row is never touched, an unconfigured row records `nothing_enabled`
//! instead of faking success, and no failure panics the scheduler.

use axum::{extract::State, response::IntoResponse, Extension, Json};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::middleware::is_super_admin;
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// Seeded the first time the panel (or the scheduler) reads an unconfigured row.
/// These are ordinary rows afterwards — an admin may edit, disable or replace
/// them. Google/Bing's legacy `?sitemap=` endpoints are kept as the honest
/// default; a buyer points the row at whatever their engines support.
const DEFAULT_TARGETS: &str = r#"[
  {"name":"Google","url":"https://www.google.com/ping?sitemap={sitemap}","enabled":true},
  {"name":"Bing","url":"https://www.bing.com/ping?sitemap={sitemap}","enabled":true}
]"#;

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingTarget {
    pub name: String,
    pub url: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SubmissionSettings {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub is_enabled: bool,
    pub cadence_hours: i32,
    pub sitemap_url: Option<String>,
    pub ping_targets: Value,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSubmissionRequest {
    pub is_enabled: Option<bool>,
    pub cadence_hours: Option<i32>,
    pub sitemap_url: Option<String>,
    pub ping_targets: Option<Vec<PingTarget>>,
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Percent-encode a value for use inside a query string. Only the unreserved
/// set survives; everything else (the `:` and `/` of a URL) becomes `%XX`.
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn resolve_sitemap(override_url: Option<&str>, base_domain: &str) -> String {
    match override_url {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => format!("https://{}/sitemap.xml", base_domain.trim_end_matches('/')),
    }
}

/// Read the platform-wide row, creating it (disabled, with default targets) if it
/// does not exist yet. `ON CONFLICT DO NOTHING` keeps a concurrent boot safe.
async fn ensure_global_row(db: &PgPool) -> Result<SubmissionSettings, AppError> {
    if let Some(row) = sqlx::query_as::<_, SubmissionSettings>(
        "SELECT * FROM seo_submission_settings WHERE directory_id IS NULL \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await?
    {
        return Ok(row);
    }

    let defaults: Value = serde_json::from_str(DEFAULT_TARGETS).unwrap_or_else(|_| json!([]));
    let inserted = sqlx::query_as::<_, SubmissionSettings>(
        "INSERT INTO seo_submission_settings (directory_id, is_enabled, cadence_hours, ping_targets) \
         VALUES (NULL, false, 24, $1) ON CONFLICT DO NOTHING RETURNING *",
    )
    .bind(&defaults)
    .fetch_optional(db)
    .await?;

    match inserted {
        Some(row) => Ok(row),
        None => Ok(sqlx::query_as::<_, SubmissionSettings>(
            "SELECT * FROM seo_submission_settings WHERE directory_id IS NULL \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_one(db)
        .await?),
    }
}

/// Run one submission pass for a settings row: hit every enabled target, log each
/// result, then stamp last_run_at / last_status / next_run_at. Never returns an
/// error for a target that failed — those are recorded, not propagated.
pub async fn run_submission(
    db: &PgPool,
    row: &SubmissionSettings,
    base_domain: &str,
) -> Result<Value, AppError> {
    let sitemap = resolve_sitemap(row.sitemap_url.as_deref(), base_domain);
    let encoded = encode_component(&sitemap);
    let targets: Vec<PingTarget> =
        serde_json::from_value(row.ping_targets.clone()).unwrap_or_default();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(format!("http client: {e}")))?;

    let mut results = Vec::new();
    let mut any_ok = false;
    let mut enabled_count = 0usize;

    for t in targets.iter().filter(|t| t.enabled) {
        enabled_count += 1;
        let url = t
            .url
            .replace("{sitemap}", &encoded)
            .replace("{sitemap_raw}", &sitemap);

        let (status_code, ok, detail) = match client.get(&url).send().await {
            Ok(resp) => {
                let code = resp.status().as_u16() as i32;
                let ok = resp.status().is_success();
                (
                    Some(code),
                    ok,
                    if ok {
                        None
                    } else {
                        Some(format!("HTTP {code}"))
                    },
                )
            }
            Err(e) => (None, false, Some(e.to_string())),
        };
        any_ok = any_ok || ok;

        sqlx::query(
            "INSERT INTO seo_submission_log \
             (directory_id, target_name, endpoint_url, sitemap_url, status_code, ok, detail) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(row.directory_id)
        .bind(&t.name)
        .bind(&url)
        .bind(&sitemap)
        .bind(status_code)
        .bind(ok)
        .bind(&detail)
        .execute(db)
        .await?;

        results.push(json!({
            "target": t.name,
            "url": url,
            "status_code": status_code,
            "ok": ok,
            "detail": detail,
        }));
    }

    let status = if enabled_count == 0 {
        "nothing_enabled"
    } else if any_ok {
        "ok"
    } else {
        "failed"
    };
    let next = Utc::now() + Duration::hours(i64::from(row.cadence_hours.max(1)));
    sqlx::query(
        "UPDATE seo_submission_settings \
         SET last_run_at = now(), last_status = $1, next_run_at = $2, updated_at = now() \
         WHERE id = $3",
    )
    .bind(status)
    .bind(next)
    .bind(row.id)
    .execute(db)
    .await?;

    Ok(json!({
        "status": status,
        "sitemap_url": sitemap,
        "targets_submitted": enabled_count,
        "results": results,
    }))
}

// ── HTTP surface ────────────────────────────────────────────────────────────

/// GET /api/v1/seo/submission-settings
pub async fn get_settings(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to view sitemap submission settings".to_string(),
        ));
    }

    let row = ensure_global_row(&s.db).await?;
    let resolved = resolve_sitemap(row.sitemap_url.as_deref(), &s.config.base_domain);

    let recent = sqlx::query_as::<_, (DateTime<Utc>, String, String, Option<i32>, bool)>(
        "SELECT created_at, target_name, endpoint_url, status_code, ok \
         FROM seo_submission_log ORDER BY created_at DESC LIMIT 20",
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({
        "settings": row,
        "resolved_sitemap_url": resolved,
        "recent": recent
            .into_iter()
            .map(|(at, target, endpoint, code, ok)| json!({
                "at": at, "target": target, "endpoint": endpoint,
                "status_code": code, "ok": ok,
            }))
            .collect::<Vec<_>>(),
    })))
}

/// PUT /api/v1/seo/submission-settings
pub async fn update_settings(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<UpdateSubmissionRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to edit sitemap submission settings".to_string(),
        ));
    }

    let current = ensure_global_row(&s.db).await?;
    let is_enabled = body.is_enabled.unwrap_or(current.is_enabled);
    let cadence = body.cadence_hours.unwrap_or(current.cadence_hours).max(1);
    let sitemap = body.sitemap_url.or_else(|| current.sitemap_url.clone());
    let targets = match body.ping_targets {
        Some(t) => serde_json::to_value(t).unwrap_or_else(|_| json!([])),
        None => current.ping_targets.clone(),
    };

    let row = sqlx::query_as::<_, SubmissionSettings>(
        "UPDATE seo_submission_settings \
         SET is_enabled = $1, cadence_hours = $2, sitemap_url = $3, ping_targets = $4, \
             updated_at = now() \
         WHERE id = $5 RETURNING *",
    )
    .bind(is_enabled)
    .bind(cadence)
    .bind(&sitemap)
    .bind(&targets)
    .bind(current.id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({ "settings": row })))
}

/// POST /api/v1/seo/run-submission — run the submission pass now.
pub async fn run_now(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to run sitemap submission".to_string(),
        ));
    }

    let row = ensure_global_row(&s.db).await?;
    let outcome = run_submission(&s.db, &row, &s.config.base_domain).await?;
    Ok(Json(outcome))
}

/// Start the submission scheduler — checks every 5 minutes for a due global row.
pub fn start_submission_scheduler(db: PgPool, base_domain: String) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            interval.tick().await;

            let row = match ensure_global_row(&db).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("[seo-submit] could not read settings: {}", e);
                    continue;
                }
            };

            if !row.is_enabled {
                continue;
            }
            let due = row.next_run_at.map(|n| n <= Utc::now()).unwrap_or(true);
            if !due {
                continue;
            }

            tracing::info!("[seo-submit] settings due — running submission pass");
            match run_submission(&db, &row, &base_domain).await {
                Ok(v) => tracing::info!(
                    "[seo-submit] pass complete: {} ({} targets)",
                    v["status"],
                    v["targets_submitted"]
                ),
                Err(e) => tracing::warn!("[seo-submit] pass failed: {}", e),
            }
        }
    });
}
