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
//! Cost note: a cold view costs at most one media call, plus one place lookup when the stored
//! name has expired. The response is cacheable for a week and a listing exposes at most
//! `MAX_PROXIED_PHOTOS` of them.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use uuid::Uuid;

use crate::error::AppError;
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

fn image_response(bytes: Vec<u8>, content_type: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=604800".to_string()),
        ],
        Body::from(bytes),
    )
        .into_response()
}

/// Ask Google for the picture behind one photo name.
async fn media_bytes(client: &reqwest::Client, key: &str, name: &str) -> MediaOutcome {
    let url = format!(
        "{}/{}/media?maxWidthPx=1200&skipHttpRedirect=true",
        PLACES_V1_BASE, name
    );
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
    let mut last_error = String::new();
    for (provider, key, _active, label) in
        crate::handlers::satellite::google_key_candidates(&s.db).await
    {
        if !provider.starts_with("google") {
            continue;
        }
        match media_bytes(&client, &key, &name).await {
            MediaOutcome::Bytes(bytes, ct) => {
                tracing::debug!(
                    business = %business_id, index, key = %label, bytes = bytes.len(),
                    "place photo served from the stored reference"
                );
                return Ok(image_response(bytes, ct));
            }
            MediaOutcome::InvalidName => {
                // Stored references expire (see the module note). Re-resolve from the place id.
                match fresh_photo_name(&client, &key, &place_id, index).await {
                    Ok(Some(fresh)) => match media_bytes(&client, &key, &fresh).await {
                        MediaOutcome::Bytes(bytes, ct) => {
                            tracing::debug!(
                                business = %business_id, index, key = %label, bytes = bytes.len(),
                                "place photo served after refreshing an expired reference"
                            );
                            return Ok(image_response(bytes, ct));
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
}
