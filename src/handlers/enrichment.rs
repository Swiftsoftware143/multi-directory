//! Enrichment cycle (T4) — the "verified contact intelligence" claim, actually run.
//!
//! `data_enrichment_logs` held 0 rows because nothing ran on a schedule. This
//! module adds the rotating re-enrichment cycle plus the admin controls:
//!
//!   * cadence + batch size + enabled flag come from `enrichment_settings`
//!     (editable in the admin Data Enrichment card);
//!   * the SEARCH PROVIDER comes from `provider_keys` — never hardcoded. The
//!     configured key decides which adapter runs (google_places / serpapi / bing /
//!     google_cse); `enrichment_settings.provider` may pin one, NULL means "use
//!     whatever is configured, preferring the default key";
//!   * with no provider configured (or a key the provider rejects) the run records
//!     a SKIP/ERROR and writes no enrichment — it never fakes success and never
//!     panics;
//!   * businesses rotate on `enriched_at ASC NULLS FIRST` so every record in a
//!     directory is revisited over time instead of the same head-of-list forever.
//!
//! A background task checks every 5 minutes for a due settings row; the same
//! cycle can be triggered manually from the card (`POST /enrich/run`) or by an
//! external cron.

use axum::{extract::Query, extract::State, response::IntoResponse, Extension, Json};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// The search adapters this app can actually speak. Which one RUNS is decided by
/// `provider_keys` (+ the optional pin in `enrichment_settings.provider`).
const SEARCH_ADAPTERS: [&str; 4] = ["google_places", "serpapi", "bing", "google_cse"];

const DEFAULT_GOOGLE_PLACES_BASE: &str = "https://maps.googleapis.com/maps/api/place";
const SERPAPI_BASE: &str = "https://serpapi.com";
const BING_BASE: &str = "https://api.bing.microsoft.com";
const GOOGLE_CSE_BASE: &str = "https://www.googleapis.com/customsearch/v1";

fn is_admin(claims: &Claims) -> bool {
    claims.role == "admin" || claims.role == "super_admin"
}

// ─────────────────────────────────────────────────────────────────────────────
// Settings
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EnrichmentSettings {
    pub id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub is_enabled: bool,
    pub cadence_hours: i32,
    pub batch_size: i32,
    pub provider: Option<String>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl EnrichmentSettings {
    fn defaults() -> Self {
        Self {
            id: None,
            directory_id: None,
            is_enabled: false,
            cadence_hours: 24,
            batch_size: 25,
            provider: None,
            last_run_at: None,
            last_status: None,
            next_run_at: None,
            updated_at: None,
        }
    }

    fn sanitize(mut self) -> Self {
        if self.cadence_hours < 1 {
            self.cadence_hours = 24;
        }
        if self.batch_size < 1 || self.batch_size > 500 {
            self.batch_size = 25;
        }
        self
    }
}

/// A directory override wins; otherwise the global default row; otherwise the
/// code defaults. A missing row degrades, never 500s.
pub async fn effective_settings(
    db: &sqlx::PgPool,
    directory_id: Option<Uuid>,
) -> Result<EnrichmentSettings, sqlx::Error> {
    if let Some(dir) = directory_id {
        let row = sqlx::query_as::<_, EnrichmentSettings>(
            "SELECT * FROM enrichment_settings WHERE directory_id = $1 ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(dir)
        .fetch_optional(db)
        .await?;
        if let Some(r) = row {
            return Ok(r.sanitize());
        }
    }

    let row = sqlx::query_as::<_, EnrichmentSettings>(
        "SELECT * FROM enrichment_settings WHERE directory_id IS NULL ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await?;
    Ok(row.unwrap_or_else(EnrichmentSettings::defaults).sanitize())
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider resolution (from provider_keys — never hardcoded)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProviderCfg {
    pub provider: String,
    pub label: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Value,
}

/// The configured search providers, default key first. Read at call time from
/// `provider_keys` (decrypted when the encrypted copy exists).
pub async fn configured_search_providers(
    db: &sqlx::PgPool,
) -> Result<Vec<ProviderCfg>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<Value>)>(
        "SELECT provider, label, COALESCE(decrypt_provider_key(api_key_encrypted), api_key) AS api_key, \
                base_url, metadata \
         FROM provider_keys \
         WHERE is_active = true AND provider = ANY($1) \
         ORDER BY is_default DESC, updated_at DESC",
    )
    .bind(&SEARCH_ADAPTERS[..])
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .filter(|(_, _, key, _, _)| !key.trim().is_empty())
        .map(
            |(provider, label, api_key, base_url, metadata)| ProviderCfg {
                provider,
                label,
                api_key,
                base_url,
                metadata: metadata.unwrap_or_else(|| json!({})),
            },
        )
        .collect())
}

/// Which adapter runs: the pinned provider when it is configured, else the first
/// configured one (preferring the marked default). None = nothing configured.
async fn resolve_provider(
    db: &sqlx::PgPool,
    pinned: Option<&str>,
) -> Result<Option<ProviderCfg>, sqlx::Error> {
    let available = configured_search_providers(db).await?;
    if let Some(pin) = pinned {
        if let Some(cfg) = available.iter().find(|c| c.provider == pin) {
            return Ok(Some(cfg.clone()));
        }
    }
    if let Some(pin) = pinned {
        if !SEARCH_ADAPTERS.contains(&pin) {
            tracing::warn!(
                "[enrich] configured provider '{}' is not a search adapter this build speaks; falling back to a configured one",
                pin
            );
        }
    }
    Ok(available.into_iter().next())
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider adapters
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct Enriched {
    pub name: Option<String>,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub rating: Option<f64>,
    pub confidence: f64,
}

impl Enriched {
    fn score(mut self) -> Self {
        self.confidence = match (self.phone.is_some(), self.website.is_some()) {
            (true, true) => 0.9,
            (true, false) | (false, true) => 0.7,
            (false, false) => 0.4,
        };
        self
    }
}

fn phone_from_text(text: &str) -> Option<String> {
    // Last resort for providers that only return a snippet (CSE): pull a
    // US-shaped phone number out of the result text.
    let digits: String = text
        .chars()
        .map(|c| if c.is_ascii_digit() { c } else { ' ' })
        .collect();
    for chunk in digits.split_whitespace() {
        if chunk.len() == 10 || chunk.len() == 11 {
            return Some(chunk.to_string());
        }
    }
    None
}

async fn http_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("http client: {}", e))?;
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.map_err(|e| format!("request: {}", e))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("body: {}", e))?;
    if !status.is_success() {
        let peek: String = body.chars().take(200).collect();
        return Err(format!("HTTP {}: {}", status.as_u16(), peek));
    }
    serde_json::from_str(&body).map_err(|e| {
        format!(
            "json: {} | {}",
            e,
            body.chars().take(160).collect::<String>()
        )
    })
}

/// Ask the configured provider about one business. `Err` means the provider
/// answered with an error (bad/expired key, quota, transport) — the caller logs it
/// and moves on; `Ok(None)` means a clean "no match".
async fn provider_search(cfg: &ProviderCfg, query: &str) -> Result<Option<Enriched>, String> {
    let q = urlencoding(query.trim());
    match cfg.provider.as_str() {
        "google_places" => {
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_GOOGLE_PLACES_BASE.to_string());
            let url = format!(
                "{}/findplacefromtext/json?input={}&inputtype=textquery&fields=place_id,name,formatted_address,formatted_phone_number,website,geometry,rating&key={}",
                base.trim_end_matches('/'),
                q,
                cfg.api_key
            );
            let v = http_json(&url, &[]).await?;
            let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
            if status != "OK" && status != "ZERO_RESULTS" {
                // REQUEST_DENIED / INVALID_REQUEST / OVER_QUERY_LIMIT — an error, never a fake match.
                return Err(format!(
                    "google_places status {} ({})",
                    status,
                    v.get("error_message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("-")
                ));
            }
            let p = match v
                .get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
            {
                Some(p) => p,
                None => return Ok(None),
            };
            Ok(Some(
                Enriched {
                    name: p.get("name").and_then(|x| x.as_str()).map(String::from),
                    address: p
                        .get("formatted_address")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    phone: p
                        .get("formatted_phone_number")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    website: p.get("website").and_then(|x| x.as_str()).map(String::from),
                    lat: p
                        .get("geometry")
                        .and_then(|g| g.get("location"))
                        .and_then(|l| l.get("lat"))
                        .and_then(|x| x.as_f64()),
                    lng: p
                        .get("geometry")
                        .and_then(|g| g.get("location"))
                        .and_then(|l| l.get("lng"))
                        .and_then(|x| x.as_f64()),
                    rating: p.get("rating").and_then(|x| x.as_f64()),
                    confidence: 0.0,
                }
                .score(),
            ))
        }
        "serpapi" => {
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| SERPAPI_BASE.to_string());
            let url = format!(
                "{}/search.json?engine=google_maps&q={}&api_key={}",
                base.trim_end_matches('/'),
                q,
                cfg.api_key
            );
            let v = http_json(&url, &[]).await?;
            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                return Err(format!("serpapi: {}", err));
            }
            let first = v
                .get("local_results")
                .and_then(|l| l.as_array())
                .and_then(|a| a.first())
                .or_else(|| {
                    v.get("place_results").filter(|p| {
                        !p.is_null() && !p.as_object().map(|o| o.is_empty()).unwrap_or(true)
                    })
                });
            let p = match first {
                Some(p) => p,
                None => return Ok(None),
            };
            Ok(Some(
                Enriched {
                    name: p.get("title").and_then(|x| x.as_str()).map(String::from),
                    address: p.get("address").and_then(|x| x.as_str()).map(String::from),
                    phone: p.get("phone").and_then(|x| x.as_str()).map(String::from),
                    website: p.get("website").and_then(|x| x.as_str()).map(String::from),
                    lat: p
                        .get("gps_coordinates")
                        .and_then(|g| g.get("latitude"))
                        .and_then(|x| x.as_f64()),
                    lng: p
                        .get("gps_coordinates")
                        .and_then(|g| g.get("longitude"))
                        .and_then(|x| x.as_f64()),
                    rating: p.get("rating").and_then(|x| x.as_f64()),
                    confidence: 0.0,
                }
                .score(),
            ))
        }
        "bing" => {
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| BING_BASE.to_string());
            let url = format!(
                "{}/v7.0/localbusinesses/search?q={}&count=1&mkt=en-US",
                base.trim_end_matches('/'),
                q
            );
            let v = http_json(&url, &[("Ocp-Apim-Subscription-Key", cfg.api_key.as_str())]).await?;
            let p = match v
                .get("localBusinesses")
                .and_then(|l| l.get("value"))
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
            {
                Some(p) => p,
                None => return Ok(None),
            };
            let addr = p.get("address").map(|a| {
                [
                    a.get("addressLine").and_then(|x| x.as_str()),
                    a.get("addressLocality").and_then(|x| x.as_str()),
                    a.get("addressRegion").and_then(|x| x.as_str()),
                    a.get("postalCode").and_then(|x| x.as_str()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ")
            });
            Ok(Some(
                Enriched {
                    name: p.get("name").and_then(|x| x.as_str()).map(String::from),
                    address: addr.filter(|a| !a.is_empty()),
                    phone: p
                        .get("phoneNumber")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    website: p.get("url").and_then(|x| x.as_str()).map(String::from),
                    lat: p
                        .get("geo")
                        .and_then(|g| g.get("latitude"))
                        .and_then(|x| x.as_f64()),
                    lng: p
                        .get("geo")
                        .and_then(|g| g.get("longitude"))
                        .and_then(|x| x.as_f64()),
                    rating: p
                        .get("rating")
                        .and_then(|r| r.get("ratingValue"))
                        .and_then(|x| x.as_f64()),
                    confidence: 0.0,
                }
                .score(),
            ))
        }
        "google_cse" => {
            // Google Programmable Search: the engine id (cx) comes from the key's
            // base_url, or metadata.cx.
            let cx = cfg
                .base_url
                .clone()
                .or_else(|| {
                    cfg.metadata
                        .get("cx")
                        .and_then(|c| c.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            if cx.trim().is_empty() {
                return Err(
                    "google_cse key has no search-engine id — set base_url or metadata.cx"
                        .to_string(),
                );
            }
            let url = format!(
                "{}?key={}&cx={}&num=1&q={}",
                GOOGLE_CSE_BASE,
                cfg.api_key,
                urlencoding(&cx),
                q
            );
            let v = http_json(&url, &[]).await?;
            if let Some(err) = v.get("error") {
                return Err(format!(
                    "google_cse: {}",
                    err.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("error")
                ));
            }
            let item = match v
                .get("items")
                .and_then(|i| i.as_array())
                .and_then(|a| a.first())
            {
                Some(i) => i,
                None => return Ok(None),
            };
            let snippet = item
                .get("snippet")
                .and_then(|x| x.as_str())
                .unwrap_or_default();
            let title = item.get("title").and_then(|x| x.as_str());
            Ok(Some(
                Enriched {
                    name: title.map(String::from),
                    address: None,
                    phone: phone_from_text(snippet),
                    website: item.get("link").and_then(|x| x.as_str()).map(String::from),
                    lat: None,
                    lng: None,
                    rating: None,
                    confidence: 0.0,
                }
                .score(),
            ))
        }
        other => Err(format!("unsupported search provider '{}'", other)),
    }
}

fn urlencoding(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "+")
        .replace('&', "%26")
        .replace('?', "%3F")
        .replace('#', "%23")
        .replace('\n', " ")
}

// ─────────────────────────────────────────────────────────────────────────────
// The cycle
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CycleOutcome {
    pub status: String,
    pub provider: Option<String>,
    pub provider_label: Option<String>,
    pub directory_id: Option<Uuid>,
    pub scanned: usize,
    pub matched: usize,
    pub updated: usize,
    pub errors: usize,
    pub message: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct Candidate {
    id: Uuid,
    directory_id: Option<Uuid>,
    name: String,
    city: Option<String>,
    state: Option<String>,
    zip: Option<String>,
    phone: Option<String>,
    website: Option<String>,
    address: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

fn build_query(b: &Candidate) -> String {
    let mut parts = vec![b.name.clone()];
    if let Some(c) = b.city.as_ref().filter(|s| !s.is_empty()) {
        parts.push(c.clone());
    }
    if let Some(s) = b.state.as_ref().filter(|x| !x.is_empty()) {
        parts.push(s.clone());
    }
    if let Some(z) = b.zip.as_ref().filter(|x| !x.is_empty()) {
        parts.push(z.clone());
    }
    parts.join(" ")
}

async fn write_log(
    db: &sqlx::PgPool,
    business_id: Option<Uuid>,
    directory_id: Option<Uuid>,
    source: &str,
    enrichment_type: &str,
    data_before: Option<&Value>,
    data_after: Option<&Value>,
    confidence: Option<f64>,
    status: &str,
    error_message: Option<&str>,
) {
    // A failed log write must be visible but must not abort the cycle.
    let res = sqlx::query(
        "INSERT INTO data_enrichment_logs \
         (business_id, directory_id, source, enrichment_type, data_before, data_after, confidence, status, error_message) \
         VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, $7, $8, $9)",
    )
    .bind(business_id)
    .bind(directory_id)
    .bind(source)
    .bind(enrichment_type)
    .bind(data_before)
    .bind(data_after)
    .bind(confidence)
    .bind(status)
    .bind(error_message)
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("[enrich] could not write data_enrichment_logs row: {}", e);
    }
}

async fn mark_settings_run(
    db: &sqlx::PgPool,
    settings_id: Option<Uuid>,
    status: &str,
    cadence_hours: i32,
) {
    let next = Utc::now() + Duration::hours(cadence_hours.max(1) as i64);
    let res = match settings_id {
        Some(id) => {
            sqlx::query(
                "UPDATE enrichment_settings SET last_run_at = now(), last_status = $1, next_run_at = $2, updated_at = now() WHERE id = $3",
            )
            .bind(status)
            .bind(next)
            .bind(id)
            .execute(db)
            .await
            .map(|_| ())
        }
        None => Ok(()),
    };
    if let Err(e) = res {
        tracing::warn!("[enrich] could not update enrichment_settings: {}", e);
    }
}

/// Run one enrichment cycle. `directory_id` scopes it; `batch` overrides the
/// configured batch size for a manual run. Never panics: every failure mode ends
/// as a recorded status.
pub async fn run_cycle(
    db: &sqlx::PgPool,
    directory_id: Option<Uuid>,
    batch_override: Option<i32>,
    id_override: Option<Uuid>,
) -> Result<CycleOutcome, sqlx::Error> {
    let started = Utc::now();
    let settings = effective_settings(db, directory_id).await?;
    let settings_id = id_override.or(settings.id);
    let batch = batch_override.unwrap_or(settings.batch_size).clamp(1, 500);

    let provider = resolve_provider(db, settings.provider.as_deref()).await?;

    let Some(cfg) = provider else {
        let msg = "No search provider configured — add a Google Places / SerpAPI / Bing / Google CSE key in the admin Provider API Keys card. Nothing was enriched.";
        tracing::warn!("[enrich] {}", msg);
        write_log(
            db,
            None,
            directory_id,
            "none",
            "cycle",
            None,
            None,
            None,
            "skipped",
            Some(msg),
        )
        .await;
        mark_settings_run(
            db,
            settings_id,
            "skipped:no_provider",
            settings.cadence_hours,
        )
        .await;
        return Ok(CycleOutcome {
            status: "skipped_no_provider".to_string(),
            provider: None,
            provider_label: None,
            directory_id,
            scanned: 0,
            matched: 0,
            updated: 0,
            errors: 0,
            message: msg.to_string(),
            started_at: started,
            finished_at: Utc::now(),
        });
    };

    // Rotating selection: least recently enriched first.
    let candidates = sqlx::query_as::<_, Candidate>(
        "SELECT id, directory_id, name, city, state, zip, phone, website, address, latitude, longitude \
         FROM businesses \
         WHERE COALESCE(status, 'active') = 'active' AND COALESCE(is_active, true) = true \
           AND ($1::uuid IS NULL OR directory_id = $1) \
         ORDER BY enriched_at ASC NULLS FIRST, id ASC \
         LIMIT $2",
    )
    .bind(directory_id)
    .bind(batch)
    .fetch_all(db)
    .await?;

    let mut matched = 0usize;
    let mut updated = 0usize;
    let mut errors = 0usize;

    for b in &candidates {
        let query = build_query(b);
        let before = json!({
            "name": b.name,
            "address": b.address,
            "phone": b.phone,
            "website": b.website,
            "latitude": b.latitude,
            "longitude": b.longitude,
        });

        match provider_search(&cfg, &query).await {
            Err(e) => {
                errors += 1;
                tracing::warn!(
                    "[enrich] provider {} failed for business {}: {}",
                    cfg.provider,
                    b.id,
                    e
                );
                write_log(
                    db,
                    Some(b.id),
                    b.directory_id.or(directory_id),
                    &cfg.provider,
                    "scheduled_re_enrichment",
                    Some(&before),
                    None,
                    None,
                    "error",
                    Some(&e),
                )
                .await;
                // The provider itself is failing: stop the cycle instead of burning
                // the whole batch against a broken key.
                break;
            }
            Ok(None) => {
                write_log(
                    db,
                    Some(b.id),
                    b.directory_id.or(directory_id),
                    &cfg.provider,
                    "scheduled_re_enrichment",
                    Some(&before),
                    None,
                    None,
                    "no_match",
                    None,
                )
                .await;
                // Advance the rotation even on a miss, or the same head-of-list
                // would be retried forever.
                let _ = sqlx::query("UPDATE businesses SET enriched_at = now() WHERE id = $1")
                    .bind(b.id)
                    .execute(db)
                    .await;
            }
            Ok(Some(found)) => {
                matched += 1;
                let after = json!({
                    "name": found.name,
                    "address": found.address,
                    "phone": found.phone,
                    "website": found.website,
                    "latitude": found.lat,
                    "longitude": found.lng,
                    "rating": found.rating,
                });

                // Fill blanks only — never overwrite what the owner or an admin typed.
                let mut did_update = false;
                if b.address.is_none() || b.address.as_deref() == Some("") {
                    if let Some(v) = found.address.as_ref() {
                        let _ = sqlx::query("UPDATE businesses SET address = $1 WHERE id = $2")
                            .bind(v)
                            .bind(b.id)
                            .execute(db)
                            .await;
                        did_update = true;
                    }
                }
                if b.phone.is_none() || b.phone.as_deref() == Some("") {
                    if let Some(v) = found.phone.as_ref() {
                        let _ = sqlx::query("UPDATE businesses SET phone = $1 WHERE id = $2")
                            .bind(v)
                            .bind(b.id)
                            .execute(db)
                            .await;
                        did_update = true;
                    }
                }
                if b.website.is_none() || b.website.as_deref() == Some("") {
                    if let Some(v) = found.website.as_ref() {
                        let _ = sqlx::query("UPDATE businesses SET website = $1 WHERE id = $2")
                            .bind(v)
                            .bind(b.id)
                            .execute(db)
                            .await;
                        did_update = true;
                    }
                }
                if (b.latitude.is_none() || b.longitude.is_none())
                    && found.lat.is_some()
                    && found.lng.is_some()
                {
                    let _ = sqlx::query(
                        "UPDATE businesses SET latitude = $1, longitude = $2 WHERE id = $3",
                    )
                    .bind(found.lat)
                    .bind(found.lng)
                    .bind(b.id)
                    .execute(db)
                    .await;
                    did_update = true;
                }
                if did_update {
                    updated += 1;
                }

                let _ = sqlx::query("UPDATE businesses SET enriched_at = now() WHERE id = $1")
                    .bind(b.id)
                    .execute(db)
                    .await;

                write_log(
                    db,
                    Some(b.id),
                    b.directory_id.or(directory_id),
                    &cfg.provider,
                    "scheduled_re_enrichment",
                    Some(&before),
                    Some(&after),
                    Some(found.confidence),
                    "completed",
                    None,
                )
                .await;
            }
        }
    }

    let status = if errors > 0 {
        "error"
    } else if candidates.is_empty() {
        "ok:idle"
    } else {
        "ok"
    };
    let message = if errors > 0 {
        format!(
            "{} scanned, {} matched, {} updated, {} provider error(s) — see the log entries",
            candidates.len(),
            matched,
            updated,
            errors
        )
    } else if candidates.is_empty() {
        "No businesses needed enrichment for this scope.".to_string()
    } else {
        format!(
            "{} scanned, {} matched, {} updated via {}",
            candidates.len(),
            matched,
            updated,
            cfg.provider
        )
    };

    mark_settings_run(db, settings_id, status, settings.cadence_hours).await;
    tracing::info!("[enrich] cycle {}: {}", cfg.provider, message);

    Ok(CycleOutcome {
        status: status.to_string(),
        provider: Some(cfg.provider),
        provider_label: Some(cfg.label),
        directory_id,
        scanned: candidates.len(),
        matched,
        updated,
        errors,
        message,
        started_at: started,
        finished_at: Utc::now(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Background scheduler — the thing that was missing (only a comment existed)
// ─────────────────────────────────────────────────────────────────────────────

pub fn start_enrichment_scheduler(db: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
        loop {
            interval.tick().await;
            let due = sqlx::query_as::<_, (Uuid, Option<Uuid>, i32)>(
                "SELECT id, directory_id, batch_size FROM enrichment_settings \
                 WHERE is_enabled = true AND (next_run_at IS NULL OR next_run_at <= now())",
            )
            .fetch_all(&db)
            .await;

            let due = match due {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(
                        "[enrich] scheduler could not read enrichment_settings: {}",
                        e
                    );
                    continue;
                }
            };

            for (id, directory_id, batch) in due {
                tracing::info!(
                    "[enrich] settings {} due — running cycle (scope {:?})",
                    id,
                    directory_id
                );
                match run_cycle(&db, directory_id, Some(batch), Some(id)).await {
                    Ok(o) => tracing::info!("[enrich] auto cycle {}: {}", o.status, o.message),
                    Err(e) => tracing::warn!("[enrich] auto cycle failed: {}", e),
                }
            }
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP surface
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SettingsQuery {
    pub directory_id: Option<Uuid>,
}

pub async fn get_enrichment_settings(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<SettingsQuery>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to view enrichment settings".to_string(),
        ));
    }
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let configured = configured_search_providers(&s.db).await?;
    let resolved = resolve_provider(&s.db, settings.provider.as_deref()).await?;

    let directories = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directories ORDER BY name ASC LIMIT 500",
    )
    .fetch_all(&s.db)
    .await?;

    let providers: Vec<Value> = configured
        .iter()
        .map(|c| json!({ "provider": c.provider, "label": c.label }))
        .collect();

    Ok(Json(json!({
        "settings": settings,
        "directories": directories
            .into_iter()
            .map(|(id, name, slug)| json!({ "id": id, "name": name, "slug": slug }))
            .collect::<Vec<_>>(),
        "supported_adapters": SEARCH_ADAPTERS,
        "configured_providers": providers,
        "active_provider": resolved.as_ref().map(|c| c.provider.clone()),
        "active_provider_label": resolved.as_ref().map(|c| c.label.clone()),
        "due": settings.is_enabled
            && settings
                .next_run_at
                .map(|n| n <= Utc::now())
                .unwrap_or(true),
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub directory_id: Option<Uuid>,
    pub is_enabled: Option<bool>,
    pub cadence_hours: Option<i32>,
    pub batch_size: Option<i32>,
    /// null/omitted = keep; "" = clear the pin (auto-detect)
    pub provider: Option<String>,
}

pub async fn update_enrichment_settings(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<UpdateSettingsRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to change enrichment settings".to_string(),
        ));
    }

    let current = effective_settings(&s.db, req.directory_id).await?;
    let is_enabled = req.is_enabled.unwrap_or(current.is_enabled);
    let cadence = req.cadence_hours.unwrap_or(current.cadence_hours).max(1);
    let batch = req.batch_size.unwrap_or(current.batch_size).clamp(1, 500);
    let provider = match req.provider.clone() {
        Some(p) if p.trim().is_empty() => None,
        Some(p) => Some(p),
        None => current.provider.clone(),
    };
    if let Some(ref p) = provider {
        if !SEARCH_ADAPTERS.contains(&p.as_str()) {
            // Allow it (a future adapter), but make the mismatch explicit.
            tracing::warn!(
                "[enrich] provider '{}' pinned but not a known search adapter",
                p
            );
        }
    }

    let next_run = if is_enabled {
        Some(Utc::now() + Duration::hours(cadence as i64))
    } else {
        None
    };

    // One row per directory scope: update in place, or create the override row.
    let existing_id = if req.directory_id.is_some() {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM enrichment_settings WHERE directory_id = $1 ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(req.directory_id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM enrichment_settings WHERE directory_id IS NULL ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(&s.db)
        .await?
    };

    let row = match existing_id {
        Some(id) => sqlx::query_as::<_, EnrichmentSettings>(
            "UPDATE enrichment_settings SET is_enabled = $1, cadence_hours = $2, batch_size = $3, \
             provider = $4, next_run_at = $5, updated_at = now() WHERE id = $6 RETURNING *",
        )
        .bind(is_enabled)
        .bind(cadence)
        .bind(batch)
        .bind(provider.clone())
        .bind(next_run)
        .bind(id)
        .fetch_one(&s.db)
        .await?,
        None => sqlx::query_as::<_, EnrichmentSettings>(
            "INSERT INTO enrichment_settings (directory_id, is_enabled, cadence_hours, batch_size, provider, next_run_at) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        )
        .bind(req.directory_id)
        .bind(is_enabled)
        .bind(cadence)
        .bind(batch)
        .bind(provider)
        .bind(next_run)
        .fetch_one(&s.db)
        .await?,
    };

    Ok(Json(json!(row)))
}

pub async fn enrichment_status(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<SettingsQuery>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to view enrichment status".to_string(),
        ));
    }
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let resolved = resolve_provider(&s.db, settings.provider.as_deref()).await?;
    let configured = configured_search_providers(&s.db).await?;

    // The card's pickers: every directory (scope selector) and every provider that
    // actually has a key right now.
    let directories = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directories ORDER BY name ASC LIMIT 500",
    )
    .fetch_all(&s.db)
    .await?;

    let logs = sqlx::query_as::<
        _,
        (
            Uuid,
            Option<Uuid>,
            String,
            String,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
        ),
    >(
        "SELECT id, business_id, source, enrichment_type, status, error_message, created_at \
         FROM data_enrichment_logs ORDER BY created_at DESC LIMIT 25",
    )
    .fetch_all(&s.db)
    .await?;

    let totals = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT count(*) FILTER (WHERE status = 'completed') AS completed, \
                count(*) FILTER (WHERE status = 'no_match') AS no_match, \
                count(*) FILTER (WHERE status = 'error') AS errors, \
                count(*) FILTER (WHERE status = 'skipped') AS skipped \
         FROM data_enrichment_logs",
    )
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "settings": settings,
        "directories": directories
            .into_iter()
            .map(|(id, name, slug)| json!({ "id": id, "name": name, "slug": slug }))
            .collect::<Vec<_>>(),
        "configured_providers": configured
            .iter()
            .map(|c| json!({ "provider": c.provider, "label": c.label }))
            .collect::<Vec<_>>(),
        "active_provider": resolved.as_ref().map(|c| c.provider.clone()),
        "active_provider_label": resolved.as_ref().map(|c| c.label.clone()),
        "supported_adapters": SEARCH_ADAPTERS,
        "recent_logs": logs
            .into_iter()
            .map(|(id, business_id, source, enrichment_type, status, error_message, created_at)| json!({
                "id": id, "business_id": business_id, "source": source,
                "enrichment_type": enrichment_type, "status": status,
                "error_message": error_message, "created_at": created_at,
            }))
            .collect::<Vec<_>>(),
        "totals": {
            "completed": totals.0, "no_match": totals.1, "errors": totals.2, "skipped": totals.3,
        },
    })))
}

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub directory_id: Option<Uuid>,
    pub batch_size: Option<i32>,
}

pub async fn run_enrichment_now(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    body: Option<Json<RunRequest>>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to run enrichment".to_string(),
        ));
    }
    let req = body.map(|Json(b)| b).unwrap_or(RunRequest {
        directory_id: None,
        batch_size: None,
    });
    let outcome = run_cycle(&s.db, req.directory_id, req.batch_size, None).await?;
    Ok(Json(json!(outcome)))
}
