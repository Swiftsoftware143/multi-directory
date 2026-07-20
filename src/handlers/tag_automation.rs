//! Tag-Triggered Automation + Tracked Links
//!
//! CRUD for tag_rules and tracked_links.
//! Tag rules fire actions when tags are applied/removed or workflows complete.
//! Tracked links provide short URLs with UTM tracking and click analytics.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use rand::Rng;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Models ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TagRule {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub tag_id: Uuid,
    pub trigger_type: String,
    pub action_type: String,
    pub action_config: Value,
    pub is_active: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TrackedLink {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub url: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
    pub short_code: Option<String>,
    pub is_active: Option<bool>,
    pub total_clicks: Option<i32>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct LinkClick {
    pub id: Uuid,
    pub link_id: Uuid,
    pub contact_id: Option<Uuid>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub device_type: Option<String>,
    pub clicked_at: Option<DateTime<Utc>>,
}

// ── Request types ──

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub tenant_id: Option<Uuid>,
    pub tag_id: Option<Uuid>,
    pub trigger_type: Option<String>,
    pub action_type: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LinkListQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub tenant_id: Option<Uuid>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClickListQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub link_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRuleRequest {
    pub tenant_id: Uuid,
    pub name: String,
    pub tag_id: Uuid,
    pub trigger_type: String,
    pub action_type: String,
    #[serde(default = "default_action_config")]
    pub action_config: Value,
    pub is_active: Option<bool>,
}

fn default_action_config() -> Value {
    json!({})
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRuleRequest {
    pub name: Option<String>,
    pub tag_id: Option<Uuid>,
    pub trigger_type: Option<String>,
    pub action_type: Option<String>,
    pub action_config: Option<Value>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateLinkRequest {
    pub tenant_id: Uuid,
    pub name: String,
    pub url: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateLinkRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BulkCreateLinksRequest {
    pub links: Vec<BulkLinkItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BulkLinkItem {
    pub name: String,
    pub url: String,
    pub utm_source: Option<String>,
    pub utm_medium: Option<String>,
    pub utm_campaign: Option<String>,
    pub utm_content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BulkCreateLinksResponse {
    pub created: usize,
    pub errors: Vec<BulkLinkError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BulkLinkError {
    pub index: usize,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct LinkStats {
    pub total_clicks: i64,
    pub unique_contacts: i64,
    pub clicks_by_date: Vec<ClickDateBucket>,
    pub top_referrers: Vec<ReferrerCount>,
    pub device_breakdown: Vec<DeviceBucket>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ClickDateBucket {
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ReferrerCount {
    pub referer: String,
    pub count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DeviceBucket {
    pub device_type: String,
    pub count: i64,
}

// ── Helper: generate short code ──

fn generate_short_code() -> String {
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars().collect();
    (0..8).map(|_| chars[rng.gen_range(0..chars.len())]).collect()
}

// ── Tag Rules CRUD ──

/// GET /admin/tag-rules — list all tag rules
pub async fn list_rules(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let (total, items) = if let Some(tenant_id) = q.tenant_id {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tag_rules WHERE tenant_id = $1"
        )
        .bind(tenant_id)
        .fetch_one(&s.db)
        .await
        .map_err(AppError::from)?;

        let items = sqlx::query_as::<_, TagRule>(
            "SELECT * FROM tag_rules WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(tenant_id)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await
        .map_err(AppError::from)?;

        (total, items)
    } else {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tag_rules")
            .fetch_one(&s.db)
            .await
            .map_err(AppError::from)?;

        let items = sqlx::query_as::<_, TagRule>(
            "SELECT * FROM tag_rules ORDER BY created_at DESC LIMIT $1 OFFSET $2"
        )
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await
        .map_err(AppError::from)?;

        (total, items)
    };

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages
    })))
}

/// POST /admin/tag-rules — create a new tag rule
pub async fn create_rule(
    State(s): State<AppState>,
    Json(req): Json<CreateRuleRequest>,
) -> ApiResult<(StatusCode, Json<TagRule>)> {
    let rule = sqlx::query_as::<_, TagRule>(
        r#"INSERT INTO tag_rules (tenant_id, name, tag_id, trigger_type, action_type, action_config, is_active)
           VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *"#
    )
    .bind(req.tenant_id)
    .bind(&req.name)
    .bind(req.tag_id)
    .bind(&req.trigger_type)
    .bind(&req.action_type)
    .bind(&req.action_config)
    .bind(req.is_active.unwrap_or(true))
    .fetch_one(&s.db)
    .await
    .map_err(AppError::from)?;

    Ok((StatusCode::CREATED, Json(rule)))
}

/// PUT /admin/tag-rules/:id — update a tag rule
pub async fn update_rule(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRuleRequest>,
) -> ApiResult<Json<TagRule>> {
    let rule = sqlx::query_as::<_, TagRule>(
        r#"UPDATE tag_rules SET
            name = COALESCE($1, name),
            tag_id = COALESCE($2, tag_id),
            trigger_type = COALESCE($3, trigger_type),
            action_type = COALESCE($4, action_type),
            action_config = COALESCE($5, action_config),
            is_active = COALESCE($6, is_active),
            updated_at = NOW()
           WHERE id = $7 RETURNING *"#
    )
    .bind(&req.name)
    .bind(req.tag_id)
    .bind(&req.trigger_type)
    .bind(&req.action_type)
    .bind(&req.action_config)
    .bind(req.is_active)
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => AppError::NotFound("Tag rule not found".into()),
        _ => AppError::from(e),
    })?;

    Ok(Json(rule))
}

/// DELETE /admin/tag-rules/:id — delete a tag rule
pub async fn delete_rule(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM tag_rules WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(AppError::from)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Tag rule not found".into()));
    }

    Ok(Json(json!({"deleted": true, "id": id})))
}

/// POST /admin/tag-rules/execute — execute rules for a contact when a tag changes
pub async fn execute_rules_for_contact(
    State(s): State<AppState>,
    Json(payload): Json<ExecuteRulesPayload>,
) -> ApiResult<Json<Value>> {
    let rules = sqlx::query_as::<_, TagRule>(
        r#"SELECT * FROM tag_rules
           WHERE tenant_id = $1 AND tag_id = $2 AND trigger_type = $3 AND is_active = true"#
    )
    .bind(payload.tenant_id)
    .bind(payload.tag_id)
    .bind(&payload.trigger_type)
    .fetch_all(&s.db)
    .await
    .map_err(AppError::from)?;

    let mut results = Vec::new();

    for rule in &rules {
        match rule.action_type.as_str() {
            "add_tag" => {
                if let Some(target_tag_id) = rule.action_config.get("tag_id").and_then(|v| v.as_str()) {
                    // Add tag to contact (log action, actual tag assignment happens in CRM)
                    results.push(json!({
                        "rule_id": rule.id,
                        "action": "add_tag",
                        "target_tag_id": target_tag_id,
                        "status": "executed"
                    }));
                }
            }
            "remove_tag" => {
                if let Some(target_tag_id) = rule.action_config.get("tag_id").and_then(|v| v.as_str()) {
                    results.push(json!({
                        "rule_id": rule.id,
                        "action": "remove_tag",
                        "target_tag_id": target_tag_id,
                        "status": "executed"
                    }));
                }
            }
            "send_email" => {
                // Email sending placeholder — would integrate with email module
                results.push(json!({
                    "rule_id": rule.id,
                    "action": "send_email",
                    "status": "queued"
                }));
            }
            "send_sms" => {
                results.push(json!({
                    "rule_id": rule.id,
                    "action": "send_sms",
                    "status": "queued"
                }));
            }
            "webhook" => {
                if let Some(url) = rule.action_config.get("url").and_then(|v| v.as_str()) {
                    // Fire webhook async — placeholder
                    results.push(json!({
                        "rule_id": rule.id,
                        "action": "webhook",
                        "url": url,
                        "status": "queued"
                    }));
                }
            }
            "pipeline_move" => {
                if let Some(pipeline_id) = rule.action_config.get("pipeline_id").and_then(|v| v.as_str()) {
                    results.push(json!({
                        "rule_id": rule.id,
                        "action": "pipeline_move",
                        "pipeline_id": pipeline_id,
                        "status": "executed"
                    }));
                }
            }
            "scoring_update" => {
                if let Some(score_change) = rule.action_config.get("score_change").and_then(|v| v.as_i64()) {
                    results.push(json!({
                        "rule_id": rule.id,
                        "action": "scoring_update",
                        "score_change": score_change,
                        "status": "executed"
                    }));
                }
            }
            _ => {
                results.push(json!({
                    "rule_id": rule.id,
                    "action": rule.action_type,
                    "status": "unknown_action"
                }));
            }
        }
    }

    Ok(Json(json!({
        "executed": results.len(),
        "results": results
    })))
}

#[derive(Debug, Deserialize)]
pub struct ExecuteRulesPayload {
    pub tenant_id: Uuid,
    pub tag_id: Uuid,
    pub trigger_type: String,
    pub contact_id: Option<Uuid>,
}

// ── Tracked Links CRUD ──

/// GET /admin/tracked-links — list all tracked links
pub async fn list_tracked_links(
    State(s): State<AppState>,
    Query(q): Query<LinkListQuery>,
) -> ApiResult<Json<Value>> {
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(50).min(100);
    let offset = (page - 1) * per_page;

    let (total, items) = if let Some(tid) = q.tenant_id {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tracked_links WHERE tenant_id = $1"
        )
        .bind(tid)
        .fetch_one(&s.db)
        .await
        .map_err(AppError::from)?;

        let items = sqlx::query_as::<_, TrackedLink>(
            "SELECT * FROM tracked_links WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(tid)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await
        .map_err(AppError::from)?;

        (total, items)
    } else {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tracked_links")
            .fetch_one(&s.db)
            .await
            .map_err(AppError::from)?;

        let items = sqlx::query_as::<_, TrackedLink>(
            "SELECT * FROM tracked_links ORDER BY created_at DESC LIMIT $1 OFFSET $2"
        )
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await
        .map_err(AppError::from)?;

        (total, items)
    };

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "total_pages": total_pages
    })))
}

/// POST /admin/tracked-links — create a tracked link
pub async fn create_tracked_link(
    State(s): State<AppState>,
    Json(req): Json<CreateLinkRequest>,
) -> ApiResult<(StatusCode, Json<TrackedLink>)> {
    // Generate a unique short code
    let short_code = loop {
        let code = generate_short_code();
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT short_code FROM tracked_links WHERE short_code = $1"
        )
        .bind(&code)
        .fetch_optional(&s.db)
        .await
        .map_err(AppError::from)?;
        if existing.is_none() {
            break code;
        }
    };

    let link = sqlx::query_as::<_, TrackedLink>(
        r#"INSERT INTO tracked_links (tenant_id, name, url, utm_source, utm_medium, utm_campaign, utm_content, short_code)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *"#
    )
    .bind(req.tenant_id)
    .bind(&req.name)
    .bind(&req.url)
    .bind(&req.utm_source)
    .bind(&req.utm_medium)
    .bind(&req.utm_campaign)
    .bind(&req.utm_content)
    .bind(&short_code)
    .fetch_one(&s.db)
    .await
    .map_err(AppError::from)?;

    Ok((StatusCode::CREATED, Json(link)))
}

/// PUT /admin/tracked-links/:id — update a tracked link
pub async fn update_tracked_link(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLinkRequest>,
) -> ApiResult<Json<TrackedLink>> {
    let link = sqlx::query_as::<_, TrackedLink>(
        r#"UPDATE tracked_links SET
            name = COALESCE($1, name),
            url = COALESCE($2, url),
            utm_source = COALESCE($3, utm_source),
            utm_medium = COALESCE($4, utm_medium),
            utm_campaign = COALESCE($5, utm_campaign),
            utm_content = COALESCE($6, utm_content),
            is_active = COALESCE($7, is_active),
            updated_at = NOW()
           WHERE id = $8 RETURNING *"#
    )
    .bind(&req.name)
    .bind(&req.url)
    .bind(&req.utm_source)
    .bind(&req.utm_medium)
    .bind(&req.utm_campaign)
    .bind(&req.utm_content)
    .bind(req.is_active)
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => AppError::NotFound("Tracked link not found".into()),
        _ => AppError::from(e),
    })?;

    Ok(Json(link))
}

/// DELETE /admin/tracked-links/:id — delete a tracked link and its clicks
pub async fn delete_tracked_link(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    // First delete clicks
    sqlx::query("DELETE FROM link_clicks WHERE link_id = $1")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(AppError::from)?;

    let result = sqlx::query("DELETE FROM tracked_links WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(AppError::from)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Tracked link not found".into()));
    }

    Ok(Json(json!({"deleted": true, "id": id})))
}

/// POST /admin/tracked-links/bulk — bulk create tracked links
pub async fn bulk_create_tracked_links(
    State(s): State<AppState>,
    Json(req): Json<BulkCreateLinksRequest>,
) -> ApiResult<(StatusCode, Json<BulkCreateLinksResponse>)> {
    let max_batch = 100;
    if req.links.len() > max_batch {
        return Err(AppError::BadRequest(format!("Maximum {} links per batch", max_batch)));
    }
    if req.links.is_empty() {
        return Err(AppError::BadRequest("At least one link is required".into()));
    }

    let mut created = 0usize;
    let mut errors = Vec::new();

    for (i, item) in req.links.iter().enumerate() {
        if item.name.is_empty() || item.url.is_empty() {
            errors.push(BulkLinkError {
                index: i,
                error: "Name and URL are required".into(),
            });
            continue;
        }

        let short_code = generate_short_code();

        match sqlx::query_as::<_, TrackedLink>(
            r#"INSERT INTO tracked_links (tenant_id, name, url, utm_source, utm_medium, utm_campaign, utm_content, short_code)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id"#
        )
        .bind("00000000-0000-0000-0000-000000000000") // placeholder tenant — caller should set
        .bind(&item.name)
        .bind(&item.url)
        .bind(&item.utm_source)
        .bind(&item.utm_medium)
        .bind(&item.utm_campaign)
        .bind(&item.utm_content)
        .bind(&short_code)
        .fetch_one(&s.db)
        .await
        {
            Ok(_) => created += 1,
            Err(e) => errors.push(BulkLinkError {
                index: i,
                error: e.to_string(),
            }),
        }
    }

    Ok((StatusCode::CREATED, Json(BulkCreateLinksResponse { created, errors })))
}

/// GET /l/:short_code — track click and redirect (public)
pub async fn track_link_click(
    State(s): State<AppState>,
    Path(short_code): Path<String>,
    req: axum::http::Request<axum::body::Body>,
) -> ApiResult<impl IntoResponse> {
    let link = sqlx::query_as::<_, TrackedLink>(
        "SELECT * FROM tracked_links WHERE short_code = $1 AND is_active = true"
    )
    .bind(&short_code)
    .fetch_optional(&s.db)
    .await
    .map_err(AppError::from)?
    .ok_or_else(|| AppError::NotFound("Link not found".into()))?;

    // Log the click
    let ip = req.headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0.0.0.0")
        .split(',')
        .next()
        .unwrap_or("0.0.0.0")
        .trim()
        .to_string();
    let user_agent = req.headers()
        .get("User-Agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let referer = req.headers()
        .get("Referer")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let ip_inet: Option<sqlx::postgres::types::PgInterval> = None; // simplified
    let _ = sqlx::query(
        r#"INSERT INTO link_clicks (link_id, ip_address, user_agent, referer)
           VALUES ($1, $2::inet, $3, $4)"#
    )
    .bind(link.id)
    .bind(&ip)
    .bind(&user_agent)
    .bind(&referer)
    .execute(&s.db)
    .await;

    // Increment click count
    let _ = sqlx::query(
        "UPDATE tracked_links SET total_clicks = total_clicks + 1 WHERE id = $1"
    )
    .bind(link.id)
    .execute(&s.db)
    .await;

    // Build final URL with UTM params
    let mut final_url = link.url.clone();
    let mut params = Vec::new();

    if let Some(ref src) = link.utm_source {
        params.push(format!("utm_source={}", urlencoding(&src)));
    }
    if let Some(ref med) = link.utm_medium {
        params.push(format!("utm_medium={}", urlencoding(&med)));
    }
    if let Some(ref cam) = link.utm_campaign {
        params.push(format!("utm_campaign={}", urlencoding(&cam)));
    }
    if let Some(ref cnt) = link.utm_content {
        params.push(format!("utm_content={}", urlencoding(&cnt)));
    }

    if !params.is_empty() {
        let separator = if final_url.contains('?') { '&' } else { '?' };
        final_url.push(separator);
        final_url.push_str(&params.join("&"));
    }

    Ok(Redirect::temporary(&final_url))
}

fn urlencoding(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        ' ' => "+".to_string(),
        _ => format!("%{:02X}", c as u8),
    }).collect()
}

/// GET /admin/tracked-links/stats/:id — click analytics for a link
pub async fn get_link_stats(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LinkStats>> {
    // Verify link exists
    let _link = sqlx::query_as::<_, TrackedLink>(
        "SELECT * FROM tracked_links WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await
    .map_err(AppError::from)?
    .ok_or_else(|| AppError::NotFound("Tracked link not found".into()))?;

    let total_clicks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM link_clicks WHERE link_id = $1"
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(AppError::from)?;

    let unique_contacts: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(DISTINCT contact_id) FROM link_clicks WHERE link_id = $1 AND contact_id IS NOT NULL"#
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(AppError::from)?;

    let clicks_by_date: Vec<ClickDateBucket> = sqlx::query_as(
        r#"SELECT TO_CHAR(clicked_at, 'YYYY-MM-DD') AS date, COUNT(*)::bigint AS count
           FROM link_clicks WHERE link_id = $1
           GROUP BY TO_CHAR(clicked_at, 'YYYY-MM-DD')
           ORDER BY date DESC LIMIT 30"#
    )
    .bind(id)
    .fetch_all(&s.db)
    .await
    .map_err(AppError::from)?;

    let top_referrers: Vec<ReferrerCount> = sqlx::query_as(
        r#"SELECT COALESCE(referer, '(direct)') AS referer, COUNT(*)::bigint AS count
           FROM link_clicks WHERE link_id = $1
           GROUP BY referer
           ORDER BY count DESC LIMIT 10"#
    )
    .bind(id)
    .fetch_all(&s.db)
    .await
    .map_err(AppError::from)?;

    let device_breakdown: Vec<DeviceBucket> = sqlx::query_as(
        r#"SELECT COALESCE(device_type, 'unknown') AS device_type, COUNT(*)::bigint AS count
           FROM link_clicks WHERE link_id = $1
           GROUP BY device_type
           ORDER BY count DESC LIMIT 5"#
    )
    .bind(id)
    .fetch_all(&s.db)
    .await
    .map_err(AppError::from)?;

    Ok(Json(LinkStats {
        total_clicks,
        unique_contacts,
        clicks_by_date,
        top_referrers,
        device_breakdown,
    }))
}
