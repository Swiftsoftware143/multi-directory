//! Satellite town approval (W30).
//!
//! Town pages publish only for towns an admin has approved, one town at a time, and only while
//! the city's "include satellite towns" switch is on. This module is the console side of that
//! gate: it lists the towns a city actually holds (with their live listing counts), creates the
//! missing rows as drafts, and flips a single town on or off. The public gate lives in
//! `subfolder::local_town_entries`, which reads the same rows — so what the console shows and
//! what the site publishes can never disagree.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::handlers::subfolder::{local_town_entries, satellite_approval};
use crate::handlers::zaarhub::{town_key, town_slug};
use crate::security::provider_key_crypto;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ApproveRequest {
    pub approved: bool,
}

#[derive(Debug, Deserialize)]
pub struct IncludeRequest {
    pub include_satellite_towns: bool,
}

/// The city a directory is anchored to.
///
/// `directories.city` is NULLABLE, and a directory created without one used to make this screen
/// answer 500 (`error occurred while decoding column 0: unexpected null`); the SEO console then
/// logged a failed call and the Sitemaps card could not finish loading (measured 2026-11-11 on a
/// city-less directory). A directory with no city has no towns to approve, so answer a plain
/// validation error the screen can show instead of a server error.
async fn directory_city(db: &PgPool, directory_id: Uuid) -> ApiResult<String> {
    let dir: Option<(Option<String>,)> =
        sqlx::query_as("SELECT city FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_optional(db)
            .await?;

    let (city,) = dir.ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    match city.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        Some(city) => Ok(city),
        None => Err(AppError::Validation(
            "This directory has no city set yet. Add the city in Directory Details, then reopen Satellite Towns."
                .to_string(),
        )),
    }
}

/// GET /api/v1/seo/satellite-towns/:directory_id
///
/// Every town of this city with its own live listing count and its effective status:
/// `draft` (not approved — publishes nothing), `approved` (approved but currently below the
/// three-listing floor, so no page yet) or `live` (approved and publishing). Missing rows are
/// created as drafts on the way through, so a newly appearing town shows up here without
/// anybody having to seed it.
pub async fn list_satellite_towns(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let city = directory_city(&s.db, directory_id).await?;

    let towns = local_town_entries(&s.db, directory_id, &city).await;
    let (include, approved) = satellite_approval(&s.db, directory_id).await;

    // Persist a draft row for every town the city actually holds, so the console and the gate
    // read the same set on the next request. A missing table (install mid-upgrade) is tolerated:
    // the gate itself fails open, and the list still renders.
    for t in &towns {
        let _ = sqlx::query(
            "INSERT INTO satellite_towns (directory_id, town_key, town_label) VALUES ($1, $2, $3) \
             ON CONFLICT (directory_id, town_key) DO NOTHING",
        )
        .bind(directory_id)
        .bind(&t.key)
        .bind(&t.label)
        .execute(&s.db)
        .await;
    }

    let mut rows: Vec<serde_json::Value> = towns
        .iter()
        .map(|t| {
            let is_approved = approved.get(&t.key).copied().unwrap_or(false);
            let status = if !is_approved {
                "draft"
            } else if t.published {
                "live"
            } else {
                "approved"
            };
            json!({
                "town_key": t.key,
                "town_label": t.label,
                "slug": t.slug,
                "count": t.count,
                "is_home": t.is_home,
                "approved": is_approved,
                "status": status,
            })
        })
        .collect();

    // W28 — towns Google detected around this city that no listing has reached yet. They exist
    // only in `satellite_towns` (no listing, so no entry comes back from the listing-derived
    // list above), and without this a detected town would be invisible: it could never be
    // approved, and the whole detection would have done nothing the operator could see.
    if include {
        let detected: Vec<(String, String, bool)> = sqlx::query_as(
            "SELECT town_key, town_label, approved FROM satellite_towns \
             WHERE directory_id = $1 ORDER BY town_label",
        )
        .bind(directory_id)
        .fetch_all(&s.db)
        .await
        .unwrap_or_default();

        for (key, label, is_approved) in detected {
            if towns.iter().any(|t| t.key == key) {
                continue;
            }
            rows.push(json!({
                "town_key": key,
                "town_label": label,
                "slug": town_slug(&label),
                "count": 0,
                "is_home": false,
                "approved": is_approved,
                "detected": true,
                "status": if is_approved { "approved" } else { "draft" },
            }));
        }
    }

    Ok(Json(json!({
        "directory_id": directory_id,
        "city": city,
        "include_satellite_towns": include,
        "min_listings": 3,
        "towns": rows,
    })))
}

/// PUT /api/v1/seo/satellite-towns/:directory_id/:town_key — approve or revoke ONE town.
pub async fn set_town_approval(
    State(s): State<AppState>,
    Path((directory_id, town_key)): Path<(Uuid, String)>,
    Json(body): Json<ApproveRequest>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let key = town_key.trim().to_lowercase();
    if key.is_empty() {
        return Err(AppError::BadRequest("town_key is required".to_string()));
    }

    let city = directory_city(&s.db, directory_id).await?;

    // Only a town this city actually holds inside its own local area can be approved — never a
    // free-text name that would publish an empty page. A town Google detected for the city
    // (W28) is legitimate too even before a listing mentions it: it is a real row in
    // `satellite_towns`, and approving it early is what makes its page publish the moment the
    // third listing arrives instead of waiting for someone to notice.
    let towns = local_town_entries(&s.db, directory_id, &city).await;
    let label = match towns.iter().find(|t| t.key == key) {
        Some(town) => town.label.clone(),
        None => {
            let detected: Option<(String,)> = sqlx::query_as(
                "SELECT town_label FROM satellite_towns WHERE directory_id = $1 AND town_key = $2",
            )
            .bind(directory_id)
            .bind(&key)
            .fetch_optional(&s.db)
            .await?;
            detected.map(|(l,)| l).ok_or_else(|| {
                AppError::BadRequest("That town is not part of this city".to_string())
            })?
        }
    };

    sqlx::query(
        "INSERT INTO satellite_towns (directory_id, town_key, town_label, approved, approved_at) \
         VALUES ($1, $2, $3, $4, CASE WHEN $4 THEN now() ELSE NULL END) \
         ON CONFLICT (directory_id, town_key) DO UPDATE SET \
           approved = EXCLUDED.approved, \
           approved_at = EXCLUDED.approved_at, \
           town_label = EXCLUDED.town_label, \
           updated_at = now()",
    )
    .bind(directory_id)
    .bind(&key)
    .bind(&label)
    .bind(body.approved)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "town_key": key,
        "town_label": label,
        "approved": body.approved,
        "status": if body.approved { "approved" } else { "draft" },
    })))
}

/// PUT /api/v1/seo/satellite-settings/:directory_id — the city's "include satellite towns"
/// switch. Unchecking unpublishes every town page of that city (approvals are kept).
pub async fn set_satellite_include(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(body): Json<IncludeRequest>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let updated = sqlx::query(
        "UPDATE directories SET include_satellite_towns = $2, updated_at = now() WHERE id = $1",
    )
    .bind(directory_id)
    .bind(body.include_satellite_towns)
    .execute(&s.db)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    Ok(Json(json!({
        "directory_id": directory_id,
        "include_satellite_towns": body.include_satellite_towns,
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// W28 — auto-detect the towns around a city (no manual lists anywhere)
// ─────────────────────────────────────────────────────────────────────────────
//
// A city's towns used to appear only once a listing happened to carry that town's name, so a
// brand-new city knew nothing about its neighbours and an admin had to wait for the data. This
// asks Google for the localities around the city centre and records every one it does not
// already know as a DRAFT (`satellite_towns` row, `approved = false`): nothing publishes, no
// page and no sitemap entry appears, but the admin screen can offer them and approving one is
// what makes its page go live the moment the third listing lands.
//
// Google is asked through the provider key on file — never a baked-in key — and two places
// endpoints are used because the legacy nearby search returns ONE locality where the current
// one returns the whole town cluster (measured on Deltona, FL: 1 vs 20).

/// How far around the city centre Google is asked for localities. Wider than the publishing
/// radius (`zaarhub::LOCAL_AREA_KM`) so a town just outside it is still recorded — the gate
/// decides what publishes; this only decides what is worth offering an admin.
const NEARBY_RADIUS_M: f64 = 30_000.0;

/// What one detection pass found and stored.
#[derive(Debug, Default)]
pub struct Detected {
    /// Distinct locality names Google returned.
    pub found: usize,
    /// New draft rows written by this pass.
    pub added: usize,
    /// Localities this city already knew about (listing-derived or detected earlier).
    pub already: usize,
    /// Locality names skipped because they are a city of their own.
    pub hubs_skipped: Vec<String>,
    /// Which configured key answered, and whether that row is switched on. Reported so the
    /// console can say which credential did the work instead of implying "Google works".
    pub key_provider: String,
    pub key_switched_on: bool,
}

/// POST one Places API (New) endpoint with one key. Returns the parsed body or a short reason.
/// The key never appears in an error string, so nothing can leak it into a log or a response.
async fn places_v1_post(
    path: &str,
    key: &str,
    field_mask: &str,
    body: Value,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("could not start the request: {e}"))?;
    let resp = client
        .post(format!("https://places.googleapis.com/v1/{path}"))
        .header("X-Goog-Api-Key", key)
        .header("X-Goog-FieldMask", field_mask)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Google could not be reached: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Google refused the request");
        return Err(format!("Google refused the request ({status}): {message}"));
    }
    Ok(parsed)
}

/// Every configured Google key, the operator's DEFAULT first, then the switched-on ones.
///
/// A key is not judged by its flags — the vendor is asked (same rule the Integration Center's
/// Test button follows): one install can hold a Maps key that only works from a browser and a
/// Places key that only speaks the legacy API, so the only honest way to know which key can
/// answer is to try it. A row that cannot be decrypted is skipped (log + skip, never hand
/// ciphertext to Google).
pub(crate) async fn google_key_candidates(db: &PgPool) -> Vec<(String, String, bool, String)> {
    let rows: Result<Vec<(String, String, bool, String)>, sqlx::Error> = sqlx::query_as(
        "SELECT provider, api_key, is_active, COALESCE(label, provider) FROM provider_keys \
         WHERE provider LIKE 'google%' \
         ORDER BY is_default DESC, is_active DESC, updated_at DESC",
    )
    .fetch_all(db)
    .await;

    let mut out = Vec::new();
    for (provider, stored, active, label) in rows.unwrap_or_default() {
        if let Some(key) = provider_key_crypto::decrypt_for_use(db, &stored, &provider).await {
            if !key.trim().is_empty() {
                out.push((provider, key, active, label));
            }
        }
    }
    out
}

/// The city's centre: the average of its own listings when it has any (free, exact, and it is
/// the same centre the town pages use), otherwise Google's answer for "City, ST".
async fn city_centre(
    db: &PgPool,
    directory_id: Uuid,
    city: &str,
    state: Option<&str>,
    key: &str,
) -> Result<(f64, f64), String> {
    let own: Result<Option<(Option<f64>, Option<f64>)>, sqlx::Error> = sqlx::query_as(
        "SELECT AVG(latitude), AVG(longitude) FROM businesses \
         WHERE directory_id = $1 AND is_active = true AND latitude IS NOT NULL",
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await;

    if let Ok(Some((Some(lat), Some(lon)))) = own {
        return Ok((lat, lon));
    }

    let query = match state.map(str::trim) {
        Some(st) if !st.is_empty() => format!("{}, {}", city.trim(), st),
        _ => city.trim().to_string(),
    };
    let body = json!({ "textQuery": query, "maxResultCount": 1 });
    let found = places_v1_post("places:searchText", key, "places.location", body).await?;
    let location = found
        .get("places")
        .and_then(Value::as_array)
        .and_then(|p| p.first())
        .and_then(|p| p.get("location"));

    let lat = location
        .and_then(|l| l.get("latitude"))
        .and_then(Value::as_f64);
    let lon = location
        .and_then(|l| l.get("longitude"))
        .and_then(Value::as_f64);
    match (lat, lon) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(format!("Google found no location for {query}")),
    }
}

/// The localities Google knows inside the radius around a point.
async fn nearby_localities(
    key: &str,
    lat: f64,
    lon: f64,
) -> Result<Vec<(String, f64, f64)>, String> {
    let body = json!({
        "includedTypes": ["locality"],
        "maxResultCount": 20,
        "locationRestriction": {
            "circle": {
                "center": { "latitude": lat, "longitude": lon },
                "radius": NEARBY_RADIUS_M,
            }
        }
    });
    let found = places_v1_post(
        "places:searchNearby",
        key,
        "places.displayName,places.location",
        body,
    )
    .await?;

    let mut out: Vec<(String, f64, f64)> = Vec::new();
    if let Some(places) = found.get("places").and_then(Value::as_array) {
        for p in places {
            let name = p
                .get("displayName")
                .and_then(|d| d.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let la = p
                .get("location")
                .and_then(|l| l.get("latitude"))
                .and_then(Value::as_f64)
                .unwrap_or(lat);
            let lo = p
                .get("location")
                .and_then(|l| l.get("longitude"))
                .and_then(Value::as_f64)
                .unwrap_or(lon);
            out.push((name, la, lo));
        }
    }
    Ok(out)
}

/// Detect and store the towns around a city. Every key on file is tried in turn (default
/// first) so one restriction on one key does not switch the capability off for the app.
pub async fn detect_and_store(
    db: &PgPool,
    directory_id: Uuid,
    city: &str,
    state: Option<&str>,
) -> Result<Detected, String> {
    let keys = google_key_candidates(db).await;
    if keys.is_empty() {
        return Err(
            "No Google Places key is configured. Add one in the Integration Center, then try again."
                .to_string(),
        );
    }

    // A name that is already a city of its own must never come back as a satellite: two pages
    // for one place compete with each other in search, which is the opposite of the point.
    let hub_rows: Vec<(String,)> =
        sqlx::query_as("SELECT city FROM directories WHERE city IS NOT NULL")
            .fetch_all(db)
            .await
            .unwrap_or_default();
    let mut hubs: Vec<String> = hub_rows
        .into_iter()
        .map(|(c,)| town_key(&c))
        .filter(|c| !c.is_empty())
        .collect();
    hubs.push(town_key(city));

    let mut last_error = String::new();
    let mut localities: Vec<(String, f64, f64)> = Vec::new();
    let mut used = String::new();
    let mut used_active = false;
    let mut centre: Option<(f64, f64)> = None;

    for (provider, key, active, label) in &keys {
        let c = match city_centre(db, directory_id, city, state, key).await {
            Ok(c) => c,
            Err(e) => {
                last_error = format!("{provider} ({label}): {e}");
                continue;
            }
        };
        match nearby_localities(key, c.0, c.1).await {
            Ok(list) => {
                localities = list;
                centre = Some(c);
                used = format!("{provider} · {label}");
                used_active = *active;
                break;
            }
            Err(e) => last_error = format!("{provider} ({label}): {e}"),
        }
    }

    let Some(centre) = centre else {
        return Err(last_error);
    };
    if !used_active {
        tracing::warn!(
            key = %used,
            "satellite towns: the key that answered is not switched on in the Integration Center"
        );
    }
    tracing::info!(
        key = %used,
        lat = centre.0,
        lon = centre.1,
        "satellite towns: asked Google for the localities around a city"
    );

    let mut det = Detected {
        key_provider: used,
        key_switched_on: used_active,
        ..Detected::default()
    };
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (name, _la, _lo) in localities {
        let key = town_key(&name);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        det.found += 1;
        if hubs.contains(&key) {
            det.hubs_skipped.push(name);
            continue;
        }
        let inserted = sqlx::query(
            "INSERT INTO satellite_towns (directory_id, town_key, town_label) VALUES ($1, $2, $3) \
             ON CONFLICT (directory_id, town_key) DO NOTHING",
        )
        .bind(directory_id)
        .bind(&key)
        .bind(name.trim())
        .execute(db)
        .await;

        match inserted {
            Ok(r) if r.rows_affected() > 0 => det.added += 1,
            Ok(_) => det.already += 1,
            Err(e) => {
                tracing::warn!(error = %e, town = %key, "satellite town could not be stored");
                det.already += 1;
            }
        }
    }
    Ok(det)
}

/// POST /api/v1/seo/satellite-towns/:directory_id/detect — ask Google which towns surround this
/// city and record every one it does not know yet as a draft. Additive only: an existing row
/// (and therefore an existing approval) is never touched, and nothing is ever deleted.
pub async fn detect_nearby_towns(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let dir: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT city, state FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_optional(&s.db)
            .await?;
    let (city, state) = dir.ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;
    if city.trim().is_empty() {
        return Err(AppError::Validation(
            "This city has no city name yet, so there is nothing to search around. Set the city in Directory Details first."
                .to_string(),
        ));
    }

    let det = detect_and_store(&s.db, directory_id, &city, state.as_deref())
        .await
        .map_err(AppError::Validation)?;

    Ok(Json(json!({
        "directory_id": directory_id,
        "city": city,
        "found": det.found,
        "added": det.added,
        "already_listed": det.already,
        "hubs_skipped": det.hubs_skipped,
        "key": { "provider": det.key_provider, "switched_on": det.key_switched_on },
    })))
}
