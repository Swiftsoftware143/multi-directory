//! Homepage configuration (card B86) — the admin-configurable HOMEPAGE per network and per
//! standalone directory.
//!
//! David (2026-09-30): "make sure that in the admin panel there is a place where you can SET THE
//! HOMEPAGE. Because the homepage isn't part of the directory if the directory is a city. But the
//! network still should have a home page. So the admin should be able to configure it."
//!
//! The homepage belongs to the NETWORK (or to a STANDALONE directory), never to a city, and no
//! surface may confuse the two. This module stores that configuration (one row per owning scope in
//! `homepage_config`) and serves it resolved directory -> network -> built-in defaults, so an
//! unconfigured network still renders a sane homepage:
//!   * which surface the root serves (the network's own home, or one of its cities featured),
//!   * the hero (headline, sub-headline, image, CTA),
//!   * which sections appear and in what order,
//!   * which cities are highlighted and their display order,
//!   * an optional announcement/banner.
//!
//! Nothing is hardcoded: ZaarHub's home is one configuration of this and a sold directory sets its
//! own from the panel (the sellable standard). The public read is GET /homepage/config; the admin
//! read/write is operator-guarded at /homepage-config/settings.

use crate::error::{ApiResult, AppError};
use crate::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// The standard homepage blocks, in the default order. The SPA renders exactly the keys that are
/// enabled, in the configured order, so the admin can both hide and reorder every block.
pub const SECTION_KEYS: [&str; 9] = [
    "spotlights",
    "loyalty",
    "cities",
    "top_rated",
    "deals",
    "buzz",
    "events",
    "cta_join",
    "cta_owner",
];

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct HomepageConfigRow {
    pub id: Uuid,
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub enabled: bool,
    pub home_surface: String,
    pub home_city_slug: Option<String>,
    pub announcement_text: Option<String>,
    pub announcement_cta_text: Option<String>,
    pub announcement_cta_url: Option<String>,
    pub hero_headline: Option<String>,
    pub hero_subheadline: Option<String>,
    pub hero_image_url: Option<String>,
    pub hero_cta_text: Option<String>,
    pub hero_cta_url: Option<String>,
    pub featured_city_slugs: Vec<String>,
    pub sections: Value,
}

#[derive(Debug, Deserialize)]
pub struct PublicQuery {
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsQuery {
    /// "network" or "directory"
    pub scope: String,
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct SaveSettings {
    pub scope: String,
    pub id: Uuid,
    pub enabled: Option<bool>,
    pub home_surface: Option<String>,
    pub home_city_slug: Option<String>,
    pub announcement_text: Option<String>,
    pub announcement_cta_text: Option<String>,
    pub announcement_cta_url: Option<String>,
    pub hero_headline: Option<String>,
    pub hero_subheadline: Option<String>,
    pub hero_image_url: Option<String>,
    pub hero_cta_text: Option<String>,
    pub hero_cta_url: Option<String>,
    pub featured_city_slugs: Option<Vec<String>>,
    pub sections: Option<Value>,
}

/// The column list every `HomepageConfigRow` select returns, kept in one place. A macro (not a
/// `const`) so the query text below is assembled by `concat!` at compile time: the SQL stays a
/// compile-time constant with no runtime string building (pre-build gate rule 5b).
macro_rules! config_cols {
    () => {
        "id, network_id, directory_id, enabled, home_surface, home_city_slug, \
         announcement_text, announcement_cta_text, announcement_cta_url, hero_headline, \
         hero_subheadline, hero_image_url, hero_cta_text, hero_cta_url, featured_city_slugs, sections"
    };
}

/// Default section list: every standard block, enabled, in the canonical order.
fn default_sections() -> Value {
    Value::Array(
        SECTION_KEYS
            .iter()
            .map(|k| json!({ "key": k, "enabled": true }))
            .collect(),
    )
}

/// Normalise an authored section list: keep only known keys, drop duplicates, preserve the author's
/// order, and append any standard block the author did not mention (enabled by default) so a new
/// block type can never silently disappear from every homepage.
fn normalise_sections(v: Option<&Value>) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    if let Some(Value::Array(items)) = v {
        for it in items {
            let key = it.get("key").and_then(|k| k.as_str()).unwrap_or("");
            if !SECTION_KEYS.contains(&key) || seen.iter().any(|s| s == key) {
                continue;
            }
            let enabled = it.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
            seen.push(key.to_string());
            out.push(json!({ "key": key, "enabled": enabled }));
        }
    }
    for k in SECTION_KEYS.iter() {
        if !seen.iter().any(|s| s == k) {
            out.push(json!({ "key": k, "enabled": true }));
        }
    }
    Value::Array(out)
}

/// Resolve a directory id -> (city_name, network_id, slug). None when the id is unknown.
async fn directory_context(
    db: &PgPool,
    id: Uuid,
) -> Result<Option<(Option<String>, Option<Uuid>, String)>, sqlx::Error> {
    sqlx::query_as::<_, (Option<String>, Option<Uuid>, String)>(
        "SELECT COALESCE(NULLIF(city, ''), name), network_id, slug FROM directories WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

/// The network the public root serves: the network owning the most directories (ZaarHub today),
/// resolved from live data rather than hardcoded.
async fn default_network_id(db: &PgPool) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"SELECT network_id FROM directories
            WHERE network_id IS NOT NULL
            GROUP BY network_id
            ORDER BY count(*) DESC, network_id
            LIMIT 1"#,
    )
    .fetch_optional(db)
    .await
}

/// The config row owned by EXACTLY this scope (used by the admin read/write paths).
async fn resolve_exact(
    db: &PgPool,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<Option<HomepageConfigRow>, sqlx::Error> {
    const SQL: &str = concat!(
        "SELECT ",
        config_cols!(),
        " FROM homepage_config \
          WHERE ($1::uuid IS NOT NULL AND directory_id = $1) \
             OR ($1::uuid IS NULL AND $2::uuid IS NOT NULL AND network_id = $2) LIMIT 1"
    );
    sqlx::query_as::<_, HomepageConfigRow>(SQL)
        .bind(directory_id)
        .bind(network_id)
        .fetch_optional(db)
        .await
}

/// The config row IN FORCE for a public page: directory -> network -> built-in defaults.
async fn resolve_effective(
    db: &PgPool,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<Option<HomepageConfigRow>, sqlx::Error> {
    const SQL: &str = concat!(
        "SELECT ",
        config_cols!(),
        " FROM homepage_config \
          WHERE ($1::uuid IS NOT NULL AND directory_id = $1) \
             OR ($2::uuid IS NOT NULL AND network_id = $2) \
          ORDER BY (directory_id IS NOT NULL) DESC LIMIT 1"
    );
    sqlx::query_as::<_, HomepageConfigRow>(SQL)
        .bind(directory_id)
        .bind(network_id)
        .fetch_optional(db)
        .await
}

/// A city row for the homepage city grid.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CityRow {
    pub slug: String,
    pub name: String,
    pub city: Option<String>,
    pub business_count: i64,
}

/// The directories the home may point at: the network's cities, or the standalone directory itself.
async fn home_cities(
    db: &PgPool,
    network_id: Option<Uuid>,
    directory_id: Option<Uuid>,
) -> Result<Vec<CityRow>, sqlx::Error> {
    sqlx::query_as::<_, CityRow>(
        r#"SELECT d.slug,
                  d.name,
                  NULLIF(d.city, '') AS city,
                  (SELECT count(*) FROM businesses b WHERE b.directory_id = d.id) AS business_count
             FROM directories d
            WHERE ($1::uuid IS NOT NULL AND d.network_id = $1)
               OR ($2::uuid IS NOT NULL AND d.id = $2)
            ORDER BY d.name"#,
    )
    .bind(network_id)
    .bind(directory_id)
    .fetch_all(db)
    .await
}

/// Order cities so the admin's featured list comes first, in their chosen order.
fn order_cities(mut cities: Vec<CityRow>, featured: &[String]) -> Vec<Value> {
    cities.sort_by(|a, b| a.name.cmp(&b.name));
    let mut featured_rows: Vec<Value> = Vec::new();
    for slug in featured {
        if let Some(pos) = cities.iter().position(|c| &c.slug == slug) {
            let c = cities.remove(pos);
            featured_rows.push(json!({
                "slug": c.slug, "name": c.name, "city": c.city,
                "business_count": c.business_count, "featured": true,
            }));
        }
    }
    for c in cities {
        featured_rows.push(json!({
            "slug": c.slug, "name": c.name, "city": c.city,
            "business_count": c.business_count, "featured": false,
        }));
    }
    featured_rows
}

/// GET /api/v1/homepage/config?directory_id=<uuid> — PUBLIC read of the resolved homepage config.
pub async fn get_public(
    State(s): State<AppState>,
    Query(q): Query<PublicQuery>,
) -> ApiResult<Json<Value>> {
    let (directory_id, network_id) = match q.directory_id {
        Some(id) => {
            let ctx = directory_context(&s.db, id).await?;
            match ctx {
                Some((_, net, _)) => (Some(id), net),
                None => (None, None),
            }
        }
        None => (None, default_network_id(&s.db).await?),
    };

    let row = resolve_effective(&s.db, directory_id, network_id).await?;
    let cities = home_cities(&s.db, network_id, directory_id).await?;
    let featured = row
        .as_ref()
        .map(|r| r.featured_city_slugs.clone())
        .unwrap_or_default();
    let city_rows = order_cities(cities, &featured);

    let sections = normalise_sections(row.as_ref().map(|r| &r.sections));
    let (announcement, hero, surface, surface_city) = match &row {
        Some(r) => {
            let surface = r.home_surface.clone();
            let surface_city = if surface == "city" {
                r.home_city_slug
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| featured.first().cloned())
            } else {
                None
            };
            (
                json!({
                    "text": r.announcement_text,
                    "cta_text": r.announcement_cta_text,
                    "cta_url": r.announcement_cta_url,
                }),
                json!({
                    "headline": r.hero_headline,
                    "subheadline": r.hero_subheadline,
                    "image_url": r.hero_image_url,
                    "cta_text": r.hero_cta_text,
                    "cta_url": r.hero_cta_url,
                }),
                surface,
                surface_city,
            )
        }
        None => (
            json!({ "text": null, "cta_text": null, "cta_url": null }),
            json!({ "headline": null, "subheadline": null, "image_url": null, "cta_text": null, "cta_url": null }),
            "network".to_string(),
            None,
        ),
    };

    let network = match network_id {
        Some(nid) => {
            sqlx::query_as::<_, (String, String)>("SELECT name, slug FROM networks WHERE id = $1")
                .bind(nid)
                .fetch_optional(&s.db)
                .await?
                .map(|(name, slug)| json!({ "id": nid, "name": name, "slug": slug }))
        }
        None => None,
    };

    Ok(Json(json!({
        "enabled": row.as_ref().map(|r| r.enabled).unwrap_or(true),
        "configured": row.is_some(),
        "home_surface": surface,
        "home_city_slug": surface_city,
        "announcement": announcement,
        "hero": hero,
        "sections": sections,
        "cities": city_rows,
        "network": network,
    })))
}

/// GET /api/v1/homepage-config/settings?scope=network|directory&id=<uuid> — admin (operator) read.
pub async fn get_settings(
    State(s): State<AppState>,
    Query(q): Query<SettingsQuery>,
) -> ApiResult<Json<Value>> {
    let (directory_id, network_id) = scope_ids(&s.db, &q.scope, q.id).await?;
    let row = resolve_exact(&s.db, directory_id, network_id).await?;
    let inherited = if row.is_none() {
        resolve_effective(&s.db, directory_id, network_id).await?
    } else {
        None
    };
    let cities = home_cities(&s.db, network_id, directory_id).await?;
    let city_rows: Vec<Value> = cities
        .into_iter()
        .map(|c| json!({ "slug": c.slug, "name": c.name, "city": c.city, "business_count": c.business_count }))
        .collect();

    Ok(Json(json!({
        "scope": q.scope,
        "id": q.id,
        "exists": row.is_some(),
        "row": row,
        "inherited": inherited,
        "defaults": { "sections": default_sections() },
        "section_keys": SECTION_KEYS,
        "cities": city_rows,
    })))
}

/// PUT /api/v1/homepage-config/settings — admin (operator) upsert.
pub async fn put_settings(
    State(s): State<AppState>,
    Json(body): Json<SaveSettings>,
) -> ApiResult<Json<Value>> {
    let (directory_id, network_id) = scope_ids(&s.db, &body.scope, body.id).await?;
    let existing = resolve_exact(&s.db, directory_id, network_id).await?;

    let surface = body
        .home_surface
        .clone()
        .unwrap_or_else(|| "network".to_string());
    if surface != "network" && surface != "city" {
        return Err(AppError::BadRequest(
            "home_surface must be 'network' or 'city'".into(),
        ));
    }
    if surface == "city" {
        let slug = body
            .home_city_slug
            .clone()
            .or_else(|| existing.as_ref().and_then(|r| r.home_city_slug.clone()))
            .unwrap_or_default();
        if slug.trim().is_empty() {
            return Err(AppError::BadRequest(
                "home_surface 'city' needs a home_city_slug".into(),
            ));
        }
    }

    // A directory row must carry directory_id and NO network_id (owner check) — same rule as
    // loyalty_messaging, where binding both 500s every directory-scope save.
    let owner_network = if directory_id.is_some() {
        None
    } else {
        network_id
    };

    let sections = normalise_sections(
        body.sections
            .as_ref()
            .or_else(|| existing.as_ref().map(|r| &r.sections)),
    );

    let row = match existing {
        Some(cur) => {
            const SQL: &str = concat!(
                "UPDATE homepage_config SET \
                   enabled = $2, home_surface = $3, home_city_slug = $4, \
                   announcement_text = $5, announcement_cta_text = $6, announcement_cta_url = $7, \
                   hero_headline = $8, hero_subheadline = $9, hero_image_url = $10, \
                   hero_cta_text = $11, hero_cta_url = $12, featured_city_slugs = $13, \
                   sections = $14, updated_at = now() \
                 WHERE id = $1 RETURNING ",
                config_cols!()
            );
            sqlx::query_as::<_, HomepageConfigRow>(SQL)
                .bind(cur.id)
                .bind(body.enabled.unwrap_or(cur.enabled))
                .bind(&surface)
                .bind(body.home_city_slug.clone().or(cur.home_city_slug))
                .bind(body.announcement_text.clone().or(cur.announcement_text))
                .bind(
                    body.announcement_cta_text
                        .clone()
                        .or(cur.announcement_cta_text),
                )
                .bind(
                    body.announcement_cta_url
                        .clone()
                        .or(cur.announcement_cta_url),
                )
                .bind(body.hero_headline.clone().or(cur.hero_headline))
                .bind(body.hero_subheadline.clone().or(cur.hero_subheadline))
                .bind(body.hero_image_url.clone().or(cur.hero_image_url))
                .bind(body.hero_cta_text.clone().or(cur.hero_cta_text))
                .bind(body.hero_cta_url.clone().or(cur.hero_cta_url))
                .bind(
                    body.featured_city_slugs
                        .clone()
                        .unwrap_or(cur.featured_city_slugs),
                )
                .bind(sections)
                .fetch_one(&s.db)
                .await?
        }
        None => {
            const SQL: &str = concat!(
                "INSERT INTO homepage_config \
                   (network_id, directory_id, enabled, home_surface, home_city_slug, \
                    announcement_text, announcement_cta_text, announcement_cta_url, \
                    hero_headline, hero_subheadline, hero_image_url, hero_cta_text, hero_cta_url, \
                    featured_city_slugs, sections) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) \
                 RETURNING ",
                config_cols!()
            );
            sqlx::query_as::<_, HomepageConfigRow>(SQL)
                .bind(owner_network)
                .bind(directory_id)
                .bind(body.enabled.unwrap_or(true))
                .bind(&surface)
                .bind(body.home_city_slug.clone())
                .bind(body.announcement_text.clone())
                .bind(body.announcement_cta_text.clone())
                .bind(body.announcement_cta_url.clone())
                .bind(body.hero_headline.clone())
                .bind(body.hero_subheadline.clone())
                .bind(body.hero_image_url.clone())
                .bind(body.hero_cta_text.clone())
                .bind(body.hero_cta_url.clone())
                .bind(body.featured_city_slugs.clone().unwrap_or_default())
                .bind(sections)
                .fetch_one(&s.db)
                .await?
        }
    };

    Ok(Json(json!({ "status": "saved", "row": row })))
}

/// Map (scope, id) -> (directory_id, network_id) for the storage row.
async fn scope_ids(
    db: &PgPool,
    scope: &str,
    id: Uuid,
) -> Result<(Option<Uuid>, Option<Uuid>), AppError> {
    match scope {
        "network" => Ok((None, Some(id))),
        "directory" => {
            let ctx = directory_context(db, id).await?;
            let network_id = ctx.and_then(|(_, net, _)| net);
            Ok((Some(id), network_id))
        }
        other => Err(AppError::BadRequest(format!(
            "Unknown scope '{}' — use 'network' or 'directory'",
            other
        ))),
    }
}
