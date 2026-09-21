/// ZaarHub legal pages + site config management
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::security::provider_key_crypto as keycrypto;
use crate::AppState;

// ── Legal Pages ──

#[derive(Deserialize)]
pub struct LegalPagePayload {
    pub slug: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub is_published: bool,
    #[serde(default)]
    pub show_in_footer: bool,
    #[serde(default)]
    pub display_order: i32,
}

/// GET /api/v1/zaarhub/admin/legal — list all legal pages
pub async fn list_legal_pages(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        "SELECT id, slug, title, is_published, show_in_footer, display_order, updated_at \
         FROM zaarhub_legal_pages ORDER BY display_order ASC, title ASC",
    )
    .fetch_all(&state.db)
    .await?;

    let pages: Vec<Value> = rows.iter().map(|r| {
        json!({
            "id": r.try_get::<Uuid,_>("id").map(|v| v.to_string()).unwrap_or_default(),
            "slug": r.try_get::<String,_>("slug").unwrap_or_default(),
            "title": r.try_get::<String,_>("title").unwrap_or_default(),
            "is_published": r.try_get::<bool,_>("is_published").unwrap_or(false),
            "show_in_footer": r.try_get::<bool,_>("show_in_footer").unwrap_or(false),
            "display_order": r.try_get::<i32,_>("display_order").unwrap_or(0),
            "updated_at": r.try_get::<Option<chrono::NaiveDateTime>,_>("updated_at").unwrap_or_default(),
        })
    }).collect();

    Ok(Json(json!({ "pages": pages })))
}

/// GET /api/v1/zaarhub/admin/legal/:slug — get a legal page by slug
pub async fn get_legal_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT id, slug, title, content, is_published, show_in_footer, display_order, created_at, updated_at \
         FROM zaarhub_legal_pages WHERE slug = $1"
    ).bind(&slug).fetch_optional(&state.db).await?;

    match row {
        Some(r) => Ok(Json(json!({
            "id": r.try_get::<Uuid,_>("id").map(|v| v.to_string()).unwrap_or_default(),
            "slug": r.try_get::<String,_>("slug").unwrap_or_default(),
            "title": r.try_get::<String,_>("title").unwrap_or_default(),
            "content": r.try_get::<String,_>("content").unwrap_or_default(),
            "is_published": r.try_get::<bool,_>("is_published").unwrap_or(false),
            "show_in_footer": r.try_get::<bool,_>("show_in_footer").unwrap_or(false),
            "display_order": r.try_get::<i32,_>("display_order").unwrap_or(0),
            "created_at": r.try_get::<Option<chrono::NaiveDateTime>,_>("created_at").unwrap_or_default(),
            "updated_at": r.try_get::<Option<chrono::NaiveDateTime>,_>("updated_at").unwrap_or_default(),
        }))),
        None => Err(AppError::NotFound("Legal page not found".into())),
    }
}

/// POST /api/v1/zaarhub/admin/legal — create or update a legal page
pub async fn save_legal_page(
    State(state): State<AppState>,
    Json(payload): Json<LegalPagePayload>,
) -> ApiResult<Json<Value>> {
    let existing = sqlx::query("SELECT id FROM zaarhub_legal_pages WHERE slug = $1")
        .bind(&payload.slug)
        .fetch_optional(&state.db)
        .await?;

    match existing {
        Some(r) => {
            let id: Uuid = r.try_get("id").unwrap_or_default();
            sqlx::query(
                "UPDATE zaarhub_legal_pages SET title = $1, content = $2, is_published = $3, \
                 show_in_footer = $4, display_order = $5, updated_at = now() WHERE id = $6",
            )
            .bind(&payload.title)
            .bind(&payload.content)
            .bind(payload.is_published)
            .bind(payload.show_in_footer)
            .bind(payload.display_order)
            .bind(id)
            .execute(&state.db)
            .await?;
            Ok(Json(
                json!({ "id": id.to_string(), "slug": payload.slug, "updated": true }),
            ))
        }
        None => {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO zaarhub_legal_pages (id, slug, title, content, is_published, show_in_footer, display_order) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)"
            )
            .bind(id).bind(&payload.slug).bind(&payload.title).bind(&payload.content)
            .bind(payload.is_published).bind(payload.show_in_footer).bind(payload.display_order)
            .execute(&state.db).await?;
            Ok(Json(
                json!({ "id": id.to_string(), "slug": payload.slug, "created": true }),
            ))
        }
    }
}

/// DELETE /api/v1/zaarhub/admin/legal/:slug — delete a legal page
pub async fn delete_legal_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM zaarhub_legal_pages WHERE slug = $1")
        .bind(&slug)
        .execute(&state.db)
        .await?;
    Ok(Json(
        json!({ "deleted": result.rows_affected() > 0, "slug": slug }),
    ))
}

// ── Site Config ──

#[derive(Deserialize)]
pub struct SiteConfigPayload {
    pub site_name: Option<String>,
    pub site_tagline: Option<String>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub google_analytics_id: Option<String>,
    pub facebook_app_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub copyright_year: Option<String>,
}

/// GET /api/v1/zaarhub/admin/config — get site config
pub async fn get_site_config(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let row = sqlx::query("SELECT * FROM zaarhub_site_config LIMIT 1")
        .fetch_optional(&state.db)
        .await?;

    match row {
        Some(r) => Ok(Json(json!({
            "site_name": r.try_get::<String,_>("site_name").unwrap_or_default(),
            "site_tagline": r.try_get::<Option<String>,_>("site_tagline").unwrap_or_default(),
            "primary_color": r.try_get::<Option<String>,_>("primary_color").unwrap_or_default(),
            "secondary_color": r.try_get::<Option<String>,_>("secondary_color").unwrap_or_default(),
            "logo_url": r.try_get::<Option<String>,_>("logo_url").unwrap_or_default(),
            "favicon_url": r.try_get::<Option<String>,_>("favicon_url").unwrap_or_default(),
            "google_analytics_id": r.try_get::<Option<String>,_>("google_analytics_id").unwrap_or_default(),
            "facebook_app_id": r.try_get::<Option<String>,_>("facebook_app_id").unwrap_or_default(),
            "twitter_handle": r.try_get::<Option<String>,_>("twitter_handle").unwrap_or_default(),
            "contact_email": r.try_get::<Option<String>,_>("contact_email").unwrap_or_default(),
            "contact_phone": r.try_get::<Option<String>,_>("contact_phone").unwrap_or_default(),
            "copyright_year": r.try_get::<Option<String>,_>("copyright_year").unwrap_or_default(),
        }))),
        None => Ok(Json(json!({
            "site_name": "ZaarHub",
            "site_tagline": "Discover Your Local Community",
        }))),
    }
}

// ── Google Places Provider Key ──

/// GET /api/v1/zaarhub/admin/provider-keys/google-places — get masked key + loaded state
pub async fn get_gplaces_key(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT id, tenant_id, provider, label, is_default, is_active, scope, metadata, \
                api_key \
         FROM provider_keys WHERE provider = 'google_places' \
         ORDER BY is_default DESC, updated_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await?;

    match row {
        Some(r) => {
            // The column holds enc:v1 ciphertext; decrypt with the env master key (never the
            // database) before masking, so the panel shows a mask and never the ciphertext.
            let stored: String = r.try_get::<String, _>("api_key").unwrap_or_default();
            let full_key = keycrypto::decrypt_for_display(&state.db, &stored).await;
            let masked = crate::handlers::provider_keys_handler::mask_key(&full_key);
            Ok(Json(json!({
                "configured": !full_key.is_empty(),
                "masked": if full_key.is_empty() { String::new() } else { masked },
                "label": r.try_get::<String, _>("label").unwrap_or_else(|_| "default".to_string()),
                "is_default": r.try_get::<bool, _>("is_default").unwrap_or(true),
                "is_active": r.try_get::<bool,_>("is_active").unwrap_or(false),
                "scope": r.try_get::<String,_>("scope").unwrap_or_else(|_| "global".to_string()),
            })))
        }
        None => Ok(Json(json!({
            "configured": false,
            "masked": "",
            "is_active": false,
            "scope": "global",
        }))),
    }
}

#[derive(Deserialize)]
pub struct GplacesKeyPayload {
    pub api_key: String,
    /// Optional human name for this key (“Palm Bay project”, “Test account”).
    #[serde(default)]
    pub label: Option<String>,
}

/// POST /api/v1/zaarhub/admin/provider-keys/google-places — upsert (encrypted at rest)
pub async fn save_gplaces_key(
    State(state): State<AppState>,
    Json(payload): Json<GplacesKeyPayload>,
) -> ApiResult<Json<Value>> {
    let key = payload.api_key.trim().to_string();
    if key.is_empty() {
        return Err(AppError::BadRequest("API key cannot be empty".into()));
    }
    if !key.starts_with("AIza") {
        return Err(AppError::BadRequest(
            "Google Places API keys start with AIza".into(),
        ));
    }

    // Encryption happens in the app (env-only master key, fail-closed) before the value reaches
    // the database; migration 095's CHECK constraint refuses a plaintext write outright.
    let stored_key = keycrypto::encrypt_for_storage(&state.db, &key).await?;

    // Bind a real tenant_id (must exist in tenants FK). Reuse an existing provider key's tenant,
    // else fall back to the super-admin seed tenant.
    let tenant_id: Uuid = sqlx::query_scalar::<_, Uuid>(
        "SELECT tenant_id FROM provider_keys WHERE provider = $1 LIMIT 1",
    )
    .bind("google_places")
    .fetch_optional(&state.db)
    .await?
    .unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());

    let label = payload
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or("default")
        .to_string();

    // Exactly one default per provider: demote the others, then save this one as default.
    sqlx::query(
        "UPDATE provider_keys SET is_default = false \
         WHERE tenant_id = $1 AND provider = 'google_places' AND is_default = true",
    )
    .bind(tenant_id)
    .execute(&state.db)
    .await?;

    sqlx::query(
        "INSERT INTO provider_keys (tenant_id, provider, label, api_key, is_active, scope, is_default) \
         VALUES ($1, $2, $3, $4, true, 'global', true) \
         ON CONFLICT (tenant_id, provider, label) DO UPDATE \
         SET api_key = EXCLUDED.api_key, is_active = true, scope = 'global', \
             is_default = true, updated_at = NOW()"
    )
    .bind(tenant_id)
    .bind("google_places")
    .bind(&label)
    .bind(&stored_key)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({"saved": true, "label": label})))
}

/// POST /api/v1/zaarhub/admin/provider-keys/google-places/test — validate via Autocomplete
pub async fn test_gplaces_key(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let row = sqlx::query(
        "SELECT api_key FROM provider_keys \
         WHERE provider = 'google_places' AND is_active = true \
         ORDER BY is_default DESC, updated_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await?;

    let key: String = match row {
        Some(r) => {
            let stored: String = r.try_get::<String, _>("api_key").unwrap_or_default();
            keycrypto::decrypt_for_use(&state.db, &stored, "google_places")
                .await
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "The saved Google Places key cannot be decrypted — re-save it".into(),
                    )
                })?
        }
        None => {
            return Err(AppError::BadRequest(
                "No Google Places API key saved yet".into(),
            ))
        }
    };

    let url = format!(
        "https://maps.googleapis.com/maps/api/place/autocomplete/json?input=italian%20restaurant&key={}",
        key
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("Network error testing key: {e}")))?;
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("UKNOWN");

    if status == "OK" || status == "ZERO_RESULTS" {
        Ok(Json(
            json!({"ok": true, "status": status, "message": "Key is valid — Google accepts it"}),
        ))
    } else {
        Ok(Json(json!({
            "ok": false,
            "status": status,
            "message": body.get("error_message").and_then(|v| v.as_str()).unwrap_or("Key rejected by Google"),
        })))
    }
}

/// PATCH /api/v1/zaarhub/admin/config — update site config
/// GET /api/v1/zaarhub/admin/places/search — text search Google Places (returns businesses)
#[derive(Deserialize)]
pub struct PlacesSearchQuery {
    pub query: String,
    pub city: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub radius: Option<i32>,
    /// TEST-ONLY stub: `?mock=true` returns a fixed fixture so the queue flow can be
    /// exercised while David's Google Places key is still invalid. The response is
    /// flagged `"stub": true` and the panel shows a warning banner.
    #[serde(default)]
    pub mock: Option<bool>,
}

pub async fn places_text_search(
    State(state): State<AppState>,
    Query(q): Query<PlacesSearchQuery>,
) -> ApiResult<Json<Value>> {
    if q.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".into()));
    }

    if q.mock == Some(true) {
        let out = mock_places_results();
        tracing::warn!("[places] STUB results returned (mock=true) - not live Google data");
        return Ok(Json(json!({
            "status": "MOCK",
            "stub": true,
            "count": out.len(),
            "results": out
        })));
    }

    let row = sqlx::query(
        "SELECT api_key FROM provider_keys \
         WHERE provider = 'google_places' AND is_active = true \
         ORDER BY is_default DESC, updated_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await?;
    let key: String = match row {
        Some(r) => {
            let stored: String = r.try_get::<String, _>("api_key").unwrap_or_default();
            keycrypto::decrypt_for_use(&state.db, &stored, "google_places")
                .await
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "The saved Google Places key cannot be decrypted — re-save it".into(),
                    )
                })?
        }
        None => {
            return Err(AppError::BadRequest(
                "No Google Places API key saved yet — save one above".into(),
            ))
        }
    };

    let mut query = q.query.trim().to_string();
    if let Some(city) = q.city.as_deref() {
        if !city.is_empty() && !query.to_lowercase().contains(&city.to_lowercase()) {
            query.push_str(" in ");
            query.push_str(city);
        }
    }
    let url = format!(
        "https://maps.googleapis.com/maps/api/place/textsearch/json?query={}&key={}",
        urlencoding(&query),
        key
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("Google Places API error: {e}")))?;
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    if status == "REQUEST_DENIED" {
        let msg = body
            .get("error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Key REQUEST_DENIED by Google");
        return Err(AppError::BadRequest(msg.to_string()));
    }

    let results: Vec<Value> = body
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let out: Vec<Value> = results.into_iter().map(|r| json!({
        "name": r.get("name").and_then(|v| v.as_str()).unwrap_or(""),
        "address": r.get("formatted_address").and_then(|v| v.as_str()).unwrap_or(""),
        "place_id": r.get("place_id").and_then(|v| v.as_str()).unwrap_or(""),
        "rating": r.get("rating").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "user_ratings_total": r.get("user_ratings_total").and_then(|v| v.as_i64()).unwrap_or(0),
        "lat": r.pointer("/geometry/location/lat").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "lng": r.pointer("/geometry/location/lng").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "website": r.pointer("/website").and_then(|v| v.as_str()).unwrap_or(""),
        "phone": r.pointer("/formatted_phone_number").and_then(|v| v.as_str()).unwrap_or(""),
        // `types` power the auto category mapping (T3) and the franchise filter.
        "types": r.get("types").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
    })).collect();

    Ok(Json(
        json!({ "status": status, "count": out.len(), "results": out }),
    ))
}

fn urlencoding(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join("%20")
}

/// PATCH /api/v1/zaarhub/admin/config — update site config
pub async fn update_site_config(
    State(state): State<AppState>,
    Json(payload): Json<SiteConfigPayload>,
) -> ApiResult<Json<Value>> {
    let existing = sqlx::query("SELECT id FROM zaarhub_site_config LIMIT 1")
        .fetch_optional(&state.db)
        .await?;

    match existing {
        Some(r) => {
            let id: Uuid = r.try_get("id").unwrap_or_default();
            sqlx::query(
                "UPDATE zaarhub_site_config SET site_name = $1, site_tagline = $2, \
                 primary_color = $3, secondary_color = $4, logo_url = $5, favicon_url = $6, \
                 google_analytics_id = $7, facebook_app_id = $8, twitter_handle = $9, \
                 contact_email = $10, contact_phone = $11, copyright_year = $12, updated_at = now() WHERE id = $13"
            )
            .bind(payload.site_name.as_deref().unwrap_or("ZaarHub"))
            .bind(payload.site_tagline.as_deref())
            .bind(payload.primary_color.as_deref())
            .bind(payload.secondary_color.as_deref())
            .bind(payload.logo_url.as_deref())
            .bind(payload.favicon_url.as_deref())
            .bind(payload.google_analytics_id.as_deref())
            .bind(payload.facebook_app_id.as_deref())
            .bind(payload.twitter_handle.as_deref())
            .bind(payload.contact_email.as_deref())
            .bind(payload.contact_phone.as_deref())
            .bind(payload.copyright_year.as_deref())
            .bind(id)
            .execute(&state.db).await?;
            Ok(Json(json!({ "updated": true })))
        }
        None => {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO zaarhub_site_config (id, site_name, site_tagline, primary_color, secondary_color, \
                 logo_url, favicon_url, google_analytics_id, facebook_app_id, twitter_handle, contact_email, contact_phone, copyright_year) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
            )
            .bind(id).bind(payload.site_name.as_deref().unwrap_or("ZaarHub"))
            .bind(payload.site_tagline.as_deref()).bind(payload.primary_color.as_deref())
            .bind(payload.secondary_color.as_deref()).bind(payload.logo_url.as_deref())
            .bind(payload.favicon_url.as_deref()).bind(payload.google_analytics_id.as_deref())
            .bind(payload.facebook_app_id.as_deref()).bind(payload.twitter_handle.as_deref())
            .bind(payload.contact_email.as_deref()).bind(payload.contact_phone.as_deref())
            .bind(payload.copyright_year.as_deref())
            .execute(&state.db).await?;
            Ok(Json(json!({ "created": true })))
        }
    }
}

/// TEST-ONLY fixture for `?mock=true` on the places search.
///
/// Deliberately mixed so the queue can be proven end to end:
///  * 3 known big chains (franchise filter must flag + auto-uncheck them)
///  * 3 businesses that already exist in the palm-bay directory (dedup must flag them)
///  * 6 local businesses, each carrying real Google-style `types` for category mapping
fn mock_places_results() -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let fixtures: [(&str, &str, f64, i64, &[&str]); 12] = [
        (
            "McDonald's",
            "100 Malabar Rd, Palm Bay, FL 32907",
            3.6,
            812,
            &["restaurant", "food", "point_of_interest", "establishment"],
        ),
        (
            "Starbucks",
            "4700 Babcock St NE, Palm Bay, FL 32908",
            4.2,
            640,
            &["cafe", "food", "point_of_interest", "establishment"],
        ),
        (
            "Subway",
            "1155 Malabar Rd, Palm Bay, FL 32907",
            3.9,
            402,
            &["restaurant", "food", "point_of_interest", "establishment"],
        ),
        (
            "Test Restaurant",
            "123 Main St, Palm Bay, FL 32905",
            4.0,
            12,
            &["restaurant", "food"],
        ),
        (
            "J&C Automotive",
            "1715 Agora Cir SE, Palm Bay, FL 32909, USA",
            4.7,
            58,
            &["car_repair", "point_of_interest"],
        ),
        (
            "W&J Gold Star Automotive",
            "4570 S Babcock St ste20, Palm Bay, FL 32905, USA",
            4.6,
            44,
            &["car_repair", "point_of_interest"],
        ),
        (
            "Palm Bay Family Dental",
            "2190 Port Malabar Blvd NE, Palm Bay, FL 32905",
            4.8,
            213,
            &["dentist", "health", "point_of_interest"],
        ),
        (
            "Space Coast Coffee Roasters",
            "605 Palm Bay Rd NE, Palm Bay, FL 32905",
            4.9,
            118,
            &["cafe", "food", "point_of_interest"],
        ),
        (
            "Bayside Auto Repair",
            "3301 Bayside Lakes Blvd, Palm Bay, FL 32909",
            4.5,
            76,
            &["car_repair", "point_of_interest"],
        ),
        (
            "Harbor City Plumbing",
            "880 Jupiter Blvd SE, Palm Bay, FL 32909",
            4.4,
            39,
            &["plumber", "point_of_interest"],
        ),
        (
            "Malabar Lawn Care",
            "1940 Malabar Rd SE, Palm Bay, FL 32909",
            4.3,
            21,
            &["point_of_interest", "establishment"],
        ),
        (
            "The Yoga Loft",
            "1420 Palm Bay Rd NE, Palm Bay, FL 32905",
            4.9,
            96,
            &["gym", "health", "point_of_interest"],
        ),
    ];
    for (i, (name, address, rating, reviews, types)) in fixtures.iter().enumerate() {
        out.push(json!({
            "name": name,
            "address": address,
            "place_id": format!("MOCK_PLACE_{:02}", i + 1),
            "rating": rating,
            "user_ratings_total": reviews,
            "lat": 28.0 + (i as f64) * 0.001,
            "lng": -80.6 - (i as f64) * 0.001,
            "website": "",
            "phone": "",
            "types": types,
        }));
    }
    out
}
