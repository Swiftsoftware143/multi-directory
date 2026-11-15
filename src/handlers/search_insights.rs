//! Search Console + GA4 for the blog engine.
//!
//! One connection per directory, credentials encrypted at rest exactly like every other
//! provider secret. A sync pulls daily query/page rows and, from them, does the two things the
//! engine was blind to: it queues keywords ranking 8-20 for the drafting queue, and it flags
//! posts whose clicks have fallen — real decay, not a guess.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::handlers::{blog, tenant_scope};
use crate::security::provider_key_crypto as keycrypto;
use crate::AppState;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GSC_BASE: &str = "https://searchconsole.googleapis.com/webmasters/v3/sites";
const GA4_BASE: &str = "https://analyticsdata.googleapis.com/v1beta/properties";

#[derive(Debug, Deserialize)]
pub struct ConnectRequest {
    pub site_url: String,
    pub ga4_property_id: Option<String>,
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

async fn access_token(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        return Err(format!(
            "Google refused the credentials: {}",
            body.get("error_description")
                .or_else(|| body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("no detail")
        ));
    }
    body.get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "token response had no access_token".to_string())
}

async fn creds_for(
    db: &PgPool,
    directory_id: Uuid,
) -> Result<(String, String, String, String, Option<String>), String> {
    let row = sqlx::query(
        "SELECT site_url, ga4_property_id, client_id, client_secret_enc, refresh_token_enc \
         FROM search_insight_connections WHERE directory_id = $1",
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("load connection: {e}"))?
    .ok_or_else(|| "No Search Console connection for this directory yet.".to_string())?;
    let site: String = row.get("site_url");
    let ga4: Option<String> = row.get("ga4_property_id");
    let client_id: String = row.get("client_id");
    let secret_enc: String = row.get("client_secret_enc");
    let token_enc: String = row.get("refresh_token_enc");
    let secret = keycrypto::decrypt_for_use(db, &secret_enc, "gsc")
        .await
        .ok_or_else(|| "stored client secret cannot be decrypted".to_string())?;
    let token = keycrypto::decrypt_for_use(db, &token_enc, "gsc")
        .await
        .ok_or_else(|| "stored refresh token cannot be decrypted".to_string())?;
    Ok((site, client_id, secret, token, ga4))
}

/// POST /api/v1/admin/directories/:id/search-insights/connect
pub async fn connect(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<ConnectRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = blog::caller(&s, &headers)?;
    tenant_scope::assert_directory_content_writer(&s.db, &claims, id).await?;

    let site = req.site_url.trim();
    if site.is_empty() {
        return Err(AppError::Validation("site_url is required".into()));
    }
    // Prove the credentials before storing them: a connection that cannot fetch is not a
    // connection, and storing one would leave the panel claiming it is linked.
    let token = access_token(&req.client_id, &req.client_secret, &req.refresh_token)
        .await
        .map_err(AppError::Validation)?;

    let secret_enc = keycrypto::encrypt_for_storage(&s.db, &req.client_secret)
        .await
        .map_err(|e| AppError::Internal(format!("encrypt client secret: {e}")))?;
    let refresh_enc = keycrypto::encrypt_for_storage(&s.db, &req.refresh_token)
        .await
        .map_err(|e| AppError::Internal(format!("encrypt refresh token: {e}")))?;

    sqlx::query(
        "INSERT INTO search_insight_connections \
            (directory_id, site_url, ga4_property_id, client_id, client_secret_enc, refresh_token_enc, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 'connected') \
         ON CONFLICT (directory_id) DO UPDATE SET \
            site_url = EXCLUDED.site_url, ga4_property_id = EXCLUDED.ga4_property_id, \
            client_id = EXCLUDED.client_id, client_secret_enc = EXCLUDED.client_secret_enc, \
            refresh_token_enc = EXCLUDED.refresh_token_enc, status = 'connected', \
            last_error = NULL, updated_at = now()",
    )
    .bind(id)
    .bind(site)
    .bind(req.ga4_property_id.as_deref().map(str::trim).filter(|v| !v.is_empty()))
    .bind(req.client_id.trim())
    .bind(&secret_enc)
    .bind(&refresh_enc)
    .execute(&s.db)
    .await?;

    // A token that works but cannot see the property is the usual mistake; say so now.
    let check = gsc_probe(site, &token).await;
    Ok(Json(json!({
        "status": "connected",
        "site_url": site,
        "token_ok": true,
        "property_access": check,
    })))
}

async fn gsc_probe(site: &str, token: &str) -> Value {
    let client = reqwest::Client::new();
    let end = Utc::now().date_naive();
    let start = end - Duration::days(7);
    let url = format!("{}/{}/searchAnalytics/query", GSC_BASE, urlencoding(site));
    match client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({
            "startDate": start.to_string(),
            "endDate": end.to_string(),
            "dimensions": ["query"],
            "rowLimit": 1
        }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => json!({"ok": true}),
        Ok(r) => {
            let code = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            json!({"ok": false, "status": code, "detail": body.chars().take(240).collect::<String>()})
        }
        Err(e) => json!({"ok": false, "detail": e.to_string()}),
    }
}

fn urlencoding(s: &str) -> String {
    // The site is a URL/domain identifier; percent-encode everything that is not unreserved.
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// GET /api/v1/admin/directories/:id/search-insights
pub async fn status(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = blog::caller(&s, &headers)?;
    tenant_scope::assert_directory_content_writer(&s.db, &claims, id).await?;
    let row = sqlx::query(
        "SELECT site_url, ga4_property_id, status, last_error, last_sync_at \
         FROM search_insight_connections WHERE directory_id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?;
    let (queries,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM gsc_query_daily WHERE directory_id = $1")
            .bind(id)
            .fetch_one(&s.db)
            .await?;
    match row {
        Some(r) => Ok(Json(json!({
            "connected": true,
            "site_url": r.get::<String, _>("site_url"),
            "ga4_property_id": r.get::<Option<String>, _>("ga4_property_id"),
            "status": r.get::<String, _>("status"),
            "last_error": r.get::<Option<String>, _>("last_error"),
            "last_sync_at": r.get::<Option<chrono::DateTime<Utc>>, _>("last_sync_at"),
            "query_rows": queries,
        }))),
        None => Ok(Json(json!({"connected": false, "query_rows": 0}))),
    }
}

/// POST /api/v1/admin/directories/:id/search-insights/sync?days=28
pub async fn sync(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = blog::caller(&s, &headers)?;
    tenant_scope::assert_directory_content_writer(&s.db, &claims, id).await?;
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(28)
        .clamp(7, 90);

    let (site, client_id, secret, refresh, ga4) =
        creds_for(&s.db, id).await.map_err(AppError::Validation)?;
    let token = access_token(&client_id, &secret, &refresh)
        .await
        .map_err(AppError::Validation)?;

    let (mut gsc_rows, mut ga4_rows) = (0usize, 0usize);
    let mut errors: Vec<String> = Vec::new();

    match fetch_gsc(&s.db, id, &site, &token, days).await {
        Ok(n) => gsc_rows = n,
        Err(e) => errors.push(format!("Search Console: {e}")),
    }
    if let Some(prop) = ga4.as_deref().filter(|p| !p.is_empty()) {
        match fetch_ga4(&s.db, id, prop, &token, days).await {
            Ok(n) => ga4_rows = n,
            Err(e) => errors.push(format!("GA4: {e}")),
        }
    }

    let opportunities = queue_opportunities(&s.db, id).await.unwrap_or(0);
    let decayed = flag_decay(&s.db, id).await.unwrap_or(0);

    sqlx::query(
        "UPDATE search_insight_connections SET last_sync_at = now(), \
                status = CASE WHEN $2 = '' THEN 'connected' ELSE 'error' END, \
                last_error = NULLIF($2, ''), updated_at = now() \
         WHERE directory_id = $1",
    )
    .bind(id)
    .bind(errors.join(" | "))
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "gsc_rows": gsc_rows,
        "ga4_rows": ga4_rows,
        "opportunities_queued": opportunities,
        "posts_flagged_for_refresh": decayed,
        "errors": errors,
    })))
}

async fn fetch_gsc(
    db: &PgPool,
    directory_id: Uuid,
    site: &str,
    token: &str,
    days: i64,
) -> Result<usize, String> {
    let client = reqwest::Client::new();
    let end = Utc::now().date_naive();
    let start = end - Duration::days(days);
    let url = format!("{}/{}/searchAnalytics/query", GSC_BASE, urlencoding(site));
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({
            "startDate": start.to_string(),
            "endDate": end.to_string(),
            "dimensions": ["date", "query", "page"],
            "rowLimit": 25000
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status.as_u16(),
            body.to_string().chars().take(200).collect::<String>()
        ));
    }
    let rows = body
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut n = 0usize;
    for row in rows {
        let keys = row
            .get("keys")
            .and_then(|k| k.as_array())
            .cloned()
            .unwrap_or_default();
        if keys.len() < 3 {
            continue;
        }
        let day = keys[0].as_str().unwrap_or_default();
        let query = keys[1].as_str().unwrap_or_default();
        let page = keys[2].as_str().unwrap_or_default();
        let clicks = row.get("clicks").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let impressions = row.get("impressions").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let ctr = row.get("ctr").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let position = row.get("position").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sqlx::query(
            "INSERT INTO gsc_query_daily (directory_id, day, query, page, clicks, impressions, ctr, position) \
             VALUES ($1, $2::date, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (directory_id, day, query, page) DO UPDATE SET \
                clicks = EXCLUDED.clicks, impressions = EXCLUDED.impressions, \
                ctr = EXCLUDED.ctr, position = EXCLUDED.position",
        )
        .bind(directory_id)
        .bind(day)
        .bind(query)
        .bind(page)
        .bind(clicks)
        .bind(impressions)
        .bind(ctr)
        .bind(position)
        .execute(db)
        .await
        .map_err(|e| format!("store gsc row: {e}"))?;

        sqlx::query(
            "INSERT INTO gsc_page_daily (directory_id, day, page, clicks, impressions, ctr, position) \
             VALUES ($1, $2::date, $3, $4, $5, $6, $7) \
             ON CONFLICT (directory_id, day, page) DO UPDATE SET \
                clicks = gsc_page_daily.clicks + EXCLUDED.clicks, \
                impressions = gsc_page_daily.impressions + EXCLUDED.impressions, \
                position = EXCLUDED.position",
        )
        .bind(directory_id)
        .bind(day)
        .bind(page)
        .bind(clicks)
        .bind(impressions)
        .bind(ctr)
        .bind(position)
        .execute(db)
        .await
        .map_err(|e| format!("store gsc page row: {e}"))?;
        n += 1;
    }
    Ok(n)
}

async fn fetch_ga4(
    db: &PgPool,
    directory_id: Uuid,
    property: &str,
    token: &str,
    days: i64,
) -> Result<usize, String> {
    let client = reqwest::Client::new();
    let url = format!("{GA4_BASE}/{property}:runReport");
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({
            "dateRanges": [{"startDate": format!("{}daysAgo", days), "endDate": "today"}],
            "dimensions": [{"name": "date"}, {"name": "pagePath"}],
            "metrics": [{"name": "sessions"}, {"name": "screenPageViews"}],
            "limit": 100000
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status.as_u16(),
            body.to_string().chars().take(200).collect::<String>()
        ));
    }
    let mut n = 0usize;
    for row in body
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
    {
        let dims = row
            .get("dimensionValues")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let mets = row
            .get("metricValues")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        if dims.len() < 2 || mets.len() < 2 {
            continue;
        }
        let raw = dims[0].get("value").and_then(|v| v.as_str()).unwrap_or("");
        if raw.len() != 8 {
            continue;
        }
        let day = format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]);
        let path = dims[1].get("value").and_then(|v| v.as_str()).unwrap_or("");
        let sessions = mets[0]
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse::<i32>()
            .unwrap_or(0);
        let views = mets[1]
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse::<i32>()
            .unwrap_or(0);
        sqlx::query(
            "INSERT INTO ga4_page_daily (directory_id, day, page_path, sessions, views) \
             VALUES ($1, $2::date, $3, $4, $5) \
             ON CONFLICT (directory_id, day, page_path) DO UPDATE SET \
                sessions = EXCLUDED.sessions, views = EXCLUDED.views",
        )
        .bind(directory_id)
        .bind(day)
        .bind(path)
        .bind(sessions)
        .bind(views)
        .execute(db)
        .await
        .map_err(|e| format!("store ga4 row: {e}"))?;
        n += 1;
    }
    Ok(n)
}

/// Queries ranking 8-20 with real impressions are the ones a new post can take. They go into the
/// existing drafting queue as topics, once each.
async fn queue_opportunities(db: &PgPool, directory_id: Uuid) -> Result<usize, String> {
    let rows: Vec<(String, f64, i64)> = sqlx::query_as(
        "SELECT query, avg(position) AS pos, sum(impressions)::bigint \
         FROM gsc_query_daily \
         WHERE directory_id = $1 AND day >= current_date - 28 \
         GROUP BY query \
         HAVING avg(position) BETWEEN 8 AND 20 AND sum(impressions) >= 50 \
         ORDER BY sum(impressions) DESC LIMIT 100",
    )
    .bind(directory_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("query opportunities: {e}"))?;
    let mut queued = 0usize;
    for (query, pos, impressions) in rows {
        let exists: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM content_topics WHERE directory_id = $1 AND target_keyword = $2 LIMIT 1",
        )
        .bind(directory_id)
        .bind(&query)
        .fetch_optional(db)
        .await
        .unwrap_or(None);
        if exists.is_some() {
            continue;
        }
        sqlx::query(
            "INSERT INTO content_topics (directory_id, title, target_keyword, status) \
             VALUES ($1, $2, $3, 'suggested')",
        )
        .bind(directory_id)
        .bind(format!(
            "'{query}' — position {:.1}, {impressions} impressions (28d)",
            pos
        ))
        .bind(&query)
        .execute(db)
        .await
        .map_err(|e| format!("insert topic: {e}"))?;
        queued += 1;
    }
    Ok(queued)
}

/// Real decay: clicks in the last 14 days against the 14 before. A post down 25% is flagged.
async fn flag_decay(db: &PgPool, directory_id: Uuid) -> Result<usize, String> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT page, \
                sum(clicks) FILTER (WHERE day >= current_date - 14)::bigint AS recent, \
                sum(clicks) FILTER (WHERE day >= current_date - 28 AND day < current_date - 14)::bigint AS prior \
         FROM gsc_page_daily WHERE directory_id = $1 GROUP BY page",
    )
    .bind(directory_id)
    .fetch_all(db)
    .await
    .map_err(|e| format!("query decay: {e}"))?;
    let mut flagged = 0usize;
    for (page, recent, prior) in rows {
        if prior <= 0 {
            continue;
        }
        let drop = 1.0 - (recent as f64 / prior as f64);
        if drop < 0.25 {
            continue;
        }
        let slug = page.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        if slug.is_empty() {
            continue;
        }
        let r = sqlx::query(
            "UPDATE blog_posts SET decay_flag = CASE WHEN $3 >= 0.5 THEN 'high' ELSE 'medium' END, \
                    refresh_priority = $3, traffic_trend = 'down', updated_at = updated_at \
             WHERE directory_id = $1 AND slug = $2",
        )
        .bind(directory_id)
        .bind(slug)
        .bind(drop)
        .execute(db)
        .await
        .map_err(|e| format!("flag decay: {e}"))?;
        flagged += r.rows_affected() as usize;
    }
    Ok(flagged)
}

/// GET /api/v1/admin/directories/:id/search-insights/summary
pub async fn summary(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = blog::caller(&s, &headers)?;
    tenant_scope::assert_directory_content_writer(&s.db, &claims, id).await?;
    let top: Vec<(String, i64, i64, f64)> = sqlx::query_as(
        "SELECT query, sum(clicks)::bigint, sum(impressions)::bigint, avg(position) \
         FROM gsc_query_daily WHERE directory_id = $1 AND day >= current_date - 28 \
         GROUP BY query ORDER BY sum(clicks) DESC LIMIT 20",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    let opportunities: Vec<(String, f64, i64)> = sqlx::query_as(
        "SELECT query, avg(position), sum(impressions)::bigint FROM gsc_query_daily \
         WHERE directory_id = $1 AND day >= current_date - 28 \
         GROUP BY query HAVING avg(position) BETWEEN 8 AND 20 AND sum(impressions) >= 50 \
         ORDER BY sum(impressions) DESC LIMIT 20",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    let traffic: Vec<(String, i64)> = sqlx::query_as(
        "SELECT page_path, sum(sessions)::bigint FROM ga4_page_daily \
         WHERE directory_id = $1 AND day >= current_date - 28 \
         GROUP BY page_path ORDER BY sum(sessions) DESC LIMIT 20",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    Ok(Json(json!({
        "top_queries": top.iter().map(|(q,c,i,p)| json!({"query":q,"clicks":c,"impressions":i,"position":p})).collect::<Vec<_>>(),
        "opportunities": opportunities.iter().map(|(q,p,i)| json!({"query":q,"position":p,"impressions":i})).collect::<Vec<_>>(),
        "top_pages_by_sessions": traffic.iter().map(|(p,s)| json!({"page":p,"sessions":s})).collect::<Vec<_>>(),
    })))
}
