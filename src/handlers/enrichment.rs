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
//!   * with NO paid key configured the cycle still runs on the free/open fallback
//!     (OpenStreetMap Nominatim, card B79) instead of skipping — enrichment without
//!     paid APIs, and an admin may pin that adapter explicitly;
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

use crate::auth::middleware::is_super_admin;
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::security::provider_key_crypto as keycrypto;
use crate::AppState;

/// Adapters that must be EXPLICITLY pinned before they run (card B98). Apify scrapes on a
/// metered, pay-per-use platform, so a live key must never be auto-selected as the rotating
/// enrichment provider — the admin switches it on by pinning it in the Data Enrichment card.
const OPT_IN_ADAPTERS: [&str; 1] = ["apify"];
/// Free / open-data adapters (card B79). They need NO API key, so they never appear in
/// `provider_keys`; they are the zero-cost fallback. An admin may PIN one, and when no paid
/// provider is configured the cycle falls back to the first of these, so enrichment still
/// runs (and fills the gaps a paid listing misses) without a paid key.
const FREE_ADAPTERS: [&str; 1] = ["openstreetmap"];
/// Every adapter this build speaks: the auto-eligible set plus the opt-in set plus the free
/// set. Kept as an explicit list (not a concat) so the three halves stay visible and
/// independent.
const ALL_ADAPTERS: [&str; 6] = [
    "google_places",
    "serpapi",
    "bing",
    "google_cse",
    "apify",
    "openstreetmap",
];

/// B83 part 1 — the catalogue the **Sources** panel renders: one row per adapter with a
/// plain-English description and whether it needs a key. `enabled` is computed per scope at
/// read time (see `source_enabled`), so a keyless source is on out of the box and a keyed
/// source switches itself on once its key is saved.
const SOURCE_CATALOG: [(&str, &str, &str, bool); 6] = [
    (
        "google_places",
        "Google Places",
        "The most complete business data — address, phone, hours, rating. Needs a Google Places API key.",
        true,
    ),
    (
        "openstreetmap",
        "OpenStreetMap (free)",
        "A free, open map of businesses that needs no key. The built-in zero-config fallback and good for filling gaps.",
        false,
    ),
    (
        "serpapi",
        "SerpAPI",
        "Google Maps search results through SerpAPI. Needs a SerpAPI key.",
        true,
    ),
    (
        "bing",
        "Bing Places",
        "Business listings from Bing. Needs a Bing Maps key.",
        true,
    ),
    (
        "google_cse",
        "Google Custom Search",
        "Finds a business's website and phone from web results. Needs a Google CSE key and engine id.",
        true,
    ),
    (
        "apify",
        "Apify (metered, pay-per-use)",
        "A pay-per-use scraper that can return rich Google Maps records. Add its token, then switch it on here.",
        true,
    ),
];

const DEFAULT_GOOGLE_PLACES_BASE: &str = "https://maps.googleapis.com/maps/api/place";
const SERPAPI_BASE: &str = "https://serpapi.com";
const BING_BASE: &str = "https://api.bing.microsoft.com";
const GOOGLE_CSE_BASE: &str = "https://www.googleapis.com/customsearch/v1";
const APIFY_BASE: &str = "https://api.apify.com";
/// The public Apify actor used when the key's metadata names no `actor_id`. It returns Google
/// Maps place records, which is exactly the shape the merge contract expects.
const DEFAULT_APIFY_ACTOR: &str = "compass/crawler-google-places";
/// OpenStreetMap's Nominatim geocoder — free, no key, no metering (card B79). Its usage policy
/// requires an identifying User-Agent; the rotating cycle is low-volume and honours that.
const OSM_NOMINATIM_BASE: &str = "https://nominatim.openstreetmap.org";
const OSM_USER_AGENT: &str = "ZaarHub-MultiDirectory/1.0 (+https://zaarhub.com)";

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
    /// B83: when true (the default) the rotating cycle only touches UNCLAIMED listings, so an
    /// owner's own data is never overwritten by automation.
    pub unclaimed_only: bool,
    /// B83 part 1: the admin's explicit per-source on/off set (the Sources panel checkboxes).
    /// NULL = use the defaults (keyless sources on; a keyed source on once its key is saved).
    pub enabled_sources: Option<Vec<String>>,
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
            unclaimed_only: true,
            enabled_sources: None,
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
        "SELECT provider, label, api_key, \
                base_url, metadata \
         FROM provider_keys \
         WHERE is_active = true AND provider = ANY($1) \
         ORDER BY is_default DESC, updated_at DESC",
    )
    .bind(&ALL_ADAPTERS[..])
    .fetch_all(db)
    .await?;

    // `api_key` is enc:v1 ciphertext at rest; it is decrypted here with the env-only master
    // key. A row that cannot be decrypted is treated as unconfigured (log + skip) rather than
    // handed to an adapter as ciphertext.
    let mut out = Vec::with_capacity(rows.len());
    for (provider, label, stored, base_url, metadata) in rows {
        let Some(api_key) = keycrypto::decrypt_for_use(db, &stored, &provider).await else {
            continue;
        };
        if api_key.trim().is_empty() {
            continue;
        }
        out.push(ProviderCfg {
            provider,
            label,
            api_key,
            base_url,
            metadata: metadata.unwrap_or_else(|| json!({})),
        });
    }
    Ok(out)
}

/// A synthetic config for a free / open-data adapter (card B79). No `provider_keys` row is
/// needed or expected: the adapter is keyless, so the base URL is the only configuration.
fn free_provider_cfg(provider: &str) -> ProviderCfg {
    let (label, base) = match provider {
        "openstreetmap" => ("OpenStreetMap (free)", OSM_NOMINATIM_BASE),
        other => (other, ""),
    };
    ProviderCfg {
        provider: provider.to_string(),
        label: label.to_string(),
        api_key: String::new(),
        base_url: Some(base.to_string()),
        metadata: json!({}),
    }
}

/// Is a source switched on for this scope (B83 part 1)?
///
/// An explicit `enabled_sources` list from the Sources panel is authoritative. With no explicit
/// choice (NULL) the defaults apply: a keyless/free source is on, and a keyed source switches
/// itself on automatically once its key is saved — so a directory works out of the box.
fn source_enabled(
    settings: &EnrichmentSettings,
    provider: &str,
    available: &[ProviderCfg],
) -> bool {
    if let Some(list) = settings.enabled_sources.as_ref() {
        return list.iter().any(|s| s == provider);
    }
    if FREE_ADAPTERS.contains(&provider) {
        return true;
    }
    available.iter().any(|c| c.provider == provider)
}

/// The Sources panel catalogue for one scope: every adapter with its plain description, whether
/// it needs a key, whether a key is configured right now, and whether it is switched on.
fn source_catalog_json(settings: &EnrichmentSettings, available: &[ProviderCfg]) -> Vec<Value> {
    SOURCE_CATALOG
        .iter()
        .map(|(provider, label, description, needs_key)| {
            let key_configured = available.iter().any(|c| c.provider == *provider);
            json!({
                "provider": provider,
                "label": label,
                "description": description,
                "needs_key": needs_key,
                "key_configured": key_configured,
                "opt_in": OPT_IN_ADAPTERS.contains(provider),
                "enabled": source_enabled(settings, provider, available),
            })
        })
        .collect()
}

/// Which adapter runs: the pinned provider when it is configured (or free) AND switched on,
/// else the first enabled configured one (preferring the marked default), else the enabled
/// free/open fallback (card B79) so enrichment still runs with no paid key. None only when
/// there is nothing at all to run — including when the admin has switched every source off.
async fn resolve_provider(
    db: &sqlx::PgPool,
    settings: &EnrichmentSettings,
) -> Result<Option<ProviderCfg>, sqlx::Error> {
    let available = configured_search_providers(db).await?;
    if let Some(pin) = settings.provider.as_deref() {
        if source_enabled(settings, pin, &available) {
            if let Some(cfg) = available.iter().find(|c| c.provider == pin) {
                return Ok(Some(cfg.clone()));
            }
            // A free adapter has no key row; a pin on it resolves straight from code.
            if FREE_ADAPTERS.contains(&pin) {
                return Ok(Some(free_provider_cfg(pin)));
            }
        }
        if !ALL_ADAPTERS.contains(&pin) {
            tracing::warn!(
                "[enrich] configured provider '{}' is not a search adapter this build speaks; falling back to a configured one",
                pin
            );
        }
    }
    // Auto-selection NEVER picks an opt-in, metered adapter (card B98): a configured Apify
    // key stays dormant until the admin pins it explicitly.
    if let Some(cfg) = available.iter().find(|c| {
        source_enabled(settings, &c.provider, &available)
            && !OPT_IN_ADAPTERS.contains(&c.provider.as_str())
    }) {
        return Ok(Some(cfg.clone()));
    }
    // Nothing paid configured (or every paid source switched off) → the free/open fallback
    // (card B79): enrichment without paid APIs, when that source is still switched on.
    Ok(FREE_ADAPTERS
        .iter()
        .find(|p| source_enabled(settings, p, &available))
        .map(|p| free_provider_cfg(p)))
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

/// GET a JSON API and return `(http_status, parsed_body)`.
///
/// Returning the HTTP status (rather than throwing the body away) is what lets a caller report
/// Google's own `status`/`error_message` alongside the transport code (card B78). A non-2xx
/// answer comes back as an `Err` naming the status; a 2xx answer is parsed and handed to the
/// caller, which must still run it through `places_client::check_response`.
async fn http_json(url: &str, headers: &[(&str, &str)]) -> Result<(u16, Value), String> {
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
    let parsed = serde_json::from_str(&body).map_err(|e| {
        format!(
            "json: {} | {}",
            e,
            body.chars().take(160).collect::<String>()
        )
    })?;
    Ok((status.as_u16(), parsed))
}

/// POST a JSON body and parse a JSON answer. Used by the Apify adapter, whose synchronous
/// actor endpoint runs a scraper and returns the dataset items inline. The timeout is longer
/// than a plain lookup because an actor run is real work, not a metadata read.
async fn http_json_post(url: &str, body: &Value) -> Result<(u16, Value), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http client: {}", e))?;
    let resp = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("request: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("body: {}", e))?;
    if !status.is_success() {
        let peek: String = text.chars().take(200).collect();
        return Err(format!("HTTP {}: {}", status.as_u16(), peek));
    }
    let parsed = serde_json::from_str(&text).map_err(|e| {
        format!(
            "json: {} | {}",
            e,
            text.chars().take(160).collect::<String>()
        )
    })?;
    Ok((status.as_u16(), parsed))
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
                "{}/findplacefromtext/json?input={}&inputtype=textquery&fields=place_id,name,formatted_address,geometry,rating&key={}",
                base.trim_end_matches('/'),
                q,
                cfg.api_key
            );
            let (http_status, v) = http_json(&url, &[]).await?;
            // Google answers HTTP 200 even when it refuses: check the body's own `status` and
            // report the HTTP code + Google's status + Google's error message (card B78).
            crate::handlers::places_client::check_response(
                &v,
                http_status,
                crate::handlers::places_client::PlacesCall::FindPlace,
            )
            .map_err(|e| e.to_string())?;
            let status = crate::handlers::places_client::provider_status(&v);
            if status == "ZERO_RESULTS" {
                return Ok(None);
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
            let (_http_status, v) = http_json(&url, &[]).await?;
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
            let (_http_status, v) =
                http_json(&url, &[("Ocp-Apim-Subscription-Key", cfg.api_key.as_str())]).await?;
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
            let (_, v) = http_json(&url, &[]).await?;
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
        "apify" => {
            // Opt-in, METERED source (card B98). A synchronous actor run per business, capped at
            // ONE place by default, so a cycle's spend is bounded by the batch size the admin set.
            // Results flow through the SAME merge contract as every other source: they may only
            // fill EMPTY fields and can never overwrite a value the owner or an admin typed.
            // Suppliers are never produced here — an existing business listing is gap-filled only.
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| APIFY_BASE.to_string());
            let actor = cfg
                .metadata
                .get("actor_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_APIFY_ACTOR);
            let max_items = cfg
                .metadata
                .get("max_items_per_query")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .clamp(1, 25);
            let url = format!(
                "{}/v2/acts/{}/run-sync-get-dataset-items?token={}",
                base.trim_end_matches('/'),
                // Apify addresses a public actor as `username~actor` in a URL path: a literal
                // `/` adds a path segment and the API answers 404 (proven 2026-10-03).
                actor.replace('/', "~"),
                cfg.api_key
            );
            let body = json!({
                "searchStringsArray": [query.trim()],
                "maxCrawledPlacesPerSearch": max_items,
                "language": "en",
                "skipClosedPlaces": false
            });
            let (_http_status, v) = http_json_post(&url, &body).await?;
            let item = match v.as_array().and_then(|a| a.first()) {
                Some(i) => i,
                None => return Ok(None),
            };
            Ok(Some(
                Enriched {
                    name: item.get("title").and_then(|x| x.as_str()).map(String::from),
                    address: item
                        .get("address")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    phone: item.get("phone").and_then(|x| x.as_str()).map(String::from),
                    website: item
                        .get("website")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    lat: item
                        .get("location")
                        .and_then(|l| l.get("lat"))
                        .and_then(|x| x.as_f64()),
                    lng: item
                        .get("location")
                        .and_then(|l| l.get("lng"))
                        .and_then(|x| x.as_f64()),
                    rating: item.get("totalScore").and_then(|x| x.as_f64()),
                    confidence: 0.0,
                }
                .score(),
            ))
        }
        "openstreetmap" => {
            // Free / open data (card B79): OpenStreetMap's Nominatim geocoder. No key, no
            // metering, so it is the zero-cost fallback. `extratags=1` surfaces phone/website
            // OSM tags the way the paid adapters' responses do, and the merge contract still
            // only ever fills EMPTY fields — OSM never overwrites what a human typed.
            let base = cfg
                .base_url
                .clone()
                .unwrap_or_else(|| OSM_NOMINATIM_BASE.to_string());
            let url = format!(
                "{}/search?q={}&format=jsonv2&addressdetails=1&extratags=1&limit=1",
                base.trim_end_matches('/'),
                q
            );
            let (_http_status, v) = http_json(
                &url,
                &[
                    ("User-Agent", OSM_USER_AGENT),
                    ("Accept-Language", "en-US,en"),
                ],
            )
            .await?;
            let p = match v.as_array().and_then(|a| a.first()) {
                Some(p) => p,
                None => return Ok(None),
            };
            let addr = p.get("address");
            let pick_addr =
                |k: &str| -> Option<&str> { addr.and_then(|a| a.get(k)).and_then(|x| x.as_str()) };
            let street = [pick_addr("house_number"), pick_addr("road")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            let mut parts: Vec<&str> = Vec::new();
            if !street.is_empty() {
                parts.push(street.as_str());
            }
            for k in ["city", "town", "village", "hamlet", "state", "postcode"] {
                if let Some(s) = pick_addr(k) {
                    if !s.trim().is_empty() {
                        parts.push(s);
                    }
                }
            }
            let address = if parts.is_empty() {
                p.get("display_name")
                    .and_then(|x| x.as_str())
                    .map(String::from)
            } else {
                Some(parts.join(", "))
            };
            let et = p.get("extratags");
            let pick_et = |keys: &[&str]| -> Option<String> {
                keys.iter().find_map(|k| {
                    et.and_then(|e| e.get(*k))
                        .and_then(|x| x.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                })
            };
            Ok(Some(
                Enriched {
                    name: p
                        .get("name")
                        .and_then(|x| x.as_str())
                        .map(String::from)
                        .or_else(|| {
                            p.get("display_name")
                                .and_then(|x| x.as_str())
                                .and_then(|d| d.split(',').next())
                                .map(|s| s.trim().to_string())
                        }),
                    address,
                    phone: pick_et(&["phone", "contact:phone"]),
                    website: pick_et(&["website", "contact:website", "url"]),
                    lat: p
                        .get("lat")
                        .and_then(|x| x.as_str())
                        .and_then(|s| s.parse().ok()),
                    lng: p
                        .get("lon")
                        .and_then(|x| x.as_str())
                        .and_then(|s| s.parse().ok()),
                    rating: None,
                    confidence: 0.0,
                }
                .score(),
            ))
        }
        other => Err(format!("unsupported search provider '{}'", other)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Candidate search (card B80 prospecting) — free-text, N results
// ─────────────────────────────────────────────────────────────────────────────

/// One external candidate returned by the free/open prospecting search.
#[derive(Debug, Serialize)]
pub struct ExternalCandidate {
    pub name: String,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

/// Free-text candidate search across the free/open enrichment source (card B80). Uses the SAME
/// source as the listing pipeline's zero-cost fallback — OpenStreetMap/Nominatim (card B79) — which
/// needs no key, so supplier prospecting always works with nothing configured. Returns up to
/// `limit` candidates (clamped 1..50). A network failure is a plain Err the caller reports honestly.
pub async fn search_external_candidates(
    query: &str,
    limit: i64,
) -> Result<Vec<ExternalCandidate>, String> {
    let q = urlencoding(query.trim());
    if q.trim().is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 50);
    let url = format!(
        "{}/search?q={}&format=jsonv2&addressdetails=1&extratags=1&limit={}",
        OSM_NOMINATIM_BASE, q, limit
    );
    let (_http_status, v) = http_json(
        &url,
        &[
            ("User-Agent", OSM_USER_AGENT),
            ("Accept-Language", "en-US,en"),
        ],
    )
    .await?;
    let mut out = Vec::new();
    for p in v.as_array().cloned().unwrap_or_default() {
        let addr = p.get("address");
        let pick_addr = |k: &str| -> Option<String> {
            addr.and_then(|a| a.get(k))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        };
        let street = [pick_addr("house_number"), pick_addr("road")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let mut parts: Vec<String> = Vec::new();
        if !street.trim().is_empty() {
            parts.push(street);
        }
        for k in ["city", "town", "village", "hamlet", "state", "postcode"] {
            if let Some(s) = pick_addr(k) {
                if !s.trim().is_empty() {
                    parts.push(s);
                }
            }
        }
        let address = if parts.is_empty() {
            p.get("display_name")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        } else {
            Some(parts.join(", "))
        };
        let et = p.get("extratags");
        let pick_et = |keys: &[&str]| -> Option<String> {
            keys.iter().find_map(|k| {
                et.and_then(|e| e.get(*k))
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
        };
        let name = p
            .get("name")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                p.get("display_name")
                    .and_then(|x| x.as_str())
                    .and_then(|d| d.split(',').next())
                    .map(|s| s.trim().to_string())
            });
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            continue;
        };
        out.push(ExternalCandidate {
            name,
            address,
            phone: pick_et(&["phone", "contact:phone"]),
            website: pick_et(&["website", "contact:website", "url"]),
            lat: p
                .get("lat")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse().ok()),
            lng: p
                .get("lon")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse().ok()),
        });
    }
    Ok(out)
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
    /// B83 part 3: matched but nothing was written because every field was already filled — a
    /// clean "checked, nothing to change" count so a bulk refresh reports honestly and is
    /// idempotent (re-running changes nothing and says so).
    pub unchanged: usize,
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

    let provider = resolve_provider(db, &settings).await?;

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
            unchanged: 0,
            errors: 0,
            message: msg.to_string(),
            started_at: started,
            finished_at: Utc::now(),
        });
    };

    // Rotating selection: least recently enriched first. When `unclaimed_only` is on (the
    // default), any business that a real owner has claimed is EXCLUDED — automation must never
    // overwrite an owner's own data.
    let candidates = sqlx::query_as::<_, Candidate>(
        "SELECT b.id, b.directory_id, b.name, b.city, b.state, b.zip, b.phone, b.website, b.address, b.latitude, b.longitude \
         FROM businesses b \
         WHERE COALESCE(b.status, 'active') = 'active' AND COALESCE(b.is_active, true) = true \
           AND ($1::uuid IS NULL OR b.directory_id = $1) \
           AND (NOT $3::boolean OR NOT EXISTS ( \
                 SELECT 1 FROM claimed_businesses cb \
                 WHERE cb.business_id = b.id AND COALESCE(cb.is_active, true) = true)) \
         ORDER BY b.enriched_at ASC NULLS FIRST, b.id ASC \
         LIMIT $2",
    )
    .bind(directory_id)
    .bind(batch)
    .bind(settings.unclaimed_only)
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
    let unchanged = matched.saturating_sub(updated);
    let message = if errors > 0 {
        format!(
            "{} scanned, {} matched, {} updated, {} unchanged, {} provider error(s) — see the log entries",
            candidates.len(),
            matched,
            updated,
            unchanged,
            errors
        )
    } else if candidates.is_empty() {
        "No businesses needed enrichment for this scope.".to_string()
    } else {
        format!(
            "{} scanned, {} matched, {} updated, {} unchanged (already complete) via {}",
            candidates.len(),
            matched,
            updated,
            unchanged,
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
        unchanged,
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
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to view enrichment settings".to_string(),
        ));
    }
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let configured = configured_search_providers(&s.db).await?;
    let resolved = resolve_provider(&s.db, &settings).await?;

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
        "settings": &settings,
        "directories": directories
            .into_iter()
            .map(|(id, name, slug)| json!({ "id": id, "name": name, "slug": slug }))
            .collect::<Vec<_>>(),
        "supported_adapters": ALL_ADAPTERS,
        "configured_providers": providers,
        "source_catalog": source_catalog_json(&settings, &configured),
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
    /// B83: when true the cycle only touches UNCLAIMED listings (claimed = owner-managed, never
    /// overwritten by automation). Omitted = keep the current value.
    pub unclaimed_only: Option<bool>,
    /// B83 part 1: the Sources panel checkbox set — the enabled source list for this scope.
    /// Omitted = keep the current value; an explicit list (including an empty one) replaces it.
    pub sources: Option<Vec<String>>,
}

pub async fn update_enrichment_settings(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<UpdateSettingsRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to change enrichment settings".to_string(),
        ));
    }

    let current = effective_settings(&s.db, req.directory_id).await?;
    let is_enabled = req.is_enabled.unwrap_or(current.is_enabled);
    let cadence = req.cadence_hours.unwrap_or(current.cadence_hours).max(1);
    let batch = req.batch_size.unwrap_or(current.batch_size).clamp(1, 500);
    let unclaimed_only = req.unclaimed_only.unwrap_or(current.unclaimed_only);
    let provider = match req.provider.clone() {
        Some(p) if p.trim().is_empty() => None,
        Some(p) => Some(p),
        None => current.provider.clone(),
    };
    if let Some(ref p) = provider {
        if !ALL_ADAPTERS.contains(&p.as_str()) {
            // Allow it (a future adapter), but make the mismatch explicit.
            tracing::warn!(
                "[enrich] provider '{}' pinned but not a known search adapter",
                p
            );
        }
    }
    // B83 part 1: the Sources panel checkbox set. Omitted = keep; an explicit list (even an
    // empty one) replaces it, filtered to adapters this build actually speaks so a stale value
    // cannot silently persist.
    let enabled_sources: Option<Vec<String>> = match req.sources.clone() {
        Some(list) => Some(
            list.into_iter()
                .filter(|p| ALL_ADAPTERS.contains(&p.as_str()))
                .collect(),
        ),
        None => current.enabled_sources.clone(),
    };

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
             provider = $4, unclaimed_only = $5, enabled_sources = $6, next_run_at = $7, \
             updated_at = now() WHERE id = $8 RETURNING *",
        )
        .bind(is_enabled)
        .bind(cadence)
        .bind(batch)
        .bind(provider.clone())
        .bind(unclaimed_only)
        .bind(enabled_sources.clone())
        .bind(next_run)
        .bind(id)
        .fetch_one(&s.db)
        .await?,
        None => sqlx::query_as::<_, EnrichmentSettings>(
            "INSERT INTO enrichment_settings (directory_id, is_enabled, cadence_hours, batch_size, provider, unclaimed_only, enabled_sources, next_run_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *",
        )
        .bind(req.directory_id)
        .bind(is_enabled)
        .bind(cadence)
        .bind(batch)
        .bind(provider)
        .bind(unclaimed_only)
        .bind(enabled_sources)
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
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to view enrichment status".to_string(),
        ));
    }
    let settings = effective_settings(&s.db, q.directory_id).await?;
    let resolved = resolve_provider(&s.db, &settings).await?;
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
        "settings": &settings,
        "directories": directories
            .into_iter()
            .map(|(id, name, slug)| json!({ "id": id, "name": name, "slug": slug }))
            .collect::<Vec<_>>(),
        "configured_providers": configured
            .iter()
            .map(|c| json!({ "provider": c.provider, "label": c.label }))
            .collect::<Vec<_>>(),
        "source_catalog": source_catalog_json(&settings, &configured),
        "active_provider": resolved.as_ref().map(|c| c.provider.clone()),
        "active_provider_label": resolved.as_ref().map(|c| c.label.clone()),
        "supported_adapters": ALL_ADAPTERS,
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
    if !is_super_admin(&claims) {
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

// ─────────────────────────────────────────────────────────────────────────────
// B83 item 2 — ONE SEARCH: query every ENABLED source at once, in parallel
// ─────────────────────────────────────────────────────────────────────────────

/// One source's participation in a one-search run, in plain words for the panel.
#[derive(Debug, Serialize, Clone)]
pub struct SourceProbe {
    pub provider: String,
    pub label: String,
    /// matched | no_match | error | off | not_configured
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchAllRequest {
    pub query: String,
    pub directory_id: Option<Uuid>,
}

/// Add a field to the merged record, remembering where it came from. The first (highest
/// confidence) source wins a field; a later source may only FILL one that is still empty, and a
/// different non-empty value is recorded as a conflict so the losing value is never silently
/// discarded (card B83 item 2 / the B79 merge contract).
fn merge_field(
    fields: &mut serde_json::Map<String, Value>,
    conflicts: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Value,
    source: &str,
) {
    if value.is_null() {
        return;
    }
    if let Some(s) = value.as_str() {
        if s.trim().is_empty() {
            return;
        }
    }
    match fields.get(key) {
        None => {
            fields.insert(key.to_string(), json!({ "value": value, "source": source }));
        }
        Some(existing) => {
            let same = existing.get("value").map(|v| v == &value).unwrap_or(false);
            if !same {
                let entry = conflicts
                    .entry(key.to_string())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Some(arr) = entry.as_array_mut() {
                    arr.push(json!({ "value": value, "source": source }));
                }
            }
        }
    }
}

/// POST /enrich/search — ONE search box, every ENABLED source queried CONCURRENTLY, one merged
/// result set with per-field provenance. Read-only: it never writes to a business. Adding the
/// record to the directory is a separate, explicit action in the panel.
pub async fn search_all_sources(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<SearchAllRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to search enrichment sources".to_string(),
        ));
    }
    let query = req.query.trim().to_string();
    if query.chars().count() < 2 {
        return Err(AppError::Validation(
            "Type at least two characters to search.".to_string(),
        ));
    }

    let settings = effective_settings(&s.db, req.directory_id).await?;
    let available = configured_search_providers(&s.db).await?;

    // Decide, per catalog source, whether it runs; a source that cannot run is reported with
    // its plain-English reason instead of being silently dropped.
    let mut runnable: Vec<ProviderCfg> = Vec::new();
    let mut static_probes: std::collections::HashMap<String, SourceProbe> =
        std::collections::HashMap::new();
    for (provider, label, _desc, _needs_key) in SOURCE_CATALOG.iter() {
        if !source_enabled(&settings, provider, &available) {
            static_probes.insert(
                provider.to_string(),
                SourceProbe {
                    provider: provider.to_string(),
                    label: label.to_string(),
                    status: "off".to_string(),
                    message: Some("Switched off in the Sources panel above.".to_string()),
                },
            );
            continue;
        }
        if FREE_ADAPTERS.contains(provider) {
            runnable.push(free_provider_cfg(provider));
        } else if let Some(c) = available.iter().find(|c| c.provider == *provider) {
            runnable.push(c.clone());
        } else {
            static_probes.insert(
                provider.to_string(),
                SourceProbe {
                    provider: provider.to_string(),
                    label: label.to_string(),
                    status: "not_configured".to_string(),
                    message: Some(
                        "Switched on, but no key is saved yet — add one in Provider API Keys."
                            .to_string(),
                    ),
                },
            );
        }
    }
    let enabled_count = runnable.len();

    // Fire every enabled source at the SAME time — not one after another (card B83 item 2).
    let mut set = tokio::task::JoinSet::new();
    for cfg in runnable {
        let q = query.clone();
        set.spawn(async move {
            let provider = cfg.provider.clone();
            let label = cfg.label.clone();
            let outcome = provider_search(&cfg, &q).await;
            (provider, label, outcome)
        });
    }

    let mut hits: Vec<(String, Enriched)> = Vec::new();
    let mut dynamic_probes: Vec<SourceProbe> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((provider, label, Ok(Some(found)))) => {
                dynamic_probes.push(SourceProbe {
                    provider: provider.clone(),
                    label,
                    status: "matched".to_string(),
                    message: None,
                });
                hits.push((provider, found));
            }
            Ok((provider, label, Ok(None))) => dynamic_probes.push(SourceProbe {
                provider,
                label,
                status: "no_match".to_string(),
                message: Some("Answered, but no matching listing.".to_string()),
            }),
            Ok((provider, label, Err(e))) => dynamic_probes.push(SourceProbe {
                provider,
                label,
                status: "error".to_string(),
                message: Some(e),
            }),
            Err(join_err) => dynamic_probes.push(SourceProbe {
                provider: "unknown".to_string(),
                label: "A source task".to_string(),
                status: "error".to_string(),
                message: Some(join_err.to_string()),
            }),
        }
    }

    // Highest-confidence source wins each field; lower ones only fill what is still missing.
    hits.sort_by(|a, b| {
        b.1.confidence
            .partial_cmp(&a.1.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut fields = serde_json::Map::new();
    let mut conflicts = serde_json::Map::new();
    for (provider, e) in &hits {
        if let Some(v) = &e.name {
            merge_field(&mut fields, &mut conflicts, "name", json!(v), provider);
        }
        if let Some(v) = &e.address {
            merge_field(&mut fields, &mut conflicts, "address", json!(v), provider);
        }
        if let Some(v) = &e.phone {
            merge_field(&mut fields, &mut conflicts, "phone", json!(v), provider);
        }
        if let Some(v) = &e.website {
            merge_field(&mut fields, &mut conflicts, "website", json!(v), provider);
        }
        if let Some(v) = e.lat {
            merge_field(&mut fields, &mut conflicts, "latitude", json!(v), provider);
        }
        if let Some(v) = e.lng {
            merge_field(&mut fields, &mut conflicts, "longitude", json!(v), provider);
        }
        if let Some(v) = e.rating {
            merge_field(&mut fields, &mut conflicts, "rating", json!(v), provider);
        }
    }

    let mut result = serde_json::Map::new();
    for (k, v) in &fields {
        result.insert(k.clone(), v.get("value").cloned().unwrap_or(Value::Null));
    }

    // Emit the source list in the same order the Sources panel shows it, so the report reads
    // top-to-bottom matching the checkboxes.
    let dynamic: std::collections::HashMap<String, SourceProbe> = dynamic_probes
        .into_iter()
        .map(|p| (p.provider.clone(), p))
        .collect();
    let mut sources: Vec<SourceProbe> = Vec::new();
    for (provider, label, _d, _k) in SOURCE_CATALOG.iter() {
        if let Some(p) = dynamic.get(*provider) {
            sources.push(p.clone());
        } else if let Some(p) = static_probes.get(*provider) {
            sources.push(p.clone());
        } else {
            sources.push(SourceProbe {
                provider: provider.to_string(),
                label: label.to_string(),
                status: "error".to_string(),
                message: Some("A source did not report a result.".to_string()),
            });
        }
    }

    Ok(Json(json!({
        "query": query,
        "enabled_count": enabled_count,
        "matched_count": hits.len(),
        "sources": sources,
        "result": Value::Object(result),
        "fields": Value::Object(fields),
        "conflicts": Value::Object(conflicts),
    })))
}
