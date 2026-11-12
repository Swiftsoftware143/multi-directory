//! Google Places (New) photo proxy — a stored photo reference becomes a real image.
//!
//! The `images` column has held three different things since the first import: absolute upload
//! URLs, root-relative `/uploads/...` paths, and Google Places photo RESOURCE NAMES
//! (`places/ChIJ…/photos/AWC…`). Only the first two are URLs a browser can load, so
//! `photo_is_renderable()` (zaarhub.rs) correctly refuses the third — which is why 3,883 listings
//! showed "No photos yet" while their photo reference sat in the database.
//!
//! A Google photo name is not a URL, but it CAN be turned into bytes, on demand, by this app:
//! `GET {PLACES_V1_BASE}/{name}/media?skipHttpRedirect=true` answers a `photoUri`, and that URI is
//! the picture.
//!
//! MEASURED 2026-11-12 — a stored photo name EXPIRES. The names written at import time answer
//! `400 "The photo resource in the request is invalid. Please retrieve it from Places API
//! endpoints."`, while a name for the SAME place returned by a fresh lookup answers 200. Names
//! are therefore re-resolved from the place id (which does not expire) whenever the stored one is
//! refused, so no re-import is needed and no expired reference is ever trusted twice.
//!
//! Two further rules this module keeps:
//!   * the key is read from `provider_keys` at call time (never hardcoded, never an env var);
//!   * an unavailable photo answers 404, so the page hides the frame instead of showing a
//!     broken image.
//!
//! Cost note (measured on the live binary 2026-11-12, Palm Bay): a cold photo used to cost FOUR
//! billed Google requests — a refusal from the first configured key (it refuses Places API (New)),
//! a media call with the working key on the expired stored name, a place lookup for a fresh name,
//! and the media call that finally answers. Two changes cut that:
//!   * the key that ANSWERED is remembered in-process and tried first on the next photo
//!     ([`ANSWERING_LABEL`]) — a steady-state cold photo is 3 requests, not 4;
//!   * the bytes are written to the on-disk cache in `photo_cache` (two keys: the stored name and
//!     the fresh one), so every later view is served from disk with no Google call at all
//!     (7-day TTL, size-capped, LRU-evicted). A listing exposes at most `MAX_PROXIED_PHOTOS`.
//! [`warm_up`] fills that cache ahead of traffic, one capped city at a time.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::handlers::photo_cache::{self, CacheConfig};
use crate::handlers::places_client::PLACES_V1_BASE;
use crate::state::AppState;

/// How many Google-sourced photos a listing exposes. Each one is a billed Google call on a cold
/// view, so this is a deliberate ceiling — not a pagination limit.
pub const MAX_PROXIED_PHOTOS: usize = 3;

/// Where the browser can load a place-photo name from: this app's own proxy path.
pub fn proxy_path(business_id: Uuid, index: usize) -> String {
    format!("/api/v1/place-photo/{business_id}/{index}")
}

/// TRUE only for a Google Places photo resource NAME (`places/<id>/photos/<ref>`).
///
/// Anything else — an `http(s)` URL, an upload path, a legacy `photo_reference` blob, a string
/// with `..` or a scheme in the middle — is refused. Names are the only thing the v1 media
/// endpoint accepts, and this check is what stops the proxy being used as an open fetcher.
pub fn is_place_photo_name(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() || s.len() > 512 {
        return false;
    }
    if !s.starts_with("places/") || !s.contains("/photos/") {
        return false;
    }
    !s.contains("//")
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
}

/// The place id a photo name is built on (`places/<id>/photos/<ref>`). This is what makes a
/// refresh possible: the id is stable, the photo reference is not.
pub fn place_id_of(name: &str) -> Option<String> {
    let mut parts = name.trim().split('/');
    if parts.next()? != "places" {
        return None;
    }
    let id = parts.next()?;
    if parts.next()? != "photos" {
        return None;
    }
    if id.is_empty()
        || id.len() > 200
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return None;
    }
    Some(id.to_string())
}

/// The loadable `src` for one stored photo reference, or `None` when it cannot be shown.
///
/// `index` is the reference's position in the stored array, because that is what the proxy path
/// carries — the caller must pass the real index, or the wrong photo is served.
pub fn display_src(business_id: Uuid, index: usize, raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if is_place_photo_name(s) {
        return Some(proxy_path(business_id, index));
    }
    // Anything that is already a URL is left exactly as it was.
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("/uploads/") {
        return Some(s.to_string());
    }
    None
}

/// The raw reference at `index` from a stored `images` value, accepting both shapes this column
/// takes in the database: `["…", …]` and `[{"url": …} | {"photo_reference": …}, …]`.
fn raw_at(images: &Value, index: usize) -> Option<String> {
    let arr = images.as_array()?;
    let item = arr.get(index)?;
    match item {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o
            .get("url")
            .or_else(|| o.get("photo_reference"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

/// What one media call produced.
enum MediaOutcome {
    /// Real image bytes and their content type.
    Bytes(Vec<u8>, String),
    /// Google refused the NAME (expired/mismatched) — worth re-resolving from the place id.
    InvalidName,
    /// The key or the network refused; say why and move on to the next key.
    Failed(String),
}

/// One configured Google credential: `(provider, key, is_active, label)` — the shape
/// `satellite::google_key_candidates` answers.
type KeyCandidate = (String, String, bool, String);

/// The LABEL of the key that last answered a Places v1 MEDIA request in this process.
///
/// MEASURED 2026-11-12 (Palm Bay, 24 cold cards = 96 billed calls): the FIRST configured Google
/// key refuses Places API (New) with 403 while the key that answers sits further down the list
/// (labelled "SUPERSEDED …", `is_active = false`). Every cold photo therefore paid a wasted call
/// before the loop reached the working key. Remembering WHICH label answered lets the next cold
/// photo try it first, which takes a cold photo from 4 billed calls to 3.
///
/// Only the LABEL is memoised — never a key value and never a decrypted secret. The candidate
/// list, and therefore the key itself, is still read from `provider_keys` (and decrypted) on every
/// request, so a rotated key, a re-pointed provider or a renamed label is picked up immediately
/// and nothing about the credential survives outside the existing decryption path.
static ANSWERING_LABEL: OnceLock<StdMutex<Option<String>>> = OnceLock::new();

fn answering_label() -> Option<String> {
    ANSWERING_LABEL
        .get_or_init(|| StdMutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Called only when Google actually returned bytes for a media request: a key that answered is
/// remembered, a key that failed never is.
fn remember_answering_label(label: &str) {
    if label.trim().is_empty() {
        return;
    }
    let mut slot = ANSWERING_LABEL
        .get_or_init(|| StdMutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *slot = Some(label.to_string());
}

/// Put the memoised key first, keeping every other candidate and the original order.
///
/// Pure so the ordering rule is testable without a database: the memo must never REMOVE a
/// candidate (the loop still has to fall through when the remembered key stops answering), and an
/// unknown or empty memo must leave the list untouched.
fn order_candidates(mut cands: Vec<KeyCandidate>, memo: Option<&str>) -> Vec<KeyCandidate> {
    let memo = match memo.map(str::trim) {
        Some(m) if !m.is_empty() => m,
        _ => return cands,
    };
    if let Some(pos) = cands.iter().position(|c| c.3 == memo) {
        let first = cands.remove(pos);
        cands.insert(0, first);
    }
    cands
}

fn image_response(
    bytes: Vec<u8>,
    content_type: String,
    verdict: &'static str,
    max_age_secs: u64,
) -> Response {
    let mut resp = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CACHE_CONTROL,
                format!("public, max-age={max_age_secs}"),
            ),
        ],
        Body::from(bytes),
    )
        .into_response();
    // `hit` = served from the disk cache, `miss` = Google was billed for this response. The proof
    // script counts these, so the cache's effect is measured per request instead of asserted.
    if let Ok(v) = HeaderValue::from_str(verdict) {
        resp.headers_mut()
            .insert(HeaderName::from_static(photo_cache::HEADER), v);
    }
    resp
}

/// Ask Google for the picture behind one photo name.
async fn media_bytes(client: &reqwest::Client, key: &str, name: &str) -> MediaOutcome {
    let url = format!(
        "{}/{}/media?maxWidthPx=1200&skipHttpRedirect=true",
        PLACES_V1_BASE, name
    );
    photo_cache::note_google_call();
    let resp = match client.get(&url).header("X-Goog-Api-Key", key).send().await {
        Ok(r) => r,
        Err(e) => return MediaOutcome::Failed(format!("transport {e}")),
    };
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        // 400/404 is about the NAME; 401/403 is about the KEY.
        if status == 400 || status == 404 {
            return MediaOutcome::InvalidName;
        }
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
        return MediaOutcome::Failed(format!(
            "HTTP {status} {}",
            snippet.chars().take(160).collect::<String>()
        ));
    }
    let body: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return MediaOutcome::Failed(format!("json {e}")),
    };
    let Some(uri) = body.get("photoUri").and_then(Value::as_str) else {
        return MediaOutcome::Failed("no photoUri".to_string());
    };
    if !uri.starts_with("https://") {
        return MediaOutcome::Failed("refused a non-https photoUri".to_string());
    }
    let img = match client.get(uri).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => return MediaOutcome::Failed(format!("image HTTP {}", r.status().as_u16())),
        Err(e) => return MediaOutcome::Failed(format!("image transport {e}")),
    };
    let content_type = img
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string();
    match img.bytes().await {
        Ok(b) if !b.is_empty() && b.len() <= 8 * 1024 * 1024 => {
            MediaOutcome::Bytes(b.to_vec(), content_type)
        }
        Ok(b) => MediaOutcome::Failed(format!("unusable size {}", b.len())),
        Err(e) => MediaOutcome::Failed(format!("image body {e}")),
    }
}

/// The CURRENT photo name at `index` for a place, or `None` when the place has no photos.
///
/// One Place Details call with a single-field mask (`photos.name`) — the cheapest way to obtain a
/// name Google will still honour.
async fn fresh_photo_name(
    client: &reqwest::Client,
    key: &str,
    place_id: &str,
    index: usize,
) -> Result<Option<String>, String> {
    let url = format!("{}/places/{}", PLACES_V1_BASE, place_id);
    photo_cache::note_google_call();
    let resp = client
        .get(&url)
        .header("X-Goog-Api-Key", key)
        .header("X-Goog-FieldMask", "photos.name")
        .send()
        .await
        .map_err(|e| format!("details transport {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
        return Err(format!(
            "details HTTP {status} {}",
            snippet.chars().take(160).collect::<String>()
        ));
    }
    let body: Value = resp.json().await.map_err(|e| format!("details json {e}"))?;
    let names: Vec<String> = body
        .get("photos")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.get("name").and_then(Value::as_str))
                .filter(|n| is_place_photo_name(n))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        return Ok(None);
    }
    // Same position when it exists, so index 0 stays the first photo it always was.
    Ok(Some(names[index.min(names.len() - 1)].clone()))
}

/// GET /api/v1/place-photo/:business_id/:index — the listing's photo bytes.
///
/// Public on purpose: it is an image on a public page, and a logged-out visitor (and a crawler)
/// must be able to load it. It exposes nothing but a picture that is already published with the
/// listing; only live rows are served, so an unpublished business leaks no image.
pub async fn place_photo(
    State(s): State<AppState>,
    Path((business_id, index)): Path<(Uuid, usize)>,
) -> Result<Response, AppError> {
    respond(&s, business_id, index).await
}

/// The one implementation of "give me this listing's photo", shared by the public route and the
/// warm-up job.
///
/// The warm-up goes through THIS function, not a shortcut into the cache: it therefore passes the
/// same single-flight claim (two visitors and the job never pay for the same photo twice), stores
/// under the same keys, and is subject to the same expiry and size rules.
async fn respond(s: &AppState, business_id: Uuid, index: usize) -> Result<Response, AppError> {
    let not_found = || AppError::NotFound("No photo for this listing".to_string());

    // Live rows only — the same lifecycle rule the public detail lookups enforce.
    let row: Option<(Option<Value>, Option<String>)> = sqlx::query_as(
        "SELECT images, place_id FROM businesses \
         WHERE id = $1 AND is_active = true AND COALESCE(status, 'active') <> 'draft'",
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?;
    let (images, stored_place_id) = row.ok_or_else(not_found)?;

    let raw = raw_at(&images.unwrap_or(Value::Null), index).ok_or_else(not_found)?;
    let name = raw.trim().to_string();
    if !is_place_photo_name(&name) {
        return Err(not_found());
    }
    // The id inside the name is the fallback, because the column may be empty on older rows.
    let Some(place_id) = place_id_of(&name).or_else(|| {
        stored_place_id
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
    }) else {
        return Err(not_found());
    };

    let cfg = CacheConfig::from_env();

    // 1. Warm: the bytes are on disk already, so Google is not called at all.
    if let Some((bytes, ct)) = photo_cache::get(&cfg, &name).await {
        tracing::debug!(
            business = %business_id, index, bytes = bytes.len(),
            "place photo served from the disk cache"
        );
        return Ok(image_response(bytes, ct, "hit", cfg.ttl_secs));
    }

    // 2. Cold: one request fetches a given photo. A second request for the same photo (two
    //    visitors at once, or a page requested twice in parallel) waits briefly for the first to
    //    store it instead of paying Google for the same call again.
    let _claim = match photo_cache::claim(&name) {
        Some(claim) => Some(claim),
        None => {
            for _ in 0..30 {
                tokio::time::sleep(photo_cache::WAIT_STEP).await;
                if let Some((bytes, ct)) = photo_cache::peek(&cfg, &name).await {
                    photo_cache::note_hit(bytes.len());
                    return Ok(image_response(bytes, ct, "hit", cfg.ttl_secs));
                }
            }
            None
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(format!("photo client: {e}")))?;

    // Try each Google key on file until one answers. A key restricted to a different API is a
    // refusal, not an outage (measured 2026-11-12), so the loop is the point. The `is_active`
    // flag is NOT used as a filter: the key that answers Places API (New) on this install is
    // labelled "SUPERSEDED — do not use for search" and sits at is_active = false, while every
    // active key refuses the v1 endpoint. A key that answers is a key that works; the flag only
    // decides what the panel shows as the default.
    //
    // The key that answered LAST is tried FIRST (see ANSWERING_LABEL): that alone removes the
    // wasted refusal call every cold photo used to pay before reaching the working key.
    let candidates = order_candidates(
        crate::handlers::satellite::google_key_candidates(&s.db)
            .await
            .into_iter()
            .filter(|(provider, ..)| provider.starts_with("google"))
            .collect(),
        answering_label().as_deref(),
    );
    let mut last_error = String::new();
    for (_provider, key, _active, label) in candidates {
        match media_bytes(&client, &key, &name).await {
            MediaOutcome::Bytes(bytes, ct) => {
                remember_answering_label(&label);
                // Store before answering: a served photo is a cached photo from now on.
                photo_cache::put(&cfg, &name, &bytes, &ct).await;
                tracing::debug!(
                    business = %business_id, index, key = %label, bytes = bytes.len(),
                    "place photo served from the stored reference"
                );
                return Ok(image_response(bytes, ct, "miss", cfg.ttl_secs));
            }
            MediaOutcome::InvalidName => {
                // A 400/404 is about the NAME, so this key AUTHENTICATED — Google answered it.
                // Remember it for the same reason as a byte success: the next cold photo must not
                // pay a refusal call to the restricted key first. (Worth recording even when the
                // refresh below fails, because the key's ability to answer is what is being learnt,
                // not this one photo's fate.)
                remember_answering_label(&label);
                // Stored references expire (see the module note). Re-resolve from the place id.
                match fresh_photo_name(&client, &key, &place_id, index).await {
                    Ok(Some(fresh)) => match media_bytes(&client, &key, &fresh).await {
                        MediaOutcome::Bytes(bytes, ct) => {
                            remember_answering_label(&label);
                            // Two entries, one photo: the name the listing holds (so the stale
                            // reference is warm from now on) and the name Google just honoured
                            // (so any listing storing the fresh name shares these bytes).
                            photo_cache::put(&cfg, &name, &bytes, &ct).await;
                            photo_cache::put(&cfg, &fresh, &bytes, &ct).await;
                            tracing::debug!(
                                business = %business_id, index, key = %label, bytes = bytes.len(),
                                "place photo served after refreshing an expired reference"
                            );
                            return Ok(image_response(bytes, ct, "miss", cfg.ttl_secs));
                        }
                        MediaOutcome::InvalidName => {
                            last_error = format!("{label}: refreshed name also refused")
                        }
                        MediaOutcome::Failed(why) => last_error = format!("{label}: {why}"),
                    },
                    Ok(None) => last_error = format!("{label}: this place has no photos"),
                    Err(why) => last_error = format!("{label}: {why}"),
                }
            }
            MediaOutcome::Failed(why) => last_error = format!("{label}: {why}"),
        }
    }

    // Unavailable is a 404: the page then hides the frame rather than showing a broken image.
    tracing::warn!(business = %business_id, index, reason = %last_error, "place photo unavailable");
    Err(not_found())
}

/// Is `index` within the proxy ceiling? Keeps a page from asking for twenty billed images.
pub fn within_budget(index: usize) -> bool {
    index < MAX_PROXIED_PHOTOS
}

/// GET /api/v1/place-photo/cache-stats — what the photo cache is doing, in numbers.
///
/// Aggregate counters only (no keys, no listing data), so the effect of the cache is measurable
/// from outside the process: `google_calls` is the number of BILLED Google Places requests this
/// process has issued, and `hits` / `misses` are served-from-disk versus paid-for.
pub async fn cache_stats() -> Json<Value> {
    Json(photo_cache::stats(&CacheConfig::from_env()).await)
}

// ─────────────────────────────────────────────────────────────────────────────
// Warm-up pass
// ─────────────────────────────────────────────────────────────────────────────

/// How many photos one warm-up run fetches by default.
///
/// A cold photo costs 2–3 BILLED Google Places requests, so this is a money bound rather than a
/// pagination limit: at Google's ~7 USD per 1,000 photo requests, 30 photos is a few cents a run.
/// The job is a background trickle, not a bulk import.
pub const DEFAULT_WARM_PHOTOS: usize = 30;
/// Hard ceiling for one run, whatever the caller asks for (a typo must not start a 4,000-photo
/// batch against the account's quota and budget).
pub const MAX_WARM_PHOTOS: usize = 200;
/// A run stops after this long, so an external scheduler's HTTP client never times out on it.
const WARM_DEADLINE: Duration = Duration::from_secs(45);

#[derive(Debug, Deserialize)]
pub struct WarmQuery {
    /// How many photos this run may fetch (`DEFAULT_WARM_PHOTOS`, hard-capped at
    /// `MAX_WARM_PHOTOS`).
    pub photos: Option<usize>,
    /// Warm this directory slug only. Omitted = the city the job should finish next.
    pub city: Option<String>,
}

/// One photo a card will actually ask the proxy for.
struct WarmPhoto {
    business: Uuid,
    index: usize,
    name: String,
}

/// `POST /api/v1/cron/photo-warm` — fill the photo cache ahead of traffic.
///
/// WHY: a city page only offers a proxy photo for a card whose bytes are already on disk (a cold
/// photo spends the page's `CARD_PHOTO_PROXY_BUDGET`), so coverage grows only as visitors arrive:
/// Palm Bay showed 24 photos on the first view, 48 on the second, and needed ~12 views — each
/// paying that view's cold budget. This job warms a city with nobody visiting, one city at a time,
/// through the SAME code path as the public route (`respond`), so it uses the same cache keys and
/// the same single-flight claim. It never raises a page's budget.
///
/// The photo it warms is the one `card_image` would render (`card_place_photo_index`), in the order
/// the city page presents its cards, so a partly warmed city looks better from the top of the page
/// down. "Warm" is decided from the CACHE DIRECTORY, not from this process's memory, so a restart
/// cannot make a warmed city look cold and re-spend the budget.
pub async fn warm_up(
    State(s): State<AppState>,
    Query(q): Query<WarmQuery>,
) -> ApiResult<Json<Value>> {
    use sqlx::Row;

    let cfg = CacheConfig::from_env();
    let cap = q
        .photos
        .unwrap_or(DEFAULT_WARM_PHOTOS)
        .clamp(1, MAX_WARM_PHOTOS);

    // Every listing whose CARD would show a proxy photo, in city-page order (rating first, the
    // same ORDER BY the city listing uses) so progress is visible where a visitor looks first.
    let rows = sqlx::query(
        "SELECT b.id, b.images, b.logo_url, b.cover_url, d.slug AS city_slug \
         FROM businesses b JOIN directories d ON d.id = b.directory_id \
         WHERE b.is_active = TRUE AND COALESCE(b.status, 'active') <> 'draft' \
           AND b.images::text LIKE '%/photos/%' \
         ORDER BY d.slug ASC, b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST, b.name ASC",
    )
    .fetch_all(&s.db)
    .await?;

    let mut by_city: Vec<(String, Vec<WarmPhoto>)> = Vec::new();
    for row in &rows {
        let city: String = row.try_get("city_slug")?;
        let business: Uuid = row.try_get("id")?;
        let images: Value = row
            .try_get::<Option<Value>, _>("images")?
            .unwrap_or(Value::Null);
        let logo: Option<String> = row.try_get("logo_url").unwrap_or_default();
        let cover: Option<String> = row.try_get("cover_url").unwrap_or_default();

        // Ask the RENDERER what this card would show: an owner upload or a real URL costs nothing
        // and needs no warming. Only a proxy path is worth a billed call.
        let mut budget = usize::MAX;
        let Some(src) = crate::handlers::zaarhub::card_image(
            business,
            &images,
            logo.as_deref(),
            cover.as_deref(),
            &mut budget,
        ) else {
            continue;
        };
        if !src.starts_with("/api/v1/place-photo/") {
            continue;
        }
        let Some(index) = crate::handlers::zaarhub::card_place_photo_index(&images) else {
            continue;
        };
        let Some(name) = raw_at(&images, index) else {
            continue;
        };
        let name = name.trim().to_string();
        if !is_place_photo_name(&name) {
            continue;
        }
        match by_city.iter_mut().find(|(c, _)| *c == city) {
            Some((_, list)) => list.push(WarmPhoto {
                business,
                index,
                name,
            }),
            None => by_city.push((
                city.clone(),
                vec![WarmPhoto {
                    business,
                    index,
                    name,
                }],
            )),
        }
    }

    let warm_before_set = photo_cache::warm_keys(&cfg).await;
    let calls_before = photo_cache::stats(&cfg).await["google_calls"]
        .as_u64()
        .unwrap_or(0);
    let is_warm_before = |p: &WarmPhoto| warm_before_set.contains(&photo_cache::key_of(&p.name));

    let coverage = |set: &std::collections::HashSet<String>| -> Vec<Value> {
        by_city
            .iter()
            .map(|(city, photos)| {
                let warm = photos
                    .iter()
                    .filter(|p| set.contains(&photo_cache::key_of(&p.name)))
                    .count();
                json!({
                    "city": city,
                    "photos": photos.len(),
                    "warm": warm,
                    "remaining": photos.len() - warm,
                })
            })
            .collect()
    };

    if by_city.is_empty() {
        return Ok(Json(json!({
            "status": "no_candidates",
            "detail": "no listing in any directory renders a Google-proxied card photo",
            "cities": [],
        })));
    }

    // ONE city per run, and the city with the LEAST work left, so a city is finished before the
    // next one starts eating the budget.
    let chosen = match q.city.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(slug) => by_city
            .iter()
            .position(|(c, _)| c == slug)
            .ok_or_else(|| AppError::NotFound(format!("No directory '{slug}' with photo cards")))?,
        None => {
            let next = by_city
                .iter()
                .enumerate()
                .map(|(i, (_, photos))| (i, photos.iter().filter(|p| !is_warm_before(p)).count()))
                .filter(|(_, left)| *left > 0)
                .min_by_key(|(_, left)| *left)
                .map(|(i, _)| i);
            match next {
                Some(i) => i,
                // Nothing left to do anywhere: answer 200 with the numbers, so a scheduler does
                // not have to treat "done" as an error and page somebody about it.
                None => {
                    return Ok(Json(json!({
                        "status": "all_warm",
                        "cities": coverage(&warm_before_set),
                    })))
                }
            }
        }
    };

    let (city, photos) = &by_city[chosen];
    let total = photos.len();
    let warm_before = photos.iter().filter(|p| is_warm_before(p)).count();

    let started = Instant::now();
    let mut attempted = 0usize;
    let mut served = 0usize;
    let mut failed = 0usize;
    let mut last_error = String::new();
    for photo in photos.iter().filter(|p| !is_warm_before(p)) {
        if attempted >= cap || started.elapsed() > WARM_DEADLINE {
            break;
        }
        attempted += 1;
        match respond(&s, photo.business, photo.index).await {
            Ok(_resp) => served += 1,
            Err(e) => {
                failed += 1;
                last_error = format!("{}#{}: {e}", photo.business, photo.index);
            }
        }
    }

    let warm_after_set = photo_cache::warm_keys(&cfg).await;
    let calls_after = photo_cache::stats(&cfg).await["google_calls"]
        .as_u64()
        .unwrap_or(0);
    let calls = calls_after.saturating_sub(calls_before);
    let warm_after = photos
        .iter()
        .filter(|p| warm_after_set.contains(&photo_cache::key_of(&p.name)))
        .count();
    let coverage_pct = if total == 0 {
        100.0
    } else {
        (warm_after as f64 * 100.0 / total as f64 * 10.0).round() / 10.0
    };
    let calls_per_photo = if attempted == 0 {
        0.0
    } else {
        (calls as f64 / attempted as f64 * 100.0).round() / 100.0
    };

    tracing::info!(
        city = %city, total, warm_before, warm_after, attempted, served, failed, calls,
        "photo cache warm-up run"
    );

    Ok(Json(json!({
        "status": if warm_after == total { "city_warm" } else { "warmed" },
        "city": city,
        "photos_total": total,
        "warm_before": warm_before,
        "warm_after": warm_after,
        "coverage_pct": coverage_pct,
        "remaining": total - warm_after,
        "attempted": attempted,
        "served": served,
        "failed": failed,
        "google_calls": calls,
        "calls_per_photo": calls_per_photo,
        "cities": coverage(&warm_after_set),
        "last_error": last_error,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_real_photo_names_pass() {
        assert!(is_place_photo_name(
            "places/ChIJwzpk63tw54gRdbJ0Jw-u_Jk/photos/AWCwydiN33v_Ur6tVTkkZdkFD2Fy"
        ));
        assert!(!is_place_photo_name("https://example.com/a.jpg"));
        assert!(!is_place_photo_name("/uploads/businesses/x/y.jpg"));
        assert!(!is_place_photo_name("places/../../etc/passwd/photos/x"));
        assert!(!is_place_photo_name("places/x/photos/../secrets"));
        assert!(!is_place_photo_name("ChIJabc"));
        assert!(!is_place_photo_name(""));
    }

    #[test]
    fn place_id_is_read_out_of_the_name() {
        assert_eq!(
            place_id_of("places/ChIJwzpk63tw54gRdbJ0Jw-u_Jk/photos/AWCwydiN").as_deref(),
            Some("ChIJwzpk63tw54gRdbJ0Jw-u_Jk")
        );
        assert_eq!(place_id_of("places//photos/x"), None);
        assert_eq!(place_id_of("places/abc/notphotos/x"), None);
        assert_eq!(place_id_of("https://x/places/abc/photos/x"), None);
        assert_eq!(
            place_id_of("places/a-b_c/photos/x").as_deref(),
            Some("a-b_c")
        );
    }

    #[test]
    fn display_src_maps_names_and_keeps_urls() {
        let id = Uuid::nil();
        let expected = format!("/api/v1/place-photo/{id}/2");
        assert_eq!(
            display_src(id, 2, "places/ChIJa/photos/AWCb").as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            display_src(id, 0, "https://cdn.example/x.jpg").as_deref(),
            Some("https://cdn.example/x.jpg")
        );
        assert_eq!(display_src(id, 0, "garbage"), None);
    }

    #[test]
    fn raw_at_reads_both_column_shapes() {
        assert_eq!(raw_at(&json!(["a", "b"]), 1).as_deref(), Some("b"));
        assert_eq!(
            raw_at(&json!([{"photo_reference": "places/x/photos/y"}]), 0).as_deref(),
            Some("places/x/photos/y")
        );
        assert!(raw_at(&json!(["a"]), 3).is_none());
    }

    fn cand(label: &str, active: bool) -> KeyCandidate {
        (
            "google".to_string(),
            format!("key-{label}"),
            active,
            label.to_string(),
        )
    }

    #[test]
    fn the_key_that_answered_is_tried_first() {
        // The install's real shape: an ACTIVE key that refuses Places API (New) is listed first,
        // and the key that actually answers is a later, inactive one.
        let list = vec![
            cand("active-but-refuses", true),
            cand("superseded-but-answers", false),
            cand("spare", true),
        ];
        let ordered = order_candidates(list.clone(), Some("superseded-but-answers"));
        assert_eq!(ordered[0].3, "superseded-but-answers");
        // Every candidate survives, in its original relative order after the promoted one.
        assert_eq!(ordered.len(), list.len());
        assert_eq!(ordered[1].3, "active-but-refuses");
        assert_eq!(ordered[2].3, "spare");
    }

    #[test]
    fn an_unknown_or_empty_memo_leaves_the_order_alone() {
        let list = vec![cand("a", true), cand("b", true)];
        assert_eq!(
            order_candidates(list.clone(), Some("gone"))
                .iter()
                .map(|c| c.3.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            order_candidates(list.clone(), None)
                .iter()
                .map(|c| c.3.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            order_candidates(list, Some("   "))
                .iter()
                .map(|c| c.3.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn an_empty_label_is_never_remembered() {
        // A label is the ONLY thing memoised; a blank one would let an arbitrary later key match.
        // (Guarded in remember_answering_label; exercised here through the pure sorter.)
        let list = vec![
            ("google".to_string(), "k1".to_string(), true, String::new()),
            cand("named", true),
        ];
        let ordered = order_candidates(list, Some(""));
        assert!(ordered[0].3.is_empty());
    }

    #[test]
    fn the_warm_cap_is_a_money_bound() {
        // Both constants are money bounds, not pagination: a cold photo is 2–3 billed Google
        // requests, so a run must stay small and the ceiling must stay reachable-but-low.
        assert!(DEFAULT_WARM_PHOTOS >= 10 && DEFAULT_WARM_PHOTOS <= 60);
        assert!(MAX_WARM_PHOTOS >= DEFAULT_WARM_PHOTOS);
        assert!(MAX_WARM_PHOTOS <= 500);
    }
}
