//! Subfolder SEO — directories served at `/<slug>` under the network domain.
//!
//! David's requirement (2026-09-23): a directory must be reachable at
//! `zaarhub.com/<slug>` (e.g. `/palm-bay`) with its sub-pages under the same
//! prefix (`/palm-bay/businesses/...`), and the canonical URL must be the
//! subfolder form so search engines never see duplicate URLs.
//!
//! Everything in here is generated **automatically from the record's own
//! fields** at read time — a new business or a newly generated article gets
//! title, meta description, canonical, Open Graph/Twitter tags, JSON-LD and a
//! sitemap entry with **no manual step**. Admin overrides are honoured when a
//! row exists in `seo_meta` (the platform's existing, UI-editable SEO store),
//! so nothing is hardcoded and no provider/tenant domain is assumed.
//!
//! This module never panics on missing data; every field falls back to a
//! derived value and every query failure degrades to "not found" rather than
//! a 500.

use crate::brand_theme::{site_headquarters, BrandTheme};
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use sqlx::{PgPool, Row};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Escaping helpers — HTML text, HTML attribute and JSON-string safety.
// ─────────────────────────────────────────────────────────────────────────────

/// Escape text for an HTML text node / attribute value.
pub fn h(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Truncate on a word boundary for meta descriptions.
fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for w in s.split_whitespace() {
        if out.chars().count() + w.chars().count() + 1 > max {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.is_empty() {
        s.chars().take(max).collect()
    } else {
        format!("{}…", out)
    }
}

/// Strip HTML tags to plain text (for meta descriptions derived from HTML body).
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ─────────────────────────────────────────────────────────────────────────────
// Origin / URL construction — derived from the request Host, never hardcoded.
// ─────────────────────────────────────────────────────────────────────────────

/// Build the site origin (`https://host`) from the request host + forwarded proto.
/// A private/loopback host keeps its port (test/dev); public hosts drop it.
/// Public origin (scheme://host) for absolute URLs. `fallback` is used when the
/// request carries no host. Shared with the SEO sitemap builder so both always
/// name the same, reachable host.
pub fn origin(host: Option<&str>, proto: &str, fallback: &str) -> String {
    // kanban t_0b3cce4d — ONE canonical host PER DIRECTORY, resolved from
    // `networks.root_domain` (data), never from a compile-time constant. This app
    // (and this vhost) serves the public directory on several hostnames: the brand
    // host, its `www`, and directory.swiftsoftware.net. The last one is ALSO the
    // factory's home — the software that creates directories, plus its admin console
    // — so it keeps answering and is never redirected away. Deriving the origin from
    // the request Host made each hostname self-canonicalise the same page, which is
    // why Search Console reported 8 "Duplicate without user-selected canonical" rows.
    // A host that names a brand (or the factory host of a single-directory
    // deployment) now gets that brand's origin; a site-facing path on the factory host
    // is re-pointed per directory by `seo_canonical::seo_headers`, which knows the
    // path. Any other host keeps its own origin: a directory mapped to its own domain,
    // and localhost / 127.x used by the probe harness and local tests.
    if let Some(brand) = crate::seo_canonical::brand_origin_for_host(host) {
        return brand;
    }
    let scheme = if proto.eq_ignore_ascii_case("http") {
        "http"
    } else {
        "https"
    };
    let raw = host.filter(|h| !h.trim().is_empty()).unwrap_or(fallback);
    let raw = raw.trim();
    let hostname = raw.split(':').next().unwrap_or(raw);
    let is_private = hostname == "localhost"
        || hostname.starts_with("127.")
        || hostname.starts_with("192.168.")
        || hostname.starts_with("10.")
        || hostname.starts_with("172.");
    let clean = if is_private { raw } else { hostname };
    format!("{}://{}", scheme, clean)
}

// ─────────────────────────────────────────────────────────────────────────────
// Collision detection — a directory slug must not shadow a top-level route.
// ─────────────────────────────────────────────────────────────────────────────

/// Top-level names that already own `/<name>` (router routes, static portals,
/// API namespaces and the served static files). A directory whose slug matches
/// one of these cannot be reached at `/<slug>`.
pub const RESERVED_TOP_LEVEL: &[&str] = &[
    "api",
    "admin",
    "admin-panel",
    "admin-panel.html",
    "admin-ops",
    "admin-ops.html",
    "admin-dashboard.html",
    "admin-login.html",
    "login",
    "login.html",
    "logout",
    "zaarhub",
    "zaarhub-sitemap.xml",
    "zaarhub-robots.txt",
    "sitemap.xml",
    "robots.txt",
    "legal",
    "rfq-marketplace",
    "coop-hub",
    "lead-exchange",
    "feed",
    "feed-page",
    "events",
    "events-page",
    "saved-places",
    "health",
    "index.html",
    "portal.html",
    "business-portal.html",
    "business-dashboard.html",
    "business-detail.html",
    "visitor-portal.html",
    "supplier-portal.html",
    "supplier-directory.html",
    "submit-business.html",
    "scanner.html",
    "scanner-manifest.json",
    "claim.html",
    "b2b-marketplace.html",
    "blog-features",
    "blog-features-admin.html",
    "content-research.html",
    "research",
    "pricing-admin.html",
    "guide.html",
    "grow.html",
    "rfq.html",
    "pricing",
    "guides",
    "assets",
    "static",
    "favicon.svg",
    "logo.png",
];

/// True when `slug` would collide with an existing top-level route/file.
pub fn slug_is_reserved(slug: &str) -> Option<&'static str> {
    RESERVED_TOP_LEVEL
        .iter()
        .find(|r| r.eq_ignore_ascii_case(slug))
        .copied()
}

/// All active directory slugs that currently collide with a reserved top-level
/// name. Used by the admin report endpoint and the boot-time warning.
pub async fn find_clashes(pool: &PgPool) -> Vec<(String, String)> {
    let rows = sqlx::query("SELECT slug FROM directories WHERE status = 'active'")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in rows {
        let slug: String = r.try_get("slug").unwrap_or_default();
        if let Some(reserved) = slug_is_reserved(&slug) {
            out.push((slug, reserved.to_string()));
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Record lookups
// ─────────────────────────────────────────────────────────────────────────────

struct DirectoryRec {
    id: Uuid,
    name: String,
    slug: String,
    city: String,
    state: String,
    description: Option<String>,
    /// The network this directory belongs to. `Some` ⇒ the directory is part
    /// of a network and shares that network's brand (home + cities consistent);
    /// `None` ⇒ standalone, so it uses its own `directory_branding` row.
    network_id: Option<Uuid>,
}

async fn load_directory(pool: &PgPool, slug: &str) -> Option<DirectoryRec> {
    let r = sqlx::query(
        "SELECT id, name, slug, city, state, description, network_id FROM directories \
         WHERE slug = $1 AND status = 'active' LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let dir_name: String = r.try_get("name").unwrap_or_default();
    let dir_slug: String = r.try_get("slug").unwrap_or_default();
    let dir_city: Option<String> = r.try_get("city").unwrap_or(None);
    let dir_state: String = r
        .try_get::<Option<String>, _>("state")
        .unwrap_or(None)
        .unwrap_or_else(|| "FL".to_string());
    let dir_city = dir_city
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| dir_name.clone());
    // Normalise "Palm Bay, FL" → "Palm Bay"
    let dir_city = dir_city
        .split(',')
        .next()
        .unwrap_or(&dir_city)
        .trim()
        .to_string();

    Some(DirectoryRec {
        id: r.try_get("id").ok()?,
        name: dir_name,
        slug: dir_slug,
        city: dir_city,
        state: dir_state,
        description: r.try_get("description").unwrap_or(None),
        network_id: r.try_get("network_id").unwrap_or(None),
    })
}

// B88 / t_68393020: the homepage's crawlable city list now lives inside
// `home_ssr_html` (one block with the h1, intro, city links, featured
// businesses and categories), so the standalone `city_links_html` list was
// retired rather than kept as a second, thinner copy of the same links.

/// Join names the way a person writes a list: `a, b, c and d`.
fn join_natural(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// The homepage's server-rendered hub block.
///
/// `/` is served from the SPA shell, which ships no prose, no real heading and
/// no listings in the initial HTML: a crawler that never executes JavaScript
/// saw ~460 characters and zero business links, while every city root now
/// carries 3.5k+. This builds a real hub page body for the same shell — one
/// `<h1>`, an introduction sized to the live catalogue, every active city as a
/// crawlable `<a href="/{slug}">`, the best-rated business in each city as a
/// card pointing at its real `/{city}/businesses/{slug}` page, and the
/// categories people search for most. `index.html` removes `#zh-home-ssr` on
/// boot, so a visitor sees the SPA and a crawler sees the content.
pub async fn home_ssr_html(pool: &PgPool) -> String {
    struct City {
        slug: String,
        city: String,
        state: String,
        count: i64,
    }

    let rows = sqlx::query(
        "SELECT d.slug, d.city, COALESCE(d.state, 'FL') AS state, \
                (SELECT COUNT(*) FROM businesses b \
                  WHERE b.directory_id = d.id AND b.is_active = true) AS listing_count \
         FROM directories d \
         WHERE (d.status = 'active' OR d.status IS NULL) \
           AND d.slug IS NOT NULL AND d.slug <> '' \
         ORDER BY d.city",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| eprintln!("subfolder: homepage city query failed: {e}"))
    .unwrap_or_default();

    let mut cities: Vec<City> = Vec::new();
    for r in &rows {
        let slug: String = r.try_get("slug").unwrap_or_default();
        let city: String = r.try_get("city").unwrap_or_default();
        if slug.is_empty() || city.is_empty() {
            continue;
        }
        cities.push(City {
            slug,
            city,
            state: r.try_get("state").unwrap_or_default(),
            count: r.try_get("listing_count").unwrap_or(0),
        });
    }
    if cities.is_empty() {
        return String::new();
    }

    // One best-rated business per city, so the block covers the whole network
    // instead of ten cards from whichever city happens to be biggest.
    let pick_rows = sqlx::query(
        "SELECT DISTINCT ON (d.id) \
                b.name, b.slug, b.rating, b.review_count, dc.name AS category, \
                b.address, d.slug AS city_slug, d.city \
         FROM businesses b \
         JOIN directories d ON d.id = b.directory_id \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE b.is_active = true AND d.status = 'active' \
           AND d.slug IS NOT NULL AND d.slug <> '' \
           AND b.rating IS NOT NULL AND b.rating > 0 \
           AND b.slug IS NOT NULL AND b.slug <> '' \
         ORDER BY d.id, b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST \
         LIMIT 12",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| eprintln!("subfolder: homepage featured query failed: {e}"))
    .unwrap_or_default();

    let mut picks: Vec<(String, String, String, String, f64, i32, String, String)> = Vec::new();
    for p in &pick_rows {
        let name: String = p.try_get("name").unwrap_or_default();
        let slug: String = p.try_get("slug").unwrap_or_default();
        let city_slug: String = p.try_get("city_slug").unwrap_or_default();
        if name.is_empty() || slug.is_empty() || city_slug.is_empty() {
            continue;
        }
        picks.push((
            name,
            slug,
            city_slug,
            p.try_get("city").unwrap_or_default(),
            p.try_get::<Option<f64>, _>("rating")
                .unwrap_or(None)
                .unwrap_or(0.0),
            p.try_get::<Option<i32>, _>("review_count")
                .unwrap_or(None)
                .unwrap_or(0),
            p.try_get::<Option<String>, _>("category")
                .unwrap_or(None)
                .unwrap_or_default(),
            p.try_get::<Option<String>, _>("address")
                .unwrap_or(None)
                .unwrap_or_default(),
        ));
    }
    picks.sort_by_key(|p| std::cmp::Reverse(p.5));

    // The categories people search for most, each pointed at the city where it
    // has the deepest listing base so the link lands on a page with content.
    let cat_rows = sqlx::query(
        "SELECT dc.slug AS cat_slug, dc.name AS cat_name, d.slug AS city_slug, d.city, \
                COUNT(*) AS n \
         FROM businesses b \
         JOIN directories d ON d.id = b.directory_id \
         JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE b.is_active = true AND d.status = 'active' \
           AND d.slug IS NOT NULL AND d.slug <> '' \
           AND dc.slug IS NOT NULL AND dc.slug <> '' \
         GROUP BY 1, 2, 3, 4 \
         ORDER BY n DESC \
         LIMIT 60",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| eprintln!("subfolder: homepage category query failed: {e}"))
    .unwrap_or_default();

    let mut seen_cats: Vec<String> = Vec::new();
    let mut cats: Vec<(String, String, String, String, i64)> = Vec::new();
    for c in &cat_rows {
        let cat_slug: String = c.try_get("cat_slug").unwrap_or_default();
        let city_slug: String = c.try_get("city_slug").unwrap_or_default();
        if cat_slug.is_empty() || city_slug.is_empty() || seen_cats.contains(&cat_slug) {
            continue;
        }
        seen_cats.push(cat_slug.clone());
        cats.push((
            cat_slug,
            c.try_get("cat_name").unwrap_or_default(),
            city_slug,
            c.try_get("city").unwrap_or_default(),
            c.try_get("n").unwrap_or(0),
        ));
        if cats.len() >= 10 {
            break;
        }
    }

    let total: i64 = cities.iter().map(|c| c.count).sum();
    let n_cities = cities.len();
    let region = if cities.iter().all(|c| c.state == "FL") {
        "Florida"
    } else {
        "the areas we cover"
    };
    let place_phrase = join_natural(&cities.iter().map(|c| c.city.clone()).collect::<Vec<_>>());

    // ── City cards ───────────────────────────────────────────────────────────
    let mut city_cards = String::new();
    for c in &cities {
        city_cards.push_str(&format!(
            "<a href=\"/{slug}\" style=\"display:block;padding:14px 16px;border:1px solid #e5e7eb;\
border-radius:12px;text-decoration:none;color:inherit;background:#fff\">\
<strong style=\"display:block;font-size:1rem\">{city}</strong>\
<span style=\"font-size:.85rem;color:#6b7280\">{count} businesses \u{2192}</span></a>",
            slug = h(&c.slug),
            city = h(&c.city),
            count = c.count
        ));
    }

    // ── Featured business cards ──────────────────────────────────────────────
    let mut pick_cards = String::new();
    for (name, slug, city_slug, city, rating, reviews, category, address) in &picks {
        let cat_html = if category.is_empty() {
            String::new()
        } else {
            format!("{} \u{00b7} ", h(category))
        };
        let addr_html = if address.is_empty() {
            String::new()
        } else {
            format!(
                "<span style=\"display:block;font-size:.8rem;color:#9ca3af\">{}</span>",
                h(&clip(address, 64))
            )
        };
        pick_cards.push_str(&format!(
            "<a href=\"/{city_slug}/businesses/{slug}\" style=\"display:block;padding:14px 16px;\
border:1px solid #e5e7eb;border-radius:12px;text-decoration:none;color:inherit;background:#fff\">\
<strong style=\"display:block;font-size:1rem\">{name}</strong>\
<span style=\"display:block;font-size:.82rem;color:#ea580c\">{cat}\u{2605} {rating:.1} ({reviews} reviews)</span>\
<span style=\"display:block;font-size:.8rem;color:#6b7280\">{city}</span>{addr}</a>",
            city_slug = h(city_slug),
            slug = h(slug),
            name = h(name),
            cat = cat_html,
            rating = rating,
            reviews = reviews,
            city = h(city),
            addr = addr_html
        ));
    }

    // ── Category chips ───────────────────────────────────────────────────────
    let mut cat_chips = String::new();
    for (cat_slug, cat_name, city_slug, city, n) in &cats {
        cat_chips.push_str(&format!(
            "<a href=\"/{city_slug}/{cat_slug}\" style=\"display:inline-block;padding:7px 13px;\
border:1px solid #e5e7eb;border-radius:999px;text-decoration:none;color:inherit;\
background:#fff;font-size:.88rem\">{name} in {city} <span style=\"color:#9ca3af\">({n})</span></a>",
            city_slug = h(city_slug),
            cat_slug = h(cat_slug),
            name = h(cat_name),
            city = h(city),
            n = n
        ));
    }

    let picks_section = if pick_cards.is_empty() {
        String::new()
    } else {
        format!(
            "<h2 style=\"font-size:1.2rem;margin:34px 0 12px\">Top-rated businesses right now</h2>\
<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:12px\">{pick_cards}</div>"
        )
    };
    let cats_section = if cat_chips.is_empty() {
        String::new()
    } else {
        format!(
            "<h2 style=\"font-size:1.2rem;margin:34px 0 12px\">What people search for most</h2>\
<div style=\"display:flex;flex-wrap:wrap;gap:8px\">{cat_chips}</div>"
        )
    };

    format!(
        "<section id=\"zh-home-ssr\" aria-label=\"About this directory\" \
style=\"max-width:1100px;margin:24px auto 48px;padding:0 20px;font-family:inherit;\
line-height:1.6;color:#111827\">\
<h1 style=\"font-size:clamp(1.5rem,3vw,2rem);margin:0 0 14px\">\
Find a local business in {n_cities} {region} cities</h1>\
<p style=\"margin:0 0 12px;max-width:70ch\">ZaarHub is a local business directory organised city by \
city: {place_phrase}. Between them these {n_cities} directories hold {total} active listings \u{2014} \
restaurants and cafés, dentists, hair salons, tattoo studios, gyms, contractors, law firms, med spas \
and more \u{2014} each with its address, phone number, opening hours, category and the customer \
rating it has earned.</p>\
<p style=\"margin:0 0 12px;max-width:70ch\">Every city below is its own directory with its own front \
page, its own category pages, local articles and weekly deals, so the business you find is one that \
actually serves your street rather than the nearest big-city list. Pick the town you live in, choose \
the category you need, and every listing opens a full profile you can call, save, share or review.</p>\
<p style=\"margin:0 0 12px;max-width:70ch\">If you own a business the listing is free: add it in a \
couple of minutes, or claim the page that is already here, and then keep your hours, photographs and \
offers current. The same listing appears on your city page, in search results, and in the local \
newsletter that goes out each week.</p>\
<h2 style=\"font-size:1.2rem;margin:30px 0 12px\">Browse a city</h2>\
<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:10px\">{city_cards}</div>\
{picks_section}\
{cats_section}\
<p style=\"margin:26px 0 0;max-width:70ch;font-size:.92rem;color:#4b5563\">ZaarHub is independent: \
listings are found and maintained by our own team rather than sold in bulk, every page carries the \
rating and review count it has actually earned, and any business can claim its own page at any time \
\u{2014} free of charge.</p>\
</section>",
        n_cities = n_cities,
        region = region,
        place_phrase = h(&place_phrase),
        total = total,
        city_cards = city_cards,
        picks_section = picks_section,
        cats_section = cats_section,
    )
}

/// The brand tokens this directory's pages render with. Resolved from the
/// shared source of truth (`crate::brand_theme`) so a city page can never
/// drift from the network homepage.
async fn theme_for(pool: &PgPool, dir: &DirectoryRec) -> BrandTheme {
    crate::brand_theme::theme_for_directory(pool, dir.id, dir.network_id).await
}

/// Admin-editable SEO override from `seo_meta` (the platform's existing store).
struct SeoOverride {
    title: Option<String>,
    description: Option<String>,
    og_image: Option<String>,
    custom_schema: Option<serde_json::Value>,
}

async fn seo_override(pool: &PgPool, page_type: &str, page_id: Uuid) -> Option<SeoOverride> {
    let r = sqlx::query(
        "SELECT title, description, og_image, custom_schema FROM seo_meta \
         WHERE page_type = $1 AND page_id = $2 LIMIT 1",
    )
    .bind(page_type)
    .bind(page_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    Some(SeoOverride {
        title: r.try_get("title").unwrap_or(None),
        description: r.try_get("description").unwrap_or(None),
        og_image: r.try_get("og_image").unwrap_or(None),
        custom_schema: r.try_get("custom_schema").unwrap_or(None),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Page shell
// ─────────────────────────────────────────────────────────────────────────────

/// Admin-authored SEO pattern for one page type (kanban B119), rendered through the
/// shared merge-field engine. A single directory-wide pattern such as
/// `{{city}} Business Directory | ZaarHub` therefore serves every city page, while an
/// explicit per-page `seo_meta` override still wins. `extra` adds surface-specific
/// values the directory context cannot know (e.g. `category`, `business_name`).
/// A pattern that resolves to nothing usable returns `None`, so the caller's own
/// default still renders instead of a blank title.
async fn seo_pattern(
    pool: &PgPool,
    directory_id: Uuid,
    page_type: &str,
    extra: &[(&str, &str)],
) -> (Option<String>, Option<String>) {
    let row = sqlx::query(
        "SELECT title_template, description_template FROM seo_fallback_templates \
         WHERE directory_id = $1 AND page_type = $2 LIMIT 1",
    )
    .bind(directory_id)
    .bind(page_type)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some(row) = row else {
        return (None, None);
    };
    let raw_t: Option<String> = row.try_get("title_template").unwrap_or(None);
    let raw_d: Option<String> = row.try_get("description_template").unwrap_or(None);

    let mut ctx = crate::merge_fields::MergeContext::for_directory(pool, directory_id).await;
    for (k, v) in extra {
        if !v.trim().is_empty() {
            ctx.set(k, *v);
        }
    }

    let render_one = |raw: Option<String>| -> Option<String> {
        let raw = raw?;
        if raw.trim().is_empty() {
            return None;
        }
        let out = crate::merge_fields::render(&raw, &ctx);
        let trimmed = out.text.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    };
    (render_one(raw_t), render_one(raw_d))
}

struct Seo {
    title: String,
    description: String,
    canonical: String,
    og_type: String,
    og_image: Option<String>,
    jsonld: Vec<serde_json::Value>,
}

fn head_html(seo: &Seo, site_name: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("<title>{}</title>\n", h(&seo.title)));
    s.push_str(&format!(
        "<meta name=\"description\" content=\"{}\">\n",
        h(&seo.description)
    ));
    s.push_str(&format!(
        "<link rel=\"canonical\" href=\"{}\">\n",
        h(&seo.canonical)
    ));
    s.push_str("<meta name=\"robots\" content=\"index,follow,max-image-preview:large\">\n");
    // Every server-rendered page names the site icon explicitly; without it a
    // browser falls back to /favicon.ico (404) and logs a console error.
    s.push_str("<link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.svg\">\n");
    s.push_str(&format!(
        "<meta property=\"og:site_name\" content=\"{}\">\n",
        h(site_name)
    ));
    s.push_str(&format!(
        "<meta property=\"og:title\" content=\"{}\">\n",
        h(&seo.title)
    ));
    s.push_str(&format!(
        "<meta property=\"og:description\" content=\"{}\">\n",
        h(&seo.description)
    ));
    s.push_str(&format!(
        "<meta property=\"og:type\" content=\"{}\">\n",
        h(&seo.og_type)
    ));
    s.push_str(&format!(
        "<meta property=\"og:url\" content=\"{}\">\n",
        h(&seo.canonical)
    ));
    if let Some(img) = &seo.og_image {
        s.push_str(&format!(
            "<meta property=\"og:image\" content=\"{}\">\n",
            h(img)
        ));
        s.push_str(&format!(
            "<meta name=\"twitter:image\" content=\"{}\">\n",
            h(img)
        ));
    }
    s.push_str("<meta name=\"twitter:card\" content=\"summary_large_image\">\n");
    s.push_str(&format!(
        "<meta name=\"twitter:title\" content=\"{}\">\n",
        h(&seo.title)
    ));
    s.push_str(&format!(
        "<meta name=\"twitter:description\" content=\"{}\">\n",
        h(&seo.description)
    ));
    for ld in &seo.jsonld {
        // serde_json escaping is JSON-safe; `<` is escaped to prevent `</script>` breakout.
        let json = ld.to_string().replace('<', "\\u003c");
        s.push_str(&format!(
            "<script type=\"application/ld+json\">{}</script>\n",
            json
        ));
    }
    s
}

/// Remove the first `open…close` span from `s` (e.g. the document's own
/// `<title>…</title>`), leaving the rest untouched.
fn strip_first_between(s: &str, open: &str, close: &str) -> String {
    match (s.find(open), s.find(close)) {
        (Some(a), Some(b)) if b >= a + open.len() => {
            let mut out = String::with_capacity(s.len());
            out.push_str(&s[..a]);
            out.push_str(&s[b + close.len()..]);
            out
        }
        _ => s.to_string(),
    }
}

/// The city hub's own visible content, server-rendered **inside** the SPA's `#app`
/// element. Measured 2026-11-11: `/<city>` shipped the 248 KB JavaScript shell whose
/// only visible text was the trail and the category chips (~280 characters), so the
/// eleven city URLs in the sitemap — the pages named after the money query — carried
/// no businesses in the HTML they served, while every page below them was fully
/// server-rendered. The SPA replaces `#app` on boot, so a visitor with JavaScript
/// sees exactly the app they saw before and nobody ever sees the list twice; a
/// crawler, a social scraper or a visitor without JavaScript gets the real listings.
/// The city hub's server-rendered body (card W36 + CITY-HUB-TEXT).
///
/// `intro` is the city's own written introduction (`city_pages.intro_copy`, migration 188),
/// printed paragraph by paragraph under the heading. It is the reason a root city page now
/// carries real prose instead of a count sentence plus a grid of names: measured 2026-11-12 the
/// served HTML held 1,842 characters of visible text, far below what a page needs to rank for a
/// commercial local query or to be useful to somebody who landed on it from a search result.
fn city_hub_html(
    dir: &DirectoryRec,
    site: &str,
    total: i64,
    top: &[BizCard],
    intro: Option<&str>,
) -> String {
    let place = if dir.state.trim().is_empty() {
        h(&dir.city)
    } else {
        format!("{}, {}", h(&dir.city), h(&dir.state))
    };
    let mut out = String::with_capacity(4096);
    // The shell's full-screen splash is normally removed by the app at boot; without
    // JavaScript it would sit on top of this content forever, so hide it only then.
    out.push_str("<noscript><style>#splash-load{display:none!important}</style></noscript>");
    out.push_str(
        "<div style=\"max-width:1180px;margin:0 auto;padding:26px 18px 6px;text-align:left\">",
    );
    out.push_str(&format!(
        "<h1 style=\"font-size:1.9rem;font-weight:800;color:#0f172a;margin:0 0 10px;line-height:1.2\">{} Business Directory</h1>",
        place
    ));
    let mut lede = format!(
        "{} local business{} in {}, with ratings, addresses and contact details.",
        total,
        if total == 1 { "" } else { "es" },
        place
    );
    if let Some(d) = dir.description.as_deref() {
        let d = clip(&strip_tags(d), 220);
        if !d.trim().is_empty() {
            lede = format!("{} {}", h(&d), lede);
        }
    }
    out.push_str(&format!(
        "<p style=\"font-size:.95rem;color:#475569;margin:0 0 18px;max-width:70ch\">{}</p>",
        lede
    ));

    // The city's own introduction — hand-written per city (migration 188), a blank line
    // separating paragraphs. It is deliberately NOT derived from `description`: that column is
    // the 158-character meta description and stays exactly as it was (requirement 5).
    if let Some(intro) = intro.map(str::trim).filter(|i| !i.is_empty()) {
        for para in intro.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
            out.push_str(&format!(
                "<p style=\"font-size:.95rem;color:#334155;margin:0 0 14px;max-width:70ch\">{}</p>",
                h(para)
            ));
        }
    }

    if !top.is_empty() {
        out.push_str(&format!(
            "<h2 style=\"font-size:1.15rem;font-weight:700;color:#0f172a;margin:0 0 10px\">Top-rated businesses in {}</h2>",
            place
        ));
        out.push_str("<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:10px;margin:0 0 18px\">");
        for b in top {
            let url = format!("/{}/businesses/{}", dir.slug, b.slug);
            let mut meta = Vec::new();
            if let Some(c) = b.category.as_deref().filter(|c| !c.trim().is_empty()) {
                meta.push(h(c));
            }
            if b.rating > 0.0 {
                meta.push(format!(
                    "★ {:.1} ({} review{})",
                    b.rating,
                    b.review_count,
                    if b.review_count == 1 { "" } else { "s" }
                ));
            } else {
                meta.push("No reviews yet".to_string());
            }
            if let Some(a) = b.address.as_deref().filter(|a| !a.trim().is_empty()) {
                meta.push(h(a));
            }
            // Short description (requirement 2: name, category, short description, link).
            // Clipped so a business that pasted its whole history cannot blow the grid apart.
            let desc = b
                .description
                .as_deref()
                .map(|d| clip(&strip_tags(d), 170))
                .filter(|d| !d.trim().is_empty())
                .map(|d| {
                    format!(
                        "<p style=\"margin:6px 0 0;font-size:.84rem;color:#475569;line-height:1.45\">{}</p>",
                        h(d.trim())
                    )
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "<article style=\"background:#fff;border:1px solid #e2e8f0;border-radius:12px;padding:12px 14px\">\
                 <h3 style=\"margin:0 0 4px;font-size:1rem;font-weight:700;line-height:1.3\">\
                 <a href=\"{url}\" style=\"color:#116dc0;text-decoration:none\">{name}</a></h3>\
                 <div style=\"font-size:.82rem;color:#64748b;line-height:1.5\">{meta}</div>{desc}</article>",
                url = url,
                name = h(&b.name),
                meta = meta.join(" · "),
                desc = desc
            ));
        }
        out.push_str("</div>");
    }

    out.push_str(&format!(
        "<p style=\"margin:0 0 26px\"><a href=\"/{slug}/businesses\" style=\"color:#ea580c;font-weight:600;text-decoration:none\">See all {total} businesses in {place} →</a></p>",
        slug = dir.slug,
        total = total,
        place = place
    ));
    let _ = site; // the brand is already in the page's <title>/head; not repeated in the body
    out.push_str("</div>");
    out
}

/// Render a city page by taking the **homepage document** (`frontend/index.html`,
/// the shared component library) and giving it the city's own SEO head. The
/// markup, CSS, navigation and component set are therefore byte-identical to the
/// homepage — only the data (loaded by the SPA from the city endpoint) and the
/// head tags are city-scoped. This is what stops the two surfaces drifting.
fn inject_city_document(
    spa_html: &str,
    seo: &Seo,
    site_name: &str,
    theme: &BrandTheme,
    dir_id: Uuid,
    nav_html: &str,
    app_html: &str,
) -> String {
    let head = head_html(seo, site_name);
    // Attribute the entry view to this city, server-side. The visitor beacon fires its first
    // page_view at boot — before the SPA has resolved the city from the path
    // (`/api/v1/zaarhub/cities/<slug>` sets window.__dir_id only later) — so without this the
    // whole first view AND its session row landed with no directory and the demand product
    // read "unknown" for every city. window.__dir_id is exactly what the beacon reads when it
    // starts the session; deferred scripts run before window 'load', so the value survives.
    let attr =
        format!("<script>window.__dir_id=\"{dir_id}\";window.__ssr_dir=\"{dir_id}\";</script>");
    let theme_tag = format!("<style id=\"brand-theme\">{}</style>", theme.css_block());
    // The homepage's own <title> is dropped (the city's is the first line of `head`).
    let base = strip_first_between(spa_html, "<title", "</title>");
    let with_head = match base.rfind("</head>") {
        Some(pos) => {
            let mut out = String::with_capacity(base.len() + head.len() + theme_tag.len());
            out.push_str(&base[..pos]);
            out.push_str(&attr);
            out.push_str(&head);
            out.push_str(&theme_tag);
            out.push_str(&base[pos..]);
            out
        }
        None => base,
    };
    // Card W36: a visible, clicked trail and a way onward in the SERVED HTML, not
    // only in structured data — the SPA search box cannot help a crawler or a
    // visitor who does not wait for JavaScript.
    // Card W36 (hardened): the trail is what keeps the city hub from being a dead end
    // for anyone who does not run JavaScript, so it must never be silently dropped.
    // When the shell has no marker (a homepage rewrite, a custom homepage HTML, an old
    // cached copy), the trail is inserted at the top of <body> instead of being lost —
    // a missing marker is a layout question, not a reason to publish a dead end.
    const MARKER: &str = "<!--ZH-NAV-->";
    let with_nav = match with_head.find(MARKER) {
        Some(pos) => format!(
            "{}{}{}",
            &with_head[..pos],
            nav_html,
            &with_head[pos + MARKER.len()..]
        ),
        None => match body_open_end(&with_head) {
            Some(pos) => {
                eprintln!(
                    "subfolder: homepage shell has no ZH-NAV marker — trail injected at <body>"
                );
                format!("{}{}{}", &with_head[..pos], nav_html, &with_head[pos..])
            }
            None => with_head,
        },
    };
    // The city's listings, in the HTML that is served. They go INSIDE `#app`, which the
    // SPA overwrites the moment it boots, so a visitor with JavaScript never sees them
    // and a crawler always does. If the shell's empty `#app` cannot be found the content
    // is appended before `</body>` — a missing anchor is a layout question, never a
    // reason to publish a hub page with no businesses on it.
    const APP_ANCHOR: &str = "<div id=\"app\"></div>";
    if let Some(pos) = with_nav.find(APP_ANCHOR) {
        return format!(
            "{}<div id=\"app\">{}</div>{}",
            &with_nav[..pos],
            app_html,
            &with_nav[pos + APP_ANCHOR.len()..]
        );
    }
    match with_nav.rfind("</body>") {
        Some(pos) => {
            eprintln!("subfolder: homepage shell has no empty #app — city listings appended at <body> end");
            format!("{}{}{}", &with_nav[..pos], app_html, &with_nav[pos..])
        }
        None => format!("{}{}", with_nav, app_html),
    }
}

const PAGE_CSS: &str = r#"
*{margin:0;padding:0;box-sizing:border-box}
body{font-family:var(--font);background:var(--bg);color:var(--text);line-height:1.6}
a{color:var(--link);text-decoration:none}
a:hover{text-decoration:underline}
header{background:var(--dark);color:#fff;padding:16px 24px}
header .inner{max-width:1120px;margin:0 auto;display:flex;justify-content:space-between;align-items:center;gap:16px}
header .logo{font-size:20px;font-weight:800;color:#fff}
header nav a{color:rgba(255,255,255,.8);font-size:14px;margin-left:18px}
/* ── Shared public page-shell: entry points + inline newsletter (card B89) ── */
.zh-topbar{background:#111827;color:#e5e7eb;font-size:13px}
.zh-topbar .inner{max-width:1120px;margin:0 auto;padding:8px 24px;display:flex;flex-wrap:wrap;gap:8px 16px;align-items:center;justify-content:space-between}
.zh-topbar .zh-loc{font-weight:600;opacity:.85}
.zh-topbar nav{display:flex;flex-wrap:wrap;gap:8px 18px}
.zh-topbar nav a{color:#e5e7eb;font-weight:600;text-decoration:none;white-space:nowrap}
.zh-topbar nav a:hover{color:#fff;text-decoration:underline}
.zh-newsletter{display:flex;flex-wrap:wrap;align-items:center;gap:10px;background:var(--primary-light,#eef2ff);border:1px solid var(--border,#e5e7eb);border-radius:12px;padding:12px 16px;margin:0 0 24px}
.zh-newsletter .zh-nl-label{font-weight:600;color:var(--dark,#111827)}
.zh-newsletter input[type=email]{flex:1 1 220px;min-width:180px;padding:9px 12px;border:1px solid #cbd5e1;border-radius:8px;font-size:14px}
.zh-newsletter button{background:var(--primary,#4f46e5);color:#fff;border:0;border-radius:8px;padding:10px 18px;font-weight:700;cursor:pointer}
.zh-newsletter .zh-nl-msg{font-size:13px;color:#374151;flex-basis:100%}
.zh-fnav{display:flex;flex-wrap:wrap;gap:10px;justify-content:center;margin-bottom:10px}
.zh-fnav a{color:var(--primary-light,#c7d2fe)}
.wrap{max-width:1120px;margin:0 auto;padding:32px 24px}
.crumbs{font-size:13px;color:var(--text-light);margin-bottom:18px}
.crumbs a{color:var(--link)}
h1{font-size:2rem;font-weight:800;color:var(--dark);margin-bottom:10px}
.lede{color:var(--text-light);max-width:760px;margin-bottom:24px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:16px}
.card{background:var(--card);border:1px solid var(--border);border-radius:var(--radius-lg);padding:16px;display:block}
.card h3{font-size:1.05rem;color:var(--dark);margin-bottom:6px}
.card .meta{font-size:.82rem;color:var(--text-light)}
.chips{margin:18px 0}
.chip{display:inline-block;background:var(--primary-light);color:var(--primary-hover);border:1px solid var(--border);border-radius:999px;padding:5px 12px;font-size:.82rem;margin:0 8px 8px 0}
.stars{color:#f59e0b;font-size:.9rem}
.pager{margin:28px 0;display:flex;gap:12px}
.explore{margin:36px 0 8px;border-top:1px solid var(--border);padding-top:18px}
.explore h2{font-size:1.2rem;margin-bottom:10px}
.explore-group{margin-bottom:14px}
.explore-group h3{font-size:.95rem;color:var(--text-light);font-weight:600;margin-bottom:6px}
.btn{background:var(--primary);color:#fff;border-radius:var(--radius);padding:10px 18px;font-weight:600}
.detail{background:var(--card);border:1px solid var(--border);border-radius:var(--radius-lg);padding:28px}
.detail h1{margin-bottom:6px}
.row{margin:8px 0;color:var(--text)}
.reviews{margin:28px 0 8px;border-top:1px solid var(--border);padding-top:18px}
.reviews h2{font-size:1.2rem;margin-bottom:12px;color:var(--dark)}
.review{background:var(--card);border:1px solid var(--border);border-radius:var(--radius-lg);padding:16px;margin-bottom:12px}
.review .who{font-size:.85rem;color:var(--text-light);margin-bottom:6px}
.review p{margin:0;color:var(--text)}
.review .resp{border-left:3px solid var(--border);padding-left:10px;margin-top:10px;font-size:.92rem;color:var(--text)}
.article-body h2{font-size:1.25rem;margin:22px 0 10px;color:var(--secondary)}
.article-body p{margin-bottom:14px}
.article-body ul,.article-body ol{margin:0 0 14px 22px}
footer{background:var(--dark);color:rgba(255,255,255,.7);padding:28px 24px;margin-top:48px;font-size:13px;text-align:center}
footer a{color:var(--primary-light)}
"#;

/// One shared entry-point bar for EVERY public page (card B89): the customer
/// sign-in/sign-up, the business owner and supplier entries, browse, and
/// change-city. Every href is a real destination; editing this one function
/// changes the chrome on every server-rendered public page at once.
pub(crate) fn public_topbar(site_name: &str, dir_label: Option<(&str, &str)>) -> String {
    let browse = match dir_label {
        Some((slug, _)) => format!("/{}/businesses", h(slug)),
        None => "/".to_string(),
    };
    format!(
        "<div class=\"zh-topbar\"><div class=\"inner\">\
<span class=\"zh-loc\">\u{1f4cd} {site}</span>\
<nav>\
<a href=\"{browse}\">\u{1f50d} Browse businesses</a>\
<a href=\"/visitor\">\u{1f464} Customer sign in / sign up</a>\
<a href=\"/portal\">\u{1f3ea} Business login / claim your listing</a>\
<a href=\"/distributor\">\u{1f4e6} Supplier login</a>\
<a href=\"/\">\u{1f5fa}\u{fe0f} Change city</a>\
</nav></div></div>",
        site = h(site_name),
        browse = browse,
    )
}

/// Inline newsletter signup — on EVERY public page, not just the footer
/// (card B89). Posts to the per-directory subscriber endpoint when the page
/// belongs to a city, and to the network/global endpoint otherwise.
pub(crate) fn newsletter_block(dir_label: Option<(&str, &str)>) -> String {
    let endpoint = match dir_label {
        Some((slug, _)) => format!("/api/v1/directories/{}/subscribers", h(slug)),
        None => "/api/v1/directories/newsletter".to_string(),
    };
    format!(
        "<form class=\"zh-newsletter\" data-endpoint=\"{endpoint}\">\
<label class=\"zh-nl-label\" for=\"zh-nl-email\">\u{1f4e7} Get local news &amp; deals by email</label>\
<input id=\"zh-nl-email\" type=\"email\" name=\"email\" required placeholder=\"you@example.com\" aria-label=\"Email address\">\
<button type=\"submit\">Subscribe</button>\
<span class=\"zh-nl-msg\" role=\"status\" aria-live=\"polite\"></span>\
</form>",
        endpoint = endpoint,
    )
}

/// Client-side handler for the inline newsletter form. One copy per page.
pub(crate) const NEWSLETTER_JS: &str = r#"<script>
(function(){
  function bind(f){
    f.addEventListener('submit', function(e){
      e.preventDefault();
      var input=f.querySelector('input[type=email]');
      var msg=f.querySelector('.zh-nl-msg');
      if(!input || !input.value){ return; }
      msg.textContent='Subscribing\u2026';
      fetch(f.getAttribute('data-endpoint'),{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({email:input.value})})
        .then(function(r){ if(!r.ok){ throw new Error('HTTP '+r.status); } return r.json(); })
        .then(function(){ msg.textContent='\u2705 Thanks \u2014 you are subscribed.'; f.reset(); })
        .catch(function(){ msg.textContent='\u26a0\ufe0f That did not work \u2014 please try again.'; });
    });
  }
  var forms=document.querySelectorAll('form.zh-newsletter');
  for(var i=0;i<forms.length;i++){ bind(forms[i]); }
})();
</script>"#;

fn shell_start(
    seo: &Seo,
    site_name: &str,
    dir_label: Option<(&str, &str)>,
    theme: &BrandTheme,
) -> String {
    let nav = match dir_label {
        Some((slug, name)) => format!(
            "<nav><a href=\"/{}\">{}</a><a href=\"/{}/businesses\">All businesses</a><a href=\"/\">Home</a></nav>",
            h(slug), h(name), h(slug)
        ),
        None => "<nav><a href=\"/\">Home</a><a href=\"/zaarhub\">All cities</a></nav>".to_string(),
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
{head}<style>{theme_root}
{css}</style>
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
</head>
<body>
{topbar}
<header><div class="inner"><a class="logo" href="/">{site}</a>{nav}</div></header>
<div class="wrap">
{newsletter}"#,
        head = head_html(seo, site_name),
        theme_root = theme.css_block(),
        css = PAGE_CSS,
        site = h(site_name),
        nav = nav,
        topbar = public_topbar(site_name, dir_label),
        newsletter = newsletter_block(dir_label),
    )
}

fn shell_end(site_name: &str, hq: Option<&str>) -> String {
    // The company address is ONE value for the whole company (zaarhub_site_config.headquarters,
    // entered once in Branding). It belongs in the footer so it reaches EVERY public page — the
    // city hub, its businesses/blog/deals pages, business listings and articles — instead of only
    // the city hero. Rendered server-side so a crawler sees it without JavaScript.
    let hq_line = match hq.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => format!(
            "<p class=\"zh-hq\" style=\"margin:0 0 6px\">\u{1f3e2} Headquarters: {}</p>",
            h(v)
        ),
        None => String::new(),
    };
    format!(
        "{js}</div><footer><div class=\"zh-fnav\">\
<a href=\"/\">Home</a>\
<a href=\"/zaarhub\">All cities</a>\
<a href=\"/visitor\">Customer sign in</a>\
<a href=\"/portal\">Business login</a>\
<a href=\"/distributor\">Supplier login</a>\
<a href=\"/legal/terms\">Terms</a>\
<a href=\"/legal/privacy\">Privacy</a></div>\
{hq_line}<p>\u{00a9} {year} {site} \u{2014} local business directory</p></footer></body></html>",
        js = NEWSLETTER_JS,
        hq_line = hq_line,
        year = chrono::Utc::now().format("%Y"),
        site = h(site_name)
    )
}

/// 301 to a canonical subfolder URL (SEO hygiene: a legacy/short URL is
/// redirected rather than served as a soft-404 SPA page).
fn redirect_301(location: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(header::LOCATION, location)
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn html_response(status: StatusCode, body: String) -> Response<Body> {
    let mut r = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    r
}

fn xml_response(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=1800")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// The brand name this directory's pages carry — the `<title>` suffix, the
/// footer, the breadcrumb root and `og:site_name`. Resolved from the shared
/// brand source (`crate::brand_theme`: network → directory → platform default),
/// so it can never be the request host and can never be hardcoded. A buyer
/// renaming their directory (or network) updates every page.
async fn brand_name_for(pool: &PgPool, dir: &DirectoryRec) -> String {
    crate::brand_theme::brand_name_for_directory(pool, &dir.name, dir.network_id).await
}

// ─────────────────────────────────────────────────────────────────────────────
// Business list (shared by directory home + businesses page)
// ─────────────────────────────────────────────────────────────────────────────

struct BizCard {
    name: String,
    slug: String,
    category: Option<String>,
    rating: f64,
    review_count: i32,
    address: Option<String>,
    /// The business's own short description. Optional everywhere: a business that never filled
    /// it in still gets a card. The city hub prints it (card CITY-HUB-TEXT, 2026-11-12) because
    /// a grid of names and star counts is thin content for a crawler and little help to a
    /// visitor deciding whom to call.
    description: Option<String>,
}

async fn list_businesses(
    pool: &PgPool,
    dir: &DirectoryRec,
    category: Option<&str>,
    limit: i64,
    offset: i64,
) -> Vec<BizCard> {
    /// Compile-time form of `COLS` for gate rule 5d: `concat!` needs a LITERAL, not a
    /// const, so the same bytes are published as a macro and the statements that use it are
    /// assembled entirely at compile time (a run-time build is the defect the rule names).
    /// MUST stay byte-identical to `COLS`.
    macro_rules! cols_lit {
        () => {
            "b.id, b.name, b.slug, b.description, b.logo_url, b.rating, \
                        b.review_count, b.address, b.city, b.state, dc.name AS category"
        };
    }

    let rows = if let Some(cat) = category {
        sqlx::query(concat!(
            "SELECT ",
            cols_lit!(),
            " FROM businesses b \
             LEFT JOIN directory_categories dc ON dc.id = b.category_id \
             WHERE b.directory_id = $1 AND b.is_active = true \
               AND lower(coalesce(dc.name,'')) = lower($2) \
             ORDER BY b.featured DESC NULLS LAST, b.rating DESC NULLS LAST, b.review_count DESC \
             LIMIT $3 OFFSET $4"
        ))
        .bind(dir.id)
        .bind(cat)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query(concat!(
            "SELECT ",
            cols_lit!(),
            " FROM businesses b \
             LEFT JOIN directory_categories dc ON dc.id = b.category_id \
             WHERE b.directory_id = $1 AND b.is_active = true \
             ORDER BY b.featured DESC NULLS LAST, b.rating DESC NULLS LAST, b.review_count DESC \
             LIMIT $2 OFFSET $3"
        ))
        .bind(dir.id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    };

    let mut out = Vec::new();
    for r in rows.unwrap_or_default() {
        let name: String = r.try_get("name").unwrap_or_default();
        let category: Option<String> = r.try_get("category").unwrap_or(None);
        out.push(BizCard {
            name,
            slug: r.try_get("slug").unwrap_or_default(),
            category,
            rating: r
                .try_get::<Option<f64>, _>("rating")
                .unwrap_or(None)
                .unwrap_or(0.0),
            review_count: r
                .try_get::<Option<i32>, _>("review_count")
                .unwrap_or(None)
                .unwrap_or(0),
            address: r.try_get("address").unwrap_or(None),
            description: r.try_get("description").unwrap_or(None),
        });
    }
    out
}

fn card_html(base: &str, dir_slug: &str, b: &BizCard) -> String {
    let cat = b
        .category
        .as_ref()
        .map(|c| format!("<span class=\"meta\">{}</span> · ", h(c)))
        .unwrap_or_default();
    let addr = b
        .address
        .as_ref()
        .map(|a| format!("<div class=\"meta\">{}</div>", h(a)))
        .unwrap_or_default();
    format!(
        "<a class=\"card\" href=\"{base}/{dir}/businesses/{slug}\"><h3>{name}</h3><div class=\"meta\">{cat}<span class=\"stars\">★ {rating:.1}</span> ({reviews})</div>{addr}</a>",
        base = h(base),
        dir = h(dir_slug),
        slug = h(&b.slug),
        name = h(&b.name),
        cat = cat,
        rating = b.rating,
        reviews = b.review_count,
        addr = addr,
    )
}

fn breadcrumbs(items: &[(&str, &str)]) -> String {
    let mut parts = Vec::new();
    for (i, (label, url)) in items.iter().enumerate() {
        if i + 1 == items.len() {
            parts.push(format!("<span>{}</span>", h(label)));
        } else {
            parts.push(format!("<a href=\"{}\">{}</a>", h(url), h(label)));
        }
    }
    format!("<div class=\"crumbs\">{}</div>", parts.join(" › "))
}

fn breadcrumb_ld(items: &[(&str, &str)]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = items
        .iter()
        .enumerate()
        .map(|(i, (name, url))| {
            serde_json::json!({
                "@type": "ListItem",
                "position": i + 1,
                "name": name,
                "item": url,
            })
        })
        .collect();
    serde_json::json!({
        "@context": "https://schema.org",
        "@type": "BreadcrumbList",
        "itemListElement": list,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Satellite towns — GET /<slug>/<town-slug>   (W26)
// ─────────────────────────────────────────────────────────────────────────────
//
// W22 scoped a city page to its own local area (its own town + neighbours inside
// `LOCAL_AREA_KM`) but published ONE URL for the whole area, and the town chips on
// that page are JS filters (`href="javascript:void(0)"` + onclick). So "plumber in
// Melbourne FL" had no page, and no page even named Melbourne: every town's listings
// competed on one city page. Each town now gets its own server-rendered,
// self-canonical URL with its own title/meta, its own businesses as real
// `<a href>` links, a breadcrumb trail UP to the city and links across to its
// sibling towns — and a town with fewer than `MIN_TOWN_LISTINGS` listings
// publishes nothing at all (no thin pages, and the sitemap never advertises one).

/// A town page needs at least this many active listings; below it the page is thin
/// content that competes with the city page instead of ranking for the town.
const MIN_TOWN_LISTINGS: i64 = 3;

/// One town inside a directory's local area. `slug` is the crawlable path segment.
pub(crate) struct TownEntry {
    /// `satellite_towns.town_key` — the lower-cased town name taken from the listings' city
    /// field, so the admin screen and this gate always talk about the same town.
    pub key: String,
    pub label: String,
    pub slug: String,
    pub count: i64,
    pub is_home: bool,
    /// Whether the town may publish anything right now: the directory includes satellite towns,
    /// an admin approved THIS town, and it still has enough listings to be a real page (W30).
    pub published: bool,
}

/// Per-directory satellite gate (W30): the master switch plus the set of approved town keys.
///
/// Fails OPEN: a database without the `satellite_towns` table (an install mid-upgrade) keeps the
/// old behaviour — towns publish on listing count alone — rather than silently unpublishing every
/// town page of every city.
pub(crate) async fn satellite_approval(
    pool: &PgPool,
    dir_id: Uuid,
) -> (bool, std::collections::HashMap<String, bool>) {
    let include: bool = sqlx::query_scalar::<_, bool>(
        "SELECT include_satellite_towns FROM directories WHERE id = $1",
    )
    .bind(dir_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(true);

    let rows: Result<Vec<(String, bool)>, sqlx::Error> =
        sqlx::query_as("SELECT town_key, approved FROM satellite_towns WHERE directory_id = $1")
            .bind(dir_id)
            .fetch_all(pool)
            .await;

    match rows {
        Ok(rows) => (include, rows.into_iter().collect()),
        Err(_) => (true, std::collections::HashMap::new()),
    }
}

/// Every town a directory holds within its local area, home town first. Uses the
/// SAME rule as the city page (`zaarhub::LOCAL_AREA_KM`): grouped in SQL so counts
/// are exact, never taken from a capped card list. A town whose row is missing from
/// `satellite_towns` is a DRAFT — it publishes nothing until an admin approves it.
pub(crate) async fn local_town_entries(
    pool: &PgPool,
    dir_id: Uuid,
    dir_city: &str,
) -> Vec<TownEntry> {
    let (include, approved) = satellite_approval(pool, dir_id).await;
    // The master switch off means the city has no satellite towns at all: no pages, no links,
    // nothing in the sitemap. The approvals are kept, so ticking it again restores them.
    if !include {
        return Vec::new();
    }

    let rows: Vec<(String, String, i64, Option<f64>, Option<f64>)> = sqlx::query_as(
        r#"SELECT town_key,
                  (ARRAY_AGG(town_label ORDER BY n DESC, town_label))[1] AS town_label,
                  SUM(n)::bigint AS n,
                  SUM(n * lat) / NULLIF(SUM(n), 0),
                  SUM(n * lon) / NULLIF(SUM(n), 0)
           FROM (
               SELECT TRIM(REGEXP_REPLACE(LOWER(SPLIT_PART(TRIM(b.city), ',', 1)), '[^[:alnum:]]+', ' ', 'g')) AS town_key,
                      TRIM(SPLIT_PART(TRIM(b.city), ',', 1)) AS town_label,
                      COUNT(*)::bigint AS n,
                      AVG(b.latitude) AS lat,
                      AVG(b.longitude) AS lon
               FROM businesses b
               WHERE b.directory_id = $1 AND b.is_active = true
                 AND b.city IS NOT NULL AND TRIM(b.city) <> ''
               GROUP BY 1, 2
           ) s
           GROUP BY town_key"#,
    )
    .bind(dir_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let own_key = crate::handlers::zaarhub::town_key(dir_city);
    let total: i64 = rows.iter().map(|(_, _, c, _, _)| *c).sum();
    // A NEIGHBOURHOOD directory is addressed by its POSTAL city, not by its own name: every
    // Lake Nona listing says "Orlando, FL 32827", so no row ever matches the directory's own
    // city and the whole local area used to collapse — no towns, no chips, no sitemap entries,
    // and 0 counts in the panel while 15 Kissimmee listings sat in the directory. When no row
    // carries the directory's own name, the postal city holding most of its listings IS the home
    // town (and gets no separate town page, so the two cannot compete for one query).
    let home_key = if rows.iter().any(|(k, _, _, _, _)| *k == own_key) {
        own_key
    } else {
        rows.iter()
            .find(|(_, _, count, _, _)| {
                total > 0 && count.saturating_mul(100) >= total.saturating_mul(60)
            })
            .map(|(k, _, _, _, _)| k.clone())
            .unwrap_or(own_key)
    };
    // The local centre: the home town's own position, else the centre of every listing the
    // directory holds (so a neighbourhood still knows where it is).
    let centre: Option<(f64, f64)> = rows
        .iter()
        .find(|(k, _, _, _, _)| *k == home_key)
        .and_then(|(_, _, _, la, lo)| la.zip(*lo))
        .or_else(|| {
            let (mut weight, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64);
            for (_, _, count, la, lo) in rows.iter() {
                if let (Some(a), Some(o)) = (la, lo) {
                    let w = *count as f64;
                    weight += w;
                    sx += a * w;
                    sy += o * w;
                }
            }
            (weight > 0.0).then(|| (sx / weight, sy / weight))
        });

    let mut out: Vec<TownEntry> = Vec::new();
    for (key, label, count, la, lo) in rows {
        if key == home_key {
            out.push(TownEntry {
                key,
                slug: crate::handlers::zaarhub::town_slug(&label),
                label,
                count,
                is_home: true,
                published: true,
            });
            continue;
        }
        // A town with no usable centre distance is not part of the local area.
        let (cla, clo) = match centre {
            Some(c) => c,
            None => continue,
        };
        let (tla, tlo) = match la.zip(lo) {
            Some(c) => c,
            None => continue,
        };
        if crate::handlers::zaarhub::haversine_km(cla, clo, tla, tlo)
            > crate::handlers::zaarhub::LOCAL_AREA_KM
        {
            continue;
        }
        out.push(TownEntry {
            slug: crate::handlers::zaarhub::town_slug(&label),
            label,
            count,
            is_home: false,
            published: approved.get(&key).copied().unwrap_or(false) && count >= MIN_TOWN_LISTINGS,
            key,
        });
    }
    out.sort_by(|a, b| b.is_home.cmp(&a.is_home).then(b.count.cmp(&a.count)));
    out
}

/// Real `<a href>` chips for every town with a published page — the crawl and click
/// path from a city surface down to each town. `skip` omits the page you are on.
async fn town_chips_html(
    pool: &PgPool,
    dir: &DirectoryRec,
    base: &str,
    skip: Option<&str>,
) -> String {
    let towns = local_town_entries(pool, dir.id, &dir.city).await;
    let mut out = String::new();
    for t in towns
        .iter()
        .filter(|t| t.published && Some(t.slug.as_str()) != skip)
    {
        let href = if t.is_home {
            format!("{}/{}", base, dir.slug)
        } else {
            format!("{}/{}/{}", base, dir.slug, t.slug)
        };
        out.push_str(&format!(
            "<a class=\"chip\" href=\"{}\">{} ({})</a>",
            h(&href),
            h(&t.label),
            t.count
        ));
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// "Keep exploring" — no public page is a dead end (card W36)
// ─────────────────────────────────────────────────────────────────────────────
//
// David's rule: every page offers a way onward, drawn from REAL relationships —
// never a random list. This one builder feeds every server-rendered page, so the
// hop count from any page to the rest of the directory is the same everywhere:
// the city's categories, its best-rated businesses, its published towns and its
// newest writing — plus whatever the caller knows is most related (the next
// article, the sibling businesses on a listing page).

/// One group of onward links inside the "Keep exploring" block.
struct ExploreGroup {
    heading: String,
    links: Vec<(String, String)>,
}

/// Push a link unless it is the page being rendered or already present.
fn push_link(out: &mut Vec<(String, String)>, label: &str, href: &str, skip: &str) {
    let href = href.trim_end_matches('/');
    if href.is_empty() || href == skip.trim_end_matches('/') {
        return;
    }
    if out.iter().any(|(_, h)| h.trim_end_matches('/') == href) {
        return;
    }
    out.push((label.to_string(), href.to_string()));
}

/// The relationships every page in this city can offer: other categories,
/// the best-rated businesses, the published towns, the newest articles and
/// the city's own hub pages. `skip` is the path of the page being rendered.
async fn city_relationships(
    pool: &PgPool,
    dir: &DirectoryRec,
    base: &str,
    skip: &str,
    biz_limit: i64,
) -> Vec<ExploreGroup> {
    let mut groups: Vec<ExploreGroup> = Vec::new();

    // 1. Other categories in this city — the browse level above every listing.
    let mut links: Vec<(String, String)> = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT dc.slug, dc.name, COUNT(b.id) AS n \
         FROM directory_categories dc \
         JOIN businesses b ON b.category_id = dc.id AND b.is_active = true \
         WHERE b.directory_id = $1 AND coalesce(dc.slug,'') <> '' \
         GROUP BY dc.slug, dc.name \
         ORDER BY COUNT(b.id) DESC, dc.name LIMIT 10",
    )
    .bind(dir.id)
    .fetch_all(pool)
    .await
    {
        for r in &rows {
            let slug: String = r.try_get("slug").unwrap_or_default();
            let name: String = r.try_get("name").unwrap_or_default();
            let n: i64 = r.try_get("n").unwrap_or(0);
            if slug.is_empty() || name.is_empty() {
                continue;
            }
            push_link(
                &mut links,
                &format!("{} ({})", name, n),
                &format!("{}/{}/{}", base, dir.slug, slug),
                skip,
            );
        }
    }
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: format!("More in {}", dir.city),
            links,
        });
    }

    // 2. The city's best-rated businesses — a real listing next to this page.
    let mut links: Vec<(String, String)> = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT b.name, b.slug FROM businesses b \
         WHERE b.directory_id = $1 AND b.is_active = true AND coalesce(b.slug,'') <> '' \
         ORDER BY b.featured DESC NULLS LAST, b.rating DESC NULLS LAST, \
                  b.review_count DESC NULLS LAST LIMIT $2",
    )
    .bind(dir.id)
    .bind(biz_limit)
    .fetch_all(pool)
    .await
    {
        for r in &rows {
            let name: String = r.try_get("name").unwrap_or_default();
            let slug: String = r.try_get("slug").unwrap_or_default();
            if name.is_empty() || slug.is_empty() {
                continue;
            }
            push_link(
                &mut links,
                &name,
                &format!("{}/{}/businesses/{}", base, dir.slug, slug),
                skip,
            );
        }
    }
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: format!("Popular in {}", dir.city),
            links,
        });
    }

    // 3. The satellite towns that actually publish a page (>= MIN_TOWN_LISTINGS).
    let mut links: Vec<(String, String)> = Vec::new();
    for t in local_town_entries(pool, dir.id, &dir.city)
        .await
        .iter()
        .filter(|t| t.published)
    {
        let href = if t.is_home {
            format!("{}/{}", base, dir.slug)
        } else {
            format!("{}/{}/{}", base, dir.slug, t.slug)
        };
        push_link(
            &mut links,
            &format!("{} ({})", t.label, t.count),
            &href,
            skip,
        );
    }
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: "Nearby towns".to_string(),
            links,
        });
    }

    // 4. The newest writing in this city (posts and sponsored articles alike).
    let mut links: Vec<(String, String)> = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT slug, title, source_type FROM ( \
           SELECT bp.slug, bp.title, 'blog'::text AS source_type, \
                  COALESCE(bp.scheduled_at, bp.created_at) AS at \
           FROM blog_posts bp \
           WHERE bp.directory_id = $1 AND bp.published = true AND bp.status = 'published' \
             AND (bp.scheduled_at IS NULL OR bp.scheduled_at <= NOW()) \
           UNION ALL \
           SELECT ba.slug, ba.title, 'article'::text AS source_type, ba.created_at AS at \
           FROM business_articles ba \
           WHERE ba.directory_id = $1 AND ba.status = 'published' \
         ) u ORDER BY at DESC LIMIT 4",
    )
    .bind(dir.id)
    .fetch_all(pool)
    .await
    {
        for r in &rows {
            let slug: String = r.try_get("slug").unwrap_or_default();
            let title: String = r.try_get("title").unwrap_or_default();
            let kind: String = r.try_get("source_type").unwrap_or_default();
            if slug.is_empty() || title.is_empty() {
                continue;
            }
            let seg = if kind == "article" {
                "articles"
            } else {
                "blog"
            };
            push_link(
                &mut links,
                &title,
                &format!("{}/{}/{}/{}", base, dir.slug, seg, slug),
                skip,
            );
        }
    }
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: "Read next".to_string(),
            links,
        });
    }

    // 5. The city's own browse surfaces — always real, always 200.
    let mut links: Vec<(String, String)> = Vec::new();
    push_link(
        &mut links,
        "All businesses",
        &format!("{}/{}/businesses", base, dir.slug),
        skip,
    );
    push_link(
        &mut links,
        &format!("{} home", dir.city),
        &format!("{}/{}", base, dir.slug),
        skip,
    );
    push_link(
        &mut links,
        "Blog",
        &format!("{}/{}/blog", base, dir.slug),
        skip,
    );
    push_link(
        &mut links,
        "Deals",
        &format!("{}/{}/deals", base, dir.slug),
        skip,
    );
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: format!("Around {}", dir.city),
            links,
        });
    }

    groups
}

/// Render the "Keep exploring" block. `extra` is the caller's own strongest
/// relationship set (same-category peers, next/previous article). Returns an
/// empty string only if there is genuinely nothing to offer.
async fn explore_block(
    pool: &PgPool,
    dir: &DirectoryRec,
    base: &str,
    skip: &str,
    extra: Vec<(String, String)>,
) -> String {
    let mut groups: Vec<ExploreGroup> = Vec::new();
    let mut own: Vec<(String, String)> = Vec::new();
    for (label, href) in extra {
        push_link(&mut own, &label, &href, skip);
    }
    if !own.is_empty() {
        groups.push(ExploreGroup {
            heading: "Related".to_string(),
            links: own,
        });
    }
    groups.extend(city_relationships(pool, dir, base, skip, 5).await);

    let mut body = String::new();
    for g in &groups {
        let chips: Vec<String> = g
            .links
            .iter()
            .take(12)
            .map(|(label, href)| format!("<a class=\"chip\" href=\"{}\">{}</a>", h(href), h(label)))
            .collect();
        if chips.is_empty() {
            continue;
        }
        body.push_str(&format!(
            "<div class=\"explore-group\"><h3>{}</h3><div class=\"chips\">{}</div></div>",
            h(&g.heading),
            chips.join("")
        ));
    }
    if body.is_empty() {
        return String::new();
    }
    format!(
        "<section class=\"explore\" aria-label=\"Keep exploring\"><h2>Keep exploring</h2>{}</section>",
        body
    )
}

/// The piece published immediately before and after this one in the city — the
/// "next / previous article" the card asks for, across posts AND sponsored
/// articles, so an archive can be read end to end.
async fn neighbour_links(
    pool: &PgPool,
    dir: &DirectoryRec,
    base: &str,
    at: Option<chrono::DateTime<chrono::Utc>>,
    skip: &str,
) -> Vec<(String, String)> {
    let Some(at) = at else {
        return Vec::new();
    };
    // Parameterized queries only (pre-build gate): the direction is part of the
    // statement, never spliced in — two complete statements, one per direction.
    const PREV: &str = "WITH u AS ( \
           SELECT slug, 'blog'::text AS kind, title, COALESCE(scheduled_at, created_at) AS at \
           FROM blog_posts WHERE directory_id = $1 AND published = true AND status = 'published' \
             AND (scheduled_at IS NULL OR scheduled_at <= NOW()) \
           UNION ALL \
           SELECT slug, 'article'::text AS kind, title, created_at AS at \
           FROM business_articles WHERE directory_id = $1 AND status = 'published' \
         ) \
         SELECT slug, kind, title FROM u WHERE at IS NOT NULL AND at < $2 ORDER BY at DESC LIMIT 1";
    const NEXT: &str = "WITH u AS ( \
           SELECT slug, 'blog'::text AS kind, title, COALESCE(scheduled_at, created_at) AS at \
           FROM blog_posts WHERE directory_id = $1 AND published = true AND status = 'published' \
             AND (scheduled_at IS NULL OR scheduled_at <= NOW()) \
           UNION ALL \
           SELECT slug, 'article'::text AS kind, title, created_at AS at \
           FROM business_articles WHERE directory_id = $1 AND status = 'published' \
         ) \
         SELECT slug, kind, title FROM u WHERE at IS NOT NULL AND at > $2 ORDER BY at ASC LIMIT 1";
    let mut out: Vec<(String, String)> = Vec::new();
    for (sql, prefix) in [(PREV, "← Previous: "), (NEXT, "Next: ")] {
        let row = sqlx::query(sql)
            .bind(dir.id)
            .bind(at)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
        if let Some(r) = row {
            let slug: String = r.try_get("slug").unwrap_or_default();
            let kind: String = r.try_get("kind").unwrap_or_default();
            let title: String = r.try_get("title").unwrap_or_default();
            if slug.is_empty() {
                continue;
            }
            let seg = if kind == "article" {
                "articles"
            } else {
                "blog"
            };
            push_link(
                &mut out,
                &format!("{}{}", prefix, title),
                &format!("{}/{}/{}/{}", base, dir.slug, seg, slug),
                skip,
            );
        }
    }
    out
}

/// Inline CSS for the server-injected navigation strip. The SPA document carries
/// its own stylesheet, which knows nothing about `.crumbs` / `.explore`, so the
/// block ships its own rules and looks identical on every page type.
const NAV_CSS: &str = "<style>\
.zh-navbar{max-width:1180px;margin:0 auto;padding:12px 18px 0;font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif}\
.zh-navbar .crumbs{font-size:13px;color:#6b7280;margin-bottom:8px}\
.zh-navbar .crumbs a{color:#ea580c;text-decoration:none}\
.zh-navbar .crumbs a:hover{text-decoration:underline}\
.zh-navbar .explore{margin:4px 0 16px;padding-top:10px;border-top:1px solid #f1f5f9}\
.zh-navbar .explore h2{font-size:15px;color:#111827;margin:0 0 8px}\
.zh-navbar .explore-group{margin-bottom:10px}\
.zh-navbar .explore-group h3{font-size:12px;font-weight:600;color:#6b7280;margin:0 0 5px;text-transform:uppercase;letter-spacing:.03em}\
.zh-navbar .chip{display:inline-block;background:#fff7ed;color:#c2410c;border:1px solid #fed7aa;border-radius:999px;padding:5px 12px;font-size:12.5px;margin:0 6px 6px 0;text-decoration:none}\
.zh-navbar .chip:hover{background:#ffedd5}\
.zh-navbar .zh-quicknav{margin:0 0 10px}\
.zh-navbar .zh-quicknav h2{font-size:12px;font-weight:600;color:#6b7280;text-transform:uppercase;letter-spacing:.03em;margin:0 0 5px}\
.zh-navbar .zh-quicknav .chips{display:flex;gap:6px;overflow-x:auto;white-space:nowrap;padding-bottom:2px;scrollbar-width:none}\
.zh-navbar .zh-quicknav .chips::-webkit-scrollbar{display:none}\
.zh-navbar .zh-quicknav .chip{margin:0;flex:0 0 auto}\
</style>";

/// A single compact band of onward links: one scrollable row, capped so the strip
/// never pushes the page's own hero down. Used on the two SPA surfaces (network
/// home and city page), where the page below renders its own layout.
fn compact_explore(links: Vec<(String, String)>) -> String {
    let mut chips = String::new();
    let mut seen = 0usize;
    for (label, href) in links {
        if seen >= 8 {
            break;
        }
        let tag = format!("href=\"{}\"", h(&href));
        if chips.contains(&tag) {
            continue;
        }
        chips.push_str(&format!(
            "<a class=\"chip\" href=\"{}\">{}</a>",
            h(&href),
            h(&label)
        ));
        seen += 1;
    }
    if chips.is_empty() {
        return String::new();
    }
    format!(
        "<section class=\"explore zh-quicknav\" aria-label=\"Keep exploring\"><h2>Keep exploring</h2><div class=\"chips\">{}</div></section>",
        chips
    )
}

/// The served navigation strip for one surface: a small inline stylesheet, the
/// breadcrumb trail, then the "Keep exploring" block.
fn nav_strip(crumbs: &str, explore: &str) -> String {
    format!(
        "{}<div class=\"zh-navbar\">{}{}</div>",
        NAV_CSS, crumbs, explore
    )
}

/// Navigation for the NETWORK homepage (`/`): the root has no level above it, so
/// the trail is the "you are here" marker, and the onward block is every city the
/// network publishes plus the two ways in for a business owner.
pub async fn home_nav_html(pool: &PgPool) -> String {
    let mut groups: Vec<ExploreGroup> = Vec::new();

    let mut links: Vec<(String, String)> = Vec::new();
    if let Ok(rows) = sqlx::query(
        "SELECT slug, city FROM directories WHERE status = 'active' \
         ORDER BY city ASC LIMIT 24",
    )
    .fetch_all(pool)
    .await
    {
        for r in &rows {
            let slug: String = r.try_get("slug").unwrap_or_default();
            let city: String = r.try_get("city").unwrap_or_default();
            if slug.is_empty() || city.is_empty() {
                continue;
            }
            push_link(
                &mut links,
                &format!("{} directory", city),
                &format!("/{}", slug),
                "/",
            );
        }
    }
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: "Cities we cover".to_string(),
            links,
        });
    }

    let mut links: Vec<(String, String)> = Vec::new();
    push_link(&mut links, "All cities", "/zaarhub", "/");
    push_link(&mut links, "Customer sign in", "/visitor", "/");
    push_link(&mut links, "Business login", "/portal", "/");
    push_link(
        &mut links,
        "Add your business",
        "/submit-business.html",
        "/",
    );
    push_link(&mut links, "Claim your listing", "/claim", "/");
    if !links.is_empty() {
        groups.push(ExploreGroup {
            heading: "Get started".to_string(),
            links,
        });
    }

    // One compact scrollable row: cities first, then the owner entry points.
    let mut flat: Vec<(String, String)> = Vec::new();
    for g in &groups {
        flat.extend(g.links.iter().cloned());
    }
    let explore = compact_explore(flat);
    nav_strip(
        "<div class=\"crumbs\" aria-label=\"Breadcrumb\"><span>ZaarHub home</span></div>",
        &explore,
    )
}

/// Navigation for a CITY page (`/<slug>`) in the SPA document: Home › City, then
/// the city's real relationships. This is what makes the city hub a hub instead
/// of a dead end for anyone who does not run JavaScript.
async fn city_nav_html(pool: &PgPool, dir: &DirectoryRec, base: &str, canonical: &str) -> String {
    let crumbs = breadcrumbs(&[("ZaarHub", &format!("{}/", base)), (&dir.name, canonical)]);
    // One compact row (categories first) so the city's own hero stays in view.
    let mut flat: Vec<(String, String)> = Vec::new();
    for g in city_relationships(pool, dir, base, canonical, 3).await {
        flat.extend(g.links);
    }
    nav_strip(&crumbs, &compact_explore(flat))
}

/// The active businesses of ONE town, in the same order the city grid uses.
async fn list_town_businesses(
    pool: &PgPool,
    dir_id: Uuid,
    town_key: &str,
    limit: i64,
) -> Vec<BizCard> {
    /// Compile-time form of the shared column list (gate rule 5d). MUST stay
    /// byte-identical to `COLS` / `list_businesses`'s `cols_lit!`.
    macro_rules! cols_lit {
        () => {
            "b.id, b.name, b.slug, b.description, b.logo_url, b.rating, \
                        b.review_count, b.address, b.city, b.state, dc.name AS category"
        };
    }

    let rows = sqlx::query(concat!(
        "SELECT ",
        cols_lit!(),
        " FROM businesses b \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE b.directory_id = $1 AND b.is_active = true \
           AND TRIM(REGEXP_REPLACE(LOWER(SPLIT_PART(TRIM(b.city), ',', 1)), '[^[:alnum:]]+', ' ', 'g')) = $2 \
         ORDER BY b.featured DESC NULLS LAST, b.rating DESC NULLS LAST, b.review_count DESC \
         LIMIT $3"
    ))
    .bind(dir_id)
    .bind(town_key)
    .bind(limit)
    .fetch_all(pool)
    .await;

    let mut out = Vec::new();
    for r in rows.unwrap_or_default() {
        out.push(BizCard {
            name: r.try_get("name").unwrap_or_default(),
            slug: r.try_get("slug").unwrap_or_default(),
            category: r.try_get("category").unwrap_or(None),
            rating: r
                .try_get::<Option<f64>, _>("rating")
                .unwrap_or(None)
                .unwrap_or(0.0),
            review_count: r
                .try_get::<Option<i32>, _>("review_count")
                .unwrap_or(None)
                .unwrap_or(0),
            address: r.try_get("address").unwrap_or(None),
            description: r.try_get("description").unwrap_or(None),
        });
    }
    out
}

/// GET `/<city>/<town-slug>` — one crawlable page per satellite town.
///
/// Returns `None` when the path is not a town of this directory (a real 404 is then
/// produced by the caller), so a town can never shadow an unknown URL.
pub async fn town_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    town_slug_req: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    let towns = local_town_entries(pool, dir.id, &dir.city).await;
    let wanted = town_slug_req.trim().to_lowercase();
    let town = towns
        .iter()
        .find(|t| !t.is_home && t.slug == wanted && t.published)?;

    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;
    let canonical = format!("{}/{}/{}", base, dir.slug, town.slug);
    let label = format!("{} Businesses", town.label);

    // Admin-editable per-town pattern when one exists; the derived title otherwise.
    let count_s = town.count.to_string();
    let (fb_title, fb_desc) = seo_pattern(
        pool,
        dir.id,
        "town",
        &[
            ("town", town.label.as_str()),
            ("city", dir.city.as_str()),
            ("count", count_s.as_str()),
        ],
    )
    .await;
    let title = fb_title.unwrap_or_else(|| {
        format!(
            "{} Businesses & Local Services, {} | {}",
            town.label, dir.state, site
        )
    });
    let description = clip(
        &fb_desc.unwrap_or_else(|| {
            format!(
                "Browse {} local businesses in {}, {}. Addresses, phone numbers, ratings and reviews — part of the {} directory on {}.",
                town.count, town.label, dir.state, dir.city, site
            )
        }),
        158,
    );

    // The page's own copy says "N businesses", so it must render all N: a cap of 100 on a town
    // with 148 listings (Melbourne on Palm Bay) advertised 148 and showed 100.
    let items = list_town_businesses(pool, dir.id, &town.key, 1200).await;
    let cards: String = items
        .iter()
        .map(|b| card_html(&base, &dir.slug, b))
        .collect();

    // W30(b): the articles filed to THIS town are reachable from its page — the town
    // is a real browse surface, not just a list of shops.
    let town_articles: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, title FROM blog_posts \
         WHERE directory_id = $1 AND town_key = $2 \
           AND coalesce(published, true) = true AND coalesce(status, 'published') = 'published' \
           AND (scheduled_at IS NULL OR scheduled_at <= NOW()) \
         ORDER BY coalesce(scheduled_at, created_at) DESC LIMIT 10",
    )
    .bind(dir.id)
    .bind(&town.key)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let articles_block = if town_articles.is_empty() {
        String::new()
    } else {
        let lis: String = town_articles
            .iter()
            .map(|(pslug, ptitle)| {
                format!(
                    "<li><a href=\"/{}/blog/{}\">{}</a></li>",
                    h(&dir.slug),
                    h(pslug),
                    h(ptitle)
                )
            })
            .collect();
        format!(
            "<h2>Reading about {}</h2>\n<ul class=\"article-list\">{}</ul>",
            h(&town.label),
            lis
        )
    };

    let mut jsonld = vec![serde_json::json!({
        "@context": "https://schema.org",
        "@type": "CollectionPage",
        "name": format!("{} businesses in {}", town.label, dir.city),
        "url": canonical.clone(),
        "mainEntity": {
            "@type": "ItemList",
            "name": label.clone(),
            "numberOfItems": town.count,
            "itemListElement": items.iter().enumerate().map(|(i, b)| serde_json::json!({
                "@type": "ListItem",
                "position": i + 1,
                "url": format!("{}/{}/businesses/{}", base, dir.slug, b.slug),
                "name": b.name,
            })).collect::<Vec<_>>(),
        },
    })];
    jsonld.push(breadcrumb_ld(&[
        (&site, &format!("{}/", base)),
        (&dir.name, &format!("{}/{}/", base, dir.slug)),
        (&town.label, &canonical),
    ]));

    let seo = Seo {
        title,
        description,
        canonical: canonical.clone(),
        og_type: "website".into(),
        og_image: None,
        jsonld,
    };

    let siblings = town_chips_html(pool, &dir, &base, Some(town.slug.as_str())).await;
    let siblings_block = if siblings.is_empty() {
        String::new()
    } else {
        format!(
            "<h2>Other towns in {}</h2>\n<div class=\"chips\">{}</div>",
            h(&dir.city),
            siblings
        )
    };

    let explore = explore_block(pool, &dir, &base, &canonical, Vec::new()).await;

    let body = format!(
        r#"{start}{crumbs}
<h1>Local Businesses in {town}, {state}</h1>
<p class="lede">{n} local businesses listed in {town}, {state} — rated and reviewed by locals. Part of the <a href="/{dir}">{city}</a> directory.</p>
<div class="grid">{cards}</div>
{articles}
{siblings}
<p style="margin-top:24px"><a class="btn" href="/{dir}/businesses">See all {city} businesses →</a></p>
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            (&town.label, &canonical),
        ]),
        town = h(&town.label),
        state = h(&dir.state),
        n = town.count,
        dir = h(&dir.slug),
        city = h(&dir.city),
        cards = cards,
        articles = articles_block,
        siblings = siblings_block,
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );
    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Directory home — GET /<slug>
// ─────────────────────────────────────────────────────────────────────────────

pub async fn directory_home(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    spa_html: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let canonical = format!("{}/{}", base, dir.slug);

    // City-page SEO row (the platform's existing city meta) is the derived default.
    let city_row = sqlx::query(
        "SELECT meta_title, meta_description, hero_image_url, intro_copy FROM city_pages \
         WHERE city_slug = $1 AND is_active = true LIMIT 1",
    )
    .bind(&dir.slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let (city_title, city_desc, hero, city_intro) = match city_row {
        Some(r) => (
            r.try_get::<Option<String>, _>("meta_title").unwrap_or(None),
            r.try_get::<Option<String>, _>("meta_description")
                .unwrap_or(None),
            r.try_get::<Option<String>, _>("hero_image_url")
                .unwrap_or(None),
            r.try_get::<Option<String>, _>("intro_copy").unwrap_or(None),
        ),
        None => (None, None, None, None),
    };

    let ov = seo_override(pool, "city", dir.id).await;
    // B119: the directory's SEO pattern for city pages (e.g. `{city} Business
    // Directory | ZaarHub`) is rendered here, so one pattern serves every city.
    let (fb_title, fb_desc) = seo_pattern(pool, dir.id, "city", &[]).await;

    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .or(city_title)
        .or(fb_title)
        .unwrap_or_else(|| format!("Local Businesses in {}, {} | {}", dir.city, dir.state, site));

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(city_desc)
        .or_else(|| dir.description.clone())
        .or(fb_desc)
        .unwrap_or_else(|| {
            format!(
                "Discover top-rated local businesses in {}, {}. Browse reviews, deals and services across every category.",
                dir.city, dir.state
            )
        });
    let description = clip(&strip_tags(&description), 158);

    let og_image = ov.as_ref().and_then(|o| o.og_image.clone()).or(hero);

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses WHERE directory_id = $1 AND is_active = true",
    )
    .bind(dir.id)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let total = if total > 0 {
        total
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM business_listings bl JOIN city_pages cp ON cp.id = bl.city_page_id \
             WHERE cp.city_slug = $1",
        )
        .bind(&dir.slug)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
    };

    // Top businesses for the ItemList JSON-LD below. (The visible listing is
    // rendered by the shared homepage template from the city API.) 18 rather than 12 keeps the
    // served page comfortably above the 3,000-character visible-text floor even in the city
    // with the fewest descriptions (measured 2026-11-12: 12 cards left St. Cloud at 3,123).
    let top = list_businesses(pool, &dir, None, 18, 0).await;

    // JSON-LD: CollectionPage + ItemList + BreadcrumbList (or admin custom_schema).
    let mut jsonld = Vec::new();
    if let Some(cs) = ov.as_ref().and_then(|o| o.custom_schema.clone()) {
        jsonld.push(cs);
    } else {
        let items: Vec<serde_json::Value> = top
            .iter()
            .enumerate()
            .map(|(i, b)| {
                serde_json::json!({
                    "@type": "ListItem",
                    "position": i + 1,
                    "url": format!("{}/{}", canonical, format!("businesses/{}", b.slug)),
                    "name": b.name,
                })
            })
            .collect();
        jsonld.push(serde_json::json!({
            "@context": "https://schema.org",
            "@type": "CollectionPage",
            "name": title,
            "description": description,
            "url": canonical,
            "about": {
                "@type": "City",
                "name": dir.city,
                "containedInPlace": {"@type": "AdministrativeArea", "name": dir.state},
            },
            "mainEntity": {
                "@type": "ItemList",
                "numberOfItems": total,
                "itemListElement": items,
            },
        }));
        jsonld.push(breadcrumb_ld(&[
            (&site, &format!("{}/", base)),
            (&format!("{}, {}", dir.city, dir.state), &canonical),
        ]));
    }

    let seo = Seo {
        title,
        description: description.clone(),
        canonical: canonical.clone(),
        og_type: "website".into(),
        og_image,
        jsonld,
    };

    // The city page IS the homepage document — identical markup, CSS, nav and
    // components — carrying only the city's own <head> and (card W36) a
    // server-rendered trail + onward links that work without JavaScript.
    let nav_html = city_nav_html(pool, &dir, &base, &canonical).await;
    // The city's own listings in the served HTML — see city_hub_html: they live inside
    // `#app`, which the SPA replaces on boot, so only non-JavaScript readers and crawlers
    // ever see them and no visitor gets the list twice.
    let app_html = city_hub_html(&dir, &site, total, &top, city_intro.as_deref());
    let body = inject_city_document(spa_html, &seo, &site, &theme, dir.id, &nav_html, &app_html);

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Businesses list — GET /<slug>/businesses
// ─────────────────────────────────────────────────────────────────────────────

pub async fn businesses_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    query: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;

    let params = parse_query(query);
    let page = params
        .get("page")
        .and_then(|p| p.parse::<i64>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(1);
    let per = 24i64;
    let category = params.get("category").cloned().filter(|c| !c.is_empty());

    let canonical = if page > 1 {
        format!("{}/{}/businesses?page={}", base, dir.slug, page)
    } else {
        format!("{}/{}/businesses", base, dir.slug)
    };

    let total: i64 = {
        let q = if category.is_some() {
            "SELECT COUNT(*) FROM businesses WHERE directory_id = $1 AND is_active = true \
             AND lower(coalesce(category::text,'')) = lower($2)"
        } else {
            "SELECT COUNT(*) FROM businesses WHERE directory_id = $1 AND is_active = true"
        };
        if let Some(c) = &category {
            sqlx::query_scalar(q)
                .bind(dir.id)
                .bind(c)
                .fetch_one(pool)
                .await
        } else {
            sqlx::query_scalar(q).bind(dir.id).fetch_one(pool).await
        }
        .unwrap_or(0)
    };

    let items = list_businesses(pool, &dir, category.as_deref(), per, (page - 1) * per).await;

    let label = match &category {
        Some(c) => format!("{} Businesses in {}, {}", c, dir.city, dir.state),
        None => format!("All Local Businesses in {}, {}", dir.city, dir.state),
    };
    // B119: a category-page pattern (`{category} in {city} | ZaarHub`) applies only when a
    // category is present, so the all-businesses view keeps its own default.
    let (fb_title, fb_desc) = match &category {
        Some(c) => seo_pattern(pool, dir.id, "category", &[("category", c.as_str())]).await,
        None => (None, None),
    };
    let title = fb_title.unwrap_or_else(|| format!("{} | {}", label, site));
    let description = fb_desc.unwrap_or_else(|| {
        clip(
            &format!(
                "Browse {} of {} local businesses in {}, {}. Ratings, reviews, addresses and contact details.",
                items.len(),
                total,
                dir.city,
                dir.state
            ),
            158,
        )
    });
    let description = clip(&description, 158);

    let mut jsonld = vec![serde_json::json!({
        "@context": "https://schema.org",
        "@type": "ItemList",
        "name": label,
        "numberOfItems": total,
        "itemListElement": items.iter().enumerate().map(|(i, b)| serde_json::json!({
            "@type": "ListItem",
            "position": (page-1)*per + i as i64 + 1,
            "url": format!("{}/{}/businesses/{}", base, dir.slug, b.slug),
            "name": b.name,
        })).collect::<Vec<_>>(),
    })];
    jsonld.push(breadcrumb_ld(&[
        (&site, &format!("{}/", base)),
        (&dir.name, &format!("{}/{}/", base, dir.slug)),
        ("Businesses", &format!("{}/{}/businesses", base, dir.slug)),
    ]));

    let seo = Seo {
        title,
        description,
        canonical: canonical.clone(),
        og_type: "website".into(),
        og_image: None,
        jsonld,
    };

    let cards: String = items
        .iter()
        .map(|b| card_html(&base, &dir.slug, b))
        .collect();
    let total_pages = ((total as f64) / (per as f64)).ceil().max(1.0) as i64;
    let mut pager = String::new();
    let qs_cat = category
        .as_ref()
        .map(|c| format!("&category={}", urlencode(c)))
        .unwrap_or_default();
    if page > 1 {
        pager.push_str(&format!(
            "<a class=\"btn\" href=\"{base}/{slug}/businesses?page={p}{cat}\">← Previous</a>",
            base = h(&base),
            slug = h(&dir.slug),
            p = page - 1,
            cat = qs_cat
        ));
    }
    if page < total_pages {
        pager.push_str(&format!(
            "<a class=\"btn\" href=\"{base}/{slug}/businesses?page={p}{cat}\">Next →</a>",
            base = h(&base),
            slug = h(&dir.slug),
            p = page + 1,
            cat = qs_cat
        ));
    }
    let pager = if pager.is_empty() {
        String::new()
    } else {
        format!("<div class=\"pager\">{}</div>", pager)
    };

    // Category links: the businesses index is the hub every category page hangs off,
    // so this block is what makes the category pages reachable by a crawler and by a
    // visitor (and passes the city page's inbound links down to every category).
    let cat_chips = category_chips(pool, &dir, &base, None).await;
    // Towns: the crawl path from the businesses hub DOWN to each satellite town page
    // (W26). Real <a href> links, so every town is one hop from the city's business
    // index — and the city is one hop back up from the town.
    let town_chips = town_chips_html(pool, &dir, &base, None).await;
    let towns_block = if town_chips.is_empty() {
        String::new()
    } else {
        format!(
            "<h2>Towns in {}</h2>\n<div class=\"chips\">{}</div>",
            h(&dir.city),
            town_chips
        )
    };

    let explore = explore_block(pool, &dir, &base, &canonical, Vec::new()).await;

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">{n} businesses · page {page} of {pages}</p>
<div class="grid">{cards}</div>
<h2>Browse by category</h2>
<div class="chips">{cat_chips}</div>
{towns_block}
{pager}
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Businesses", &canonical),
        ]),
        label = h(&label),
        n = total,
        page = page,
        pages = total_pages,
        cards = cards,
        pager = pager,
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Category page — GET /<slug>/<category-slug>
// ─────────────────────────────────────────────────────────────────────────────

/// Links to every category that has listings in this directory. The internal-link
/// backbone: the businesses index links here once, and every category page links to
/// its siblings, so no category page is an orphan and every listing below inherits
/// the inbound links of its category.
async fn category_chips(
    pool: &PgPool,
    dir: &DirectoryRec,
    base: &str,
    exclude: Option<&str>,
) -> String {
    let rows = sqlx::query(
        "SELECT dc.slug, dc.name, COUNT(b.id) AS n \
         FROM directory_categories dc \
         JOIN businesses b ON b.category_id = dc.id AND b.is_active = true \
         WHERE b.directory_id = $1 AND coalesce(dc.slug,'') <> '' \
         GROUP BY dc.slug, dc.name \
         ORDER BY COUNT(b.id) DESC, dc.name",
    )
    .bind(dir.id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut out = String::new();
    for r in &rows {
        let s: String = r.try_get("slug").unwrap_or_default();
        let name: String = r.try_get("name").unwrap_or_default();
        let n: i64 = r.try_get("n").unwrap_or(0);
        if s.is_empty() || name.is_empty() || exclude == Some(s.as_str()) {
            continue;
        }
        out.push_str(&format!(
            "<a class=\"chip\" href=\"{}/{}/{}\">{} ({})</a>",
            h(base),
            h(&dir.slug),
            h(&s),
            h(&name),
            n
        ));
    }
    out
}

/// One crawlable page per category per city (`/palm-bay/roofing`).
///
/// Before this, a city's only server-rendered listing surfaces were its home and
/// `/businesses`: a search like "roofers in Palm Bay" had no page naming the
/// category, so demand for every category landed on one city URL and every business
/// below it inherited that page's rank. Each category with real listings now gets
/// its own URL — its own title/description, a self-canonical, an ItemList of every
/// business in it (each a real `<a href>` to the server-rendered listing), a
/// breadcrumb trail and links to its sibling categories.
///
/// Returns `None` when the slug is not a category with at least one active business
/// here, so the caller keeps its genuine 404 — never a thin page, never the SPA
/// shell.
pub async fn category_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    cat_slug: &str,
    _query: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // A category is network-wide (`directory_id IS NULL`, how the factory seeds
    // them) or owned by one directory; a directory's own row wins over the shared one.
    let row = match sqlx::query(
        "SELECT dc.name, dc.slug FROM directory_categories dc \
         WHERE dc.slug = $1 AND (dc.directory_id = $2 OR dc.directory_id IS NULL) \
         ORDER BY dc.directory_id NULLS LAST LIMIT 1",
    )
    .bind(cat_slug)
    .bind(dir.id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        _ => return None,
    };
    let cat_name: String = row.try_get("name").unwrap_or_default();
    let cat_slug: String = row.try_get("slug").unwrap_or_default();
    if cat_name.is_empty() || cat_slug.is_empty() {
        return None;
    }

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses b \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE b.directory_id = $1 AND b.is_active = true \
           AND lower(coalesce(dc.name,'')) = lower($2)",
    )
    .bind(dir.id)
    .bind(&cat_name)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    if total == 0 {
        return None;
    }

    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;
    let canonical = format!("{}/{}/{}", base, dir.slug, cat_slug);
    let label = format!("{} in {}, {}", cat_name, dir.city, dir.state);

    // The admin's own category pattern applies here too (same pattern the
    // /businesses?category= view uses), so one pattern serves both surfaces.
    let (fb_title, fb_desc) =
        seo_pattern(pool, dir.id, "category", &[("category", cat_name.as_str())]).await;
    let title = fb_title.unwrap_or_else(|| format!("{} | {}", label, site));
    let description = clip(
        &fb_desc.unwrap_or_else(|| {
            format!(
                "Browse {} businesses listed under {} in {}, {}. Ratings, reviews, addresses and contact details.",
                total, cat_name, dir.city, dir.state
            )
        }),
        158,
    );

    let items = list_businesses(pool, &dir, Some(&cat_name), 100, 0).await;

    let mut jsonld = vec![serde_json::json!({
        "@context": "https://schema.org",
        "@type": "ItemList",
        "name": label,
        "numberOfItems": total,
        "itemListElement": items.iter().enumerate().map(|(i, b)| serde_json::json!({
            "@type": "ListItem",
            "position": i + 1,
            "url": format!("{}/{}/businesses/{}", base, dir.slug, b.slug),
            "name": b.name,
        })).collect::<Vec<_>>(),
    })];
    jsonld.push(breadcrumb_ld(&[
        (&site, &format!("{}/", base)),
        (&dir.name, &format!("{}/{}/", base, dir.slug)),
        ("Businesses", &format!("{}/{}/businesses", base, dir.slug)),
        (&label, &canonical),
    ]));

    let canonical_crumb = canonical.clone();
    let seo = Seo {
        title,
        description,
        canonical,
        og_type: "website".into(),
        og_image: None,
        jsonld,
    };

    let cards: String = items
        .iter()
        .map(|b| card_html(&base, &dir.slug, b))
        .collect();
    let chips = category_chips(pool, &dir, &base, Some(&cat_slug)).await;

    let explore = explore_block(
        pool,
        &dir,
        &base,
        &canonical_crumb,
        items
            .iter()
            .take(3)
            .map(|b| {
                (
                    b.name.clone(),
                    format!("{}/{}/businesses/{}", base, dir.slug, b.slug),
                )
            })
            .collect(),
    )
    .await;

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">Browse {n} businesses listed under {cat} in {city}, {state} — rated and reviewed by locals.</p>
<div class="grid">{cards}</div>
<h2>More categories in {city}</h2>
<div class="chips">{chips}</div>
<p><a class="btn" href="/{dir}/businesses">See all {city} businesses →</a></p>
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Businesses", &format!("{}/{}/businesses", base, dir.slug)),
            (&label, &canonical_crumb),
        ]),
        label = h(&label),
        n = total,
        cat = h(&cat_name),
        city = h(&dir.city),
        state = h(&dir.state),
        cards = cards,
        chips = chips,
        dir = h(&dir.slug),
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Business detail — GET /<slug>/businesses/<slug-or-id>
// ─────────────────────────────────────────────────────────────────────────────

pub async fn business_detail(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    ident: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;

    let by_uuid = Uuid::parse_str(ident).ok();
    let fetched = sqlx::query(
        "SELECT id, name, slug, description, category_id, address, city, state, zip, phone, \
                website, logo_url, cover_url, images, rating, review_count, latitude, longitude, featured \
         FROM businesses WHERE directory_id = $1 AND is_active = true \
           AND (lower(slug) = lower($2) OR ($3::uuid IS NOT NULL AND id = $3::uuid)) LIMIT 1",
    )
    .bind(dir.id)
    .bind(ident)
    .bind(by_uuid)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    // A listing the owner renamed (card W19): the address in the link may be one this page
    // used before. Answer a permanent redirect to the current address so old links, printed
    // QR codes and search results keep working instead of landing on a dead page.
    let row = match fetched {
        Some(r) => r,
        None => {
            let current: Option<String> = sqlx::query_scalar(
                "SELECT b.slug FROM business_slug_history h \
                 JOIN businesses b ON b.id = h.business_id \
                 WHERE h.directory_id = $1 AND lower(h.slug) = lower($2) \
                   AND b.is_active = true AND coalesce(b.slug, '') <> '' LIMIT 1",
            )
            .bind(dir.id)
            .bind(ident)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            return match current {
                Some(s) => Some(redirect_301(&format!(
                    "{}/{}/businesses/{}",
                    origin(host, proto, fallback_domain),
                    dir.slug,
                    s
                ))),
                None => None,
            };
        }
    };

    let biz_id: Uuid = row.try_get("id").ok()?;
    let name: String = row.try_get("name").unwrap_or_default();
    let biz_slug: String = row.try_get("slug").unwrap_or_default();
    let description: String = row
        .try_get::<Option<String>, _>("description")
        .unwrap_or(None)
        .unwrap_or_default();
    // B119: the business's own description is authored copy — resolve the shared
    // merge vocabulary (per directory, plus this business) before it is displayed
    // or used as the meta description.
    let mut mctx = crate::merge_fields::MergeContext::for_directory(pool, dir.id).await;
    mctx.set("business_name", &name);
    let description = crate::merge_fields::render(&description, &mctx).text;
    let address: Option<String> = row.try_get("address").unwrap_or(None);
    let rec_city: Option<String> = row
        .try_get::<Option<String>, _>("city")
        .unwrap_or(None)
        .filter(|c| !c.trim().is_empty());
    // The directory's own city is the canonical locality (records sometimes carry a
    // truncated city such as "Palm" for "Palm Bay"); fall back to the record.
    let city: String = if !dir.city.trim().is_empty() {
        dir.city.clone()
    } else {
        rec_city.clone().unwrap_or_default()
    };
    let state: String = row
        .try_get::<Option<String>, _>("state")
        .unwrap_or(None)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| dir.state.clone());
    let zip: Option<String> = row.try_get("zip").unwrap_or(None);
    let phone: Option<String> = row.try_get("phone").unwrap_or(None);
    let website: Option<String> = row.try_get("website").unwrap_or(None);
    let logo: Option<String> = row.try_get("logo_url").unwrap_or(None);
    let cover: Option<String> = row.try_get("cover_url").unwrap_or(None);
    // The listing's own photo (card W24 part 2). A Google Places photo NAME is not a URL, so an
    // imported listing had nothing to render and this page showed no picture at all. The app's
    // own /api/v1/place-photo proxy turns that name into bytes on demand; only a real http(s)
    // URL or that proxy path is ever used here, never a path that could 404.
    let images_json: serde_json::Value = row.try_get("images").unwrap_or(serde_json::Value::Null);
    let hero_src: Option<String> = images_json.as_array().and_then(|arr| {
        arr.iter().enumerate().find_map(|(i, item)| {
            let raw = match item {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(o) => o
                    .get("url")
                    .or_else(|| o.get("photo_reference"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_default(),
                _ => return None,
            };
            crate::handlers::place_photos::display_src(biz_id, i, &raw).filter(|src| {
                src.starts_with("https://")
                    || src.starts_with("http://")
                    || src.starts_with("/api/v1/place-photo/")
            })
        })
    });
    let rating: f64 = row
        .try_get::<Option<f64>, _>("rating")
        .unwrap_or(None)
        .unwrap_or(0.0);
    let reviews: i32 = row
        .try_get::<Option<i32>, _>("review_count")
        .unwrap_or(None)
        .unwrap_or(0);
    let lat: Option<f64> = row.try_get("latitude").unwrap_or(None);
    let lng: Option<f64> = row.try_get("longitude").unwrap_or(None);

    // Category label (best-effort; falls back to the business category id).
    let category_id: Option<Uuid> = row.try_get("category_id").unwrap_or(None);
    let category: Option<String> = {
        if let Some(cid) = category_id {
            sqlx::query_scalar::<_, String>("SELECT name FROM directory_categories WHERE id = $1")
                .bind(cid)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
        } else {
            None
        }
    };
    // The slug of that category's crawlable page (`/<city>/<category-slug>`), resolved with the
    // same rule the category page uses (a directory's own row wins over the network-wide one) so
    // the link only ever points at a page that renders. Without it the category surface is linked
    // from the hub alone and a crawler reaching a business cannot walk up to its category.
    let category_slug: Option<String> = match &category {
        Some(c) => sqlx::query_scalar::<_, String>(
            "SELECT slug FROM directory_categories \
             WHERE lower(name) = lower($1) AND (directory_id = $2 OR directory_id IS NULL) \
             ORDER BY directory_id NULLS LAST LIMIT 1",
        )
        .bind(c)
        .bind(dir.id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty()),
        None => None,
    };

    // Related listings: same category first, then nearest (when we know where this one is),
    // then best rated. Every card is a real crawlable href to that listing's own page, so a
    // crawler arriving at any business can walk on to its neighbours instead of dead-ending,
    // and a shopper always has somewhere to go next.
    let related: Vec<BizCard> = match sqlx::query(
        "SELECT b.name, b.slug, b.rating, b.review_count, b.address, dc.name AS category \
         FROM businesses b \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         WHERE b.directory_id = $1 AND b.is_active = true AND b.id <> $2 \
           AND coalesce(b.slug, '') <> '' \
         ORDER BY CASE WHEN $3::uuid IS NOT NULL AND b.category_id = $3::uuid THEN 0 ELSE 1 END, \
                  CASE WHEN $4::float8 IS NOT NULL AND $5::float8 IS NOT NULL \
                            AND b.latitude IS NOT NULL AND b.longitude IS NOT NULL \
                       THEN (b.latitude - $4::float8) ^ 2 + (b.longitude - $5::float8) ^ 2 \
                       ELSE 1e9 END, \
                  b.rating DESC NULLS LAST, b.review_count DESC \
         LIMIT 6",
    )
    .bind(dir.id)
    .bind(biz_id)
    .bind(category_id)
    .bind(lat)
    .bind(lng)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows
            .iter()
            .map(|r| BizCard {
                name: r.try_get("name").unwrap_or_default(),
                slug: r.try_get("slug").unwrap_or_default(),
                category: r.try_get("category").unwrap_or(None),
                rating: r
                    .try_get::<Option<f64>, _>("rating")
                    .unwrap_or(None)
                    .unwrap_or(0.0),
                review_count: r
                    .try_get::<Option<i32>, _>("review_count")
                    .unwrap_or(None)
                    .unwrap_or(0),
                address: r.try_get("address").unwrap_or(None),
                // Not selected by this query — the related grid stays a name/rating grid.
                description: None,
            })
            .collect(),
        Err(e) => {
            // Decorative section: never fail the page over it, but never swallow it silently
            // either (that is how the related-table export gap stayed invisible).
            eprintln!("related businesses query failed for {biz_id}: {e}");
            Vec::new()
        }
    };
    let related_html = if related.is_empty() {
        // No neighbours to show as cards: fall back to the shared block so the page
        // still cannot dead-end (card W36).
        explore_block(
            pool,
            &dir,
            &base,
            &format!("{}/{}/businesses/{}", base, dir.slug, biz_slug),
            Vec::new(),
        )
        .await
    } else {
        let cards: Vec<String> = related
            .iter()
            .map(|b| card_html(&base, &dir.slug, b))
            .collect();
        let heading = match &category {
            Some(c) => format!("More {} in {}", h(c), h(&city)),
            None => format!("Similar businesses in {}", h(&city)),
        };
        // The card grid sits inside the one shared "Keep exploring" section, together
        // with the city's browse levels, so this page offers both a next listing and a
        // way back up — never a dead end (card W36).
        let mut chips = String::new();
        if let Some(cs) = &category_slug {
            chips.push_str(&format!(
                "<a class=\"chip\" href=\"{}/{}/{}\">All {}</a>",
                h(&base),
                h(&dir.slug),
                h(cs),
                h(category.as_deref().unwrap_or(""))
            ));
        }
        chips.push_str(&format!(
            "<a class=\"chip\" href=\"{}/{}/businesses\">All businesses in {}</a>",
            h(&base),
            h(&dir.slug),
            h(&city)
        ));
        chips.push_str(&format!(
            "<a class=\"chip\" href=\"{}/{}\">{} home</a>",
            h(&base),
            h(&dir.slug),
            h(&city)
        ));
        chips.push_str(&format!(
            "<a class=\"chip\" href=\"{}/{}/blog\">{} blog</a>",
            h(&base),
            h(&dir.slug),
            h(&city)
        ));
        format!(
            "<section class=\"explore\" aria-label=\"Keep exploring\"><h2>Keep exploring</h2>\
             <div class=\"explore-group\"><h3>{}</h3><div class=\"grid\">{}</div></div>\
             <div class=\"explore-group\"><h3>Browse {}</h3><div class=\"chips\">{}</div></div></section>",
            heading,
            cards.join(""),
            h(&city),
            chips
        )
    };

    let canonical = format!("{}/{}/businesses/{}", base, dir.slug, biz_slug);
    // One trail for both the visible crumbs and the BreadcrumbList JSON-LD: Home › City ›
    // Businesses › Category › Business, so every level of the new category surface is one
    // click — and one hop for a crawler — from a business.
    let crumb_items: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = vec![
            (site.clone(), format!("{}/", base)),
            (dir.name.clone(), format!("{}/{}/", base, dir.slug)),
            (
                "Businesses".to_string(),
                format!("{}/{}/businesses", base, dir.slug),
            ),
        ];
        if let (Some(c), Some(s)) = (&category, &category_slug) {
            v.push((c.clone(), format!("{}/{}/{}", base, dir.slug, s)));
        }
        v.push((name.clone(), canonical.clone()));
        v
    };
    let crumb_pairs: Vec<(&str, &str)> = crumb_items
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let ov = seo_override(pool, "business", biz_id).await;

    // B119: business-page pattern (e.g. `{business_name} in {city}, {state} | ZaarHub`).
    let (fb_title, fb_desc) = seo_pattern(
        pool,
        dir.id,
        "business",
        &[("business_name", name.as_str())],
    )
    .await;

    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .or(fb_title)
        .unwrap_or_else(|| match &category {
            Some(c) => format!("{} — {} in {}, {} | {}", name, c, city, state, site),
            None => format!("{} in {}, {} | {}", name, city, state, site),
        });

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(fb_desc)
        .unwrap_or_else(|| {
            if !description.trim().is_empty() {
                strip_tags(&description)
            } else {
                format!(
                    "{} is a local business in {}, {}{}. View address, phone number, website and reviews.",
                    name,
                    city,
                    state,
                    category
                        .as_ref()
                        .map(|c| format!(" offering {}", c))
                        .unwrap_or_default()
                )
            }
        });
    let description = clip(&description, 158);
    let og_image = ov
        .as_ref()
        .and_then(|o| o.og_image.clone())
        .or_else(|| {
            hero_src.as_ref().map(|s| {
                if s.starts_with('/') {
                    format!("{}{}", base.trim_end_matches('/'), s)
                } else {
                    s.clone()
                }
            })
        })
        .or_else(|| logo.clone())
        .or_else(|| cover.clone());

    // Approved customer reviews. The page used to publish only the aggregate ("★ 4.6 · 87
    // reviews") with no review text anywhere, so a reader — and a crawler — saw the claim and
    // never the evidence, and reviews a business had already approved were invisible on the one
    // page search sends people to. Only `status = 'approved'` rows are ever shown.
    struct PageReview {
        who: String,
        rating: i32,
        when: Option<chrono::DateTime<chrono::Utc>>,
        content: String,
        response: Option<String>,
    }
    let page_reviews: Vec<PageReview> = match sqlx::query(
        "SELECT reviewer_name, rating, content, created_at, owner_response FROM reviews \
         WHERE business_id = $1 AND status = 'approved' \
         ORDER BY featured DESC NULLS LAST, rating DESC, created_at DESC LIMIT 10",
    )
    .bind(biz_id)
    .fetch_all(pool)
    .await
    {
        Ok(rs) => rs
            .iter()
            .map(|r| PageReview {
                who: r
                    .try_get::<Option<String>, _>("reviewer_name")
                    .unwrap_or(None)
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "A customer".to_string()),
                rating: r.try_get::<i32, _>("rating").unwrap_or(0),
                when: r
                    .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at")
                    .unwrap_or(None),
                content: r
                    .try_get::<Option<String>, _>("content")
                    .unwrap_or(None)
                    .unwrap_or_default(),
                response: r
                    .try_get::<Option<String>, _>("owner_response")
                    .unwrap_or(None)
                    .filter(|s| !s.trim().is_empty()),
            })
            .collect(),
        Err(e) => {
            // Never fail the page over the decorative section, but never hide the failure
            // either (that is how the related-table export gap stayed invisible).
            eprintln!("reviews query failed for {biz_id}: {e}");
            Vec::new()
        }
    };
    let stars_for = |r: i32| -> String {
        let full = r.clamp(0, 5) as usize;
        format!("{}{}", "★".repeat(full), "☆".repeat(5 - full))
    };
    let reviews_html = if page_reviews.is_empty() {
        String::new()
    } else {
        let mut cards = String::new();
        for rv in &page_reviews {
            let when = rv
                .when
                .map(|d| d.format("%B %Y").to_string())
                .unwrap_or_default();
            let when_html = if when.is_empty() {
                String::new()
            } else {
                format!(" · {}", h(&when))
            };
            let body_html = if rv.content.trim().is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", h(rv.content.trim()))
            };
            let resp_html = match &rv.response {
                Some(o) => format!(
                    "<div class=\"resp\"><strong>Response from {}</strong><br>{}</div>",
                    h(&name),
                    h(o)
                ),
                None => String::new(),
            };
            cards.push_str(&format!(
                "<article class=\"review\"><div class=\"who\"><span class=\"stars\">{}</span> · \
                 <strong>{}</strong>{}</div>{}{}</article>",
                stars_for(rv.rating),
                h(&rv.who),
                when_html,
                body_html,
                resp_html
            ));
        }
        format!(
            "<section class=\"reviews\" aria-label=\"Customer reviews\"><h2>Reviews</h2>{}</section>",
            cards
        )
    };
    // Review markup for the crawler: only reviews that actually exist here, with the author
    // and rating Google needs; a rating-only row carries no text, so it stays out.
    let review_ld: Vec<serde_json::Value> = page_reviews
        .iter()
        .filter(|r| !r.content.trim().is_empty())
        .map(|r| {
            let mut o = serde_json::json!({
                "@type": "Review",
                "author": {"@type": "Person", "name": r.who},
                "reviewRating": {"@type": "Rating", "ratingValue": r.rating, "bestRating": 5},
                "reviewBody": r.content.trim(),
            });
            if let Some(d) = r.when {
                o["datePublished"] = serde_json::json!(d.format("%Y-%m-%d").to_string());
            }
            o
        })
        .collect();

    // JSON-LD LocalBusiness — includes the city / area served, auto-derived.
    let mut jsonld = Vec::new();
    if let Some(cs) = ov.as_ref().and_then(|o| o.custom_schema.clone()) {
        jsonld.push(cs);
    } else {
        let mut lb = serde_json::json!({
            "@context": "https://schema.org",
            "@type": "LocalBusiness",
            "@id": canonical,
            "name": name,
            "url": canonical,
            "description": description,
            "address": {
                "@type": "PostalAddress",
                "streetAddress": address.clone().unwrap_or_default(),
                "addressLocality": city,
                "addressRegion": state,
                "postalCode": zip.clone().unwrap_or_default(),
            },
            "areaServed": [
                {"@type": "City", "name": city},
                {"@type": "AdministrativeArea", "name": state},
            ],
        });
        if let Some(p) = &phone {
            lb["telephone"] = serde_json::json!(p);
        }
        if let Some(w) = &website {
            lb["sameAs"] = serde_json::json!([w]);
        }
        if let Some(img) = &og_image {
            lb["image"] = serde_json::json!(img);
        }
        if let (Some(la), Some(ln)) = (lat, lng) {
            lb["geo"] =
                serde_json::json!({"@type": "GeoCoordinates", "latitude": la, "longitude": ln});
        }
        if rating > 0.0 && reviews > 0 {
            lb["aggregateRating"] = serde_json::json!({
                "@type": "AggregateRating",
                "ratingValue": rating,
                "reviewCount": reviews,
                "bestRating": 5,
            });
        }
        if !review_ld.is_empty() {
            lb["review"] = serde_json::json!(review_ld);
        }
        jsonld.push(lb);
        jsonld.push(breadcrumb_ld(&crumb_pairs));
    }

    let seo = Seo {
        title,
        description: description.clone(),
        canonical: canonical.clone(),
        og_type: "business.business".into(),
        og_image: og_image.clone(),
        jsonld,
    };

    let mut rows = String::new();
    if let Some(a) = &address {
        rows.push_str(&format!("<div class=\"row\">📍 {}</div>", h(a)));
    }
    if let Some(p) = &phone {
        rows.push_str(&format!(
            "<div class=\"row\">📞 <a href=\"tel:{p}\">{p}</a></div>",
            p = h(p)
        ));
    }
    if let Some(w) = &website {
        let (clean, label) = crate::utils::url_cleaner::clean_url_pair(w);
        rows.push_str(&format!(
            "<div class=\"row\">🌐 <a href=\"{}\" rel=\"noopener nofollow\" target=\"_blank\">{}</a></div>",
            h(&clean),
            h(&label)
        ));
    }
    if let (Some(la), Some(ln)) = (lat, lng) {
        rows.push_str(&format!(
            "<div class=\"row\">🗺️ <a href=\"https://maps.google.com/?q={},{}&amp;query_place_id=\" rel=\"noopener nofollow\" target=\"_blank\">View on the map</a></div>",
            la, ln
        ));
    }
    let rating_html = if rating > 0.0 {
        format!(
            "<div class=\"row\"><span class=\"stars\">★ {:.1}</span> · {} {}</div>",
            rating,
            reviews,
            if reviews == 1 { "review" } else { "reviews" }
        )
    } else {
        String::new()
    };
    let logo_html = match &logo {
        Some(l) if !l.is_empty() => format!(
            "<img src=\"{}\" alt=\"{}\" style=\"width:84px;height:84px;border-radius:16px;object-fit:cover;float:right\">",
            h(l),
            h(&name)
        ),
        _ => String::new(),
    };
    let desc_html = if description.is_empty() {
        String::new()
    } else {
        format!("<p class=\"lede\">{}</p>", h(&description))
    };
    // The picture at the top of the page. Decorative when it is absent — the block disappears
    // rather than showing an empty frame.
    let hero_html = match &hero_src {
        Some(s) => format!(
            "<img src=\"{}\" alt=\"{}\" loading=\"lazy\" style=\"width:100%;max-height:420px;object-fit:cover;border-radius:14px;margin:0 0 18px\">",
            h(s),
            h(&name)
        ),
        None => String::new(),
    };

    // Expose this page's own business + directory to the injected visitor tracker, so a
    // click or scroll lands attributed instead of directory-less / category-less ("unknown").
    let track_ctx = serde_json::json!({
        "business_id": biz_id,
        "category_id": category_id,
        "directory_id": dir.id,
    });
    let ctx_script = format!(
        "<script>window.__dir_id=\"{}\";window.__biz_ctx={};</script>",
        dir.id, track_ctx
    );

    let body = format!(
        r#"{start}{ctx}{crumbs}
<div class="detail">
{hero}
{logo}
<h1>{name}</h1>
{rating_html}
{desc_html}
<div class="row">{cat}</div>
{rows}
<p class="row"><a href="{base}/{dslug}/businesses">← All businesses in {city}</a></p>
</div>
{reviews}
{related}
{end}"#,
        ctx = ctx_script,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&crumb_pairs),
        logo = logo_html,
        hero = hero_html,
        name = h(&name),
        rating_html = rating_html,
        desc_html = desc_html,
        cat = match (&category, &category_slug) {
            (Some(c), Some(s)) => format!(
                "<div class=\"row\">🏷️ <a href=\"{}/{}/{}\">{}</a></div>",
                h(&base),
                h(&dir.slug),
                h(s),
                h(c)
            ),
            (Some(c), None) => format!("<div class=\"row\">🏷️ {}</div>", h(c)),
            _ => String::new(),
        },
        rows = rows,
        reviews = reviews_html,
        related = related_html,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&city),
        end = shell_end(&site, hq.as_deref()),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Article — GET /<slug>/articles/<article-slug>
// ─────────────────────────────────────────────────────────────────────────────

pub async fn article_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    article_slug: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;

    let row = sqlx::query(
        "SELECT id, title, slug, meta_description, content, keyword, business_id, created_at, updated_at \
         FROM business_articles WHERE directory_id = $1 AND slug = $2 \
           AND status = 'published' LIMIT 1",
    )
    .bind(dir.id)
    .bind(article_slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let art_id: Uuid = row.try_get("id").ok()?;
    let title_raw: String = row.try_get("title").unwrap_or_default();
    let a_slug: String = row.try_get("slug").unwrap_or_default();
    let meta_description: Option<String> = row.try_get("meta_description").unwrap_or(None);
    let content: String = row.try_get("content").unwrap_or(None).unwrap_or_default();
    let keyword: String = row.try_get("keyword").unwrap_or_default();
    let biz_id: Option<Uuid> = row.try_get("business_id").unwrap_or(None);
    let created: Option<chrono::DateTime<chrono::Utc>> = row.try_get("created_at").unwrap_or(None);
    let updated: Option<chrono::DateTime<chrono::Utc>> = row.try_get("updated_at").unwrap_or(None);

    // B119: authored article copy resolves the shared merge vocabulary at render
    // time, per directory/city, so ONE network template serves every city.
    let mctx = crate::merge_fields::MergeContext::for_directory(pool, dir.id).await;
    let title_raw = crate::merge_fields::render(&title_raw, &mctx).text;
    let content = crate::merge_fields::render(&content, &mctx).text;
    let meta_description = meta_description.map(|d| crate::merge_fields::render(&d, &mctx).text);

    let canonical = format!("{}/{}/articles/{}", base, dir.slug, a_slug);
    let ov = seo_override(pool, "article", art_id).await;

    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .unwrap_or_else(|| format!("{} | {}", title_raw, site));

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(meta_description)
        .unwrap_or_else(|| strip_tags(&content));
    let description = clip(&description, 158);

    let published = created
        .or(updated)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let mut jsonld = Vec::new();
    if let Some(cs) = ov.as_ref().and_then(|o| o.custom_schema.clone()) {
        jsonld.push(cs);
    } else {
        jsonld.push(serde_json::json!({
            "@context": "https://schema.org",
            "@type": "Article",
            "headline": clip(&strip_tags(&title_raw), 110),
            "description": description,
            "datePublished": published,
            "dateModified": updated.map(|d| d.to_rfc3339()).unwrap_or_else(|| published.clone()),
            "mainEntityOfPage": {"@type": "WebPage", "@id": canonical},
            "author": {"@type": "Organization", "name": site},
            "publisher": {"@type": "Organization", "name": site},
            "about": {"@type": "Thing", "name": keyword},
            "contentLocation": {
                "@type": "City",
                "name": dir.city,
                "containedInPlace": {"@type": "AdministrativeArea", "name": dir.state},
            },
        }));
        jsonld.push(breadcrumb_ld(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            (&title_raw, &canonical),
        ]));
    }

    let seo = Seo {
        title,
        description: description.clone(),
        canonical: canonical.clone(),
        og_type: "article".into(),
        og_image: ov.as_ref().and_then(|o| o.og_image.clone()),
        jsonld,
    };

    // Link back to the sponsored business when the article has one.
    let biz_html = if let Some(bid) = biz_id {
        sqlx::query("SELECT name, slug FROM businesses WHERE id = $1")
            .bind(bid)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .map(|r| {
                let bn: String = r.try_get("name").unwrap_or_default();
                let bs: String = r.try_get("slug").unwrap_or_default();
                format!(
                    "<p class=\"lede\">Brought to you by <a href=\"{}/{}/businesses/{}\">{}</a>.</p>",
                    h(&base),
                    h(&dir.slug),
                    h(&bs),
                    h(&bn)
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Content is platform-generated HTML; sanitise defensively before embedding.
    let safe_content = crate::template_engine::sanitize_html(&content);
    let date_label = created
        .map(|d| d.format("%B %-d, %Y").to_string())
        .unwrap_or_default();

    // Card W36: the next and the previous piece in this city — a visitor reading one
    // article can always read the next one, and a crawler can walk the whole archive.
    let neighbours = neighbour_links(pool, &dir, &base, created.or(updated), &canonical).await;
    let explore = explore_block(pool, &dir, &base, &canonical, neighbours).await;

    let body = format!(
        r#"{start}{crumbs}
<article class="detail article-body">
<h1>{h1}</h1>
<p class="lede">{date}</p>
{biz}
{content}
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← More about {city}</a></p>
</article>
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            (
                &format!("{} Blog", dir.city),
                &format!("{}/{}/blog", base, dir.slug)
            ),
            (&title_raw, &canonical),
        ]),
        h1 = h(&title_raw),
        date = h(&date_label),
        biz = biz_html,
        content = safe_content,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Blog post — GET /<slug>/blog/<post-slug>  (blog_posts CMS articles)
// ─────────────────────────────────────────────────────────────────────────────

pub async fn blog_post_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
    post_slug: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;

    let row = sqlx::query(
        "SELECT id, title, slug, excerpt, content, meta_title, meta_description, canonical_url, \
                robots_meta, featured_image_url, author_name, blog_category, created_at, updated_at, \
                town_key \
         FROM blog_posts WHERE directory_id = $1 AND slug = $2 \
           AND coalesce(published, true) = true LIMIT 1",
    )
    .bind(dir.id)
    .bind(post_slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    // B186: a published blog_post wins; if none matches, fall back to a published
    // business / sponsored article. The public RSS feed (articles_feed.rs) advertises
    // business_articles under this exact `/<city>/blog/<slug>` url, so without the
    // fallback every sponsored article is advertised but unreachable — the request
    // landed on the SPA shell instead of the article. Same column shape, so the render
    // below is shared unchanged.
    let row = match row {
        Some(r) => Some(r),
        None => {
            sqlx::query(
                "SELECT ba.id, ba.title, ba.slug, NULL::text AS excerpt, ba.content, \
                        NULL::text AS meta_title, ba.meta_description, NULL::text AS canonical_url, \
                        NULL::text AS robots_meta, NULL::text AS featured_image_url, \
                        (SELECT b.name FROM businesses b WHERE b.id = ba.business_id) AS author_name, \
                        ba.keyword AS blog_category, ba.created_at, ba.updated_at, \
                        NULL::text AS town_key \
                 FROM business_articles ba \
                 WHERE ba.directory_id = $1 AND ba.slug = $2 AND ba.status = 'published' LIMIT 1",
            )
            .bind(dir.id)
            .bind(post_slug)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
        }
    }?;

    let post_id: Uuid = row.try_get("id").ok()?;
    let title_raw: String = row.try_get("title").unwrap_or_default();
    let p_slug: String = row.try_get("slug").unwrap_or_default();
    let excerpt: Option<String> = row.try_get("excerpt").unwrap_or(None);
    let content: String = row.try_get("content").unwrap_or(None).unwrap_or_default();
    let meta_title: Option<String> = row.try_get("meta_title").unwrap_or(None);
    let meta_description: Option<String> = row.try_get("meta_description").unwrap_or(None);
    let canonical_override: Option<String> = row.try_get("canonical_url").unwrap_or(None);
    let robots_meta: Option<String> = row.try_get("robots_meta").unwrap_or(None);
    let featured: Option<String> = row.try_get("featured_image_url").unwrap_or(None);
    let author_name: Option<String> = row.try_get("author_name").unwrap_or(None);
    let category: Option<String> = row.try_get("blog_category").unwrap_or(None);
    let created: Option<chrono::DateTime<chrono::Utc>> = row.try_get("created_at").unwrap_or(None);
    let updated: Option<chrono::DateTime<chrono::Utc>> = row.try_get("updated_at").unwrap_or(None);

    // W30(b): an article filed to ONE town must link DOWN into that town's listings,
    // so the town is a real crawl path (article → its businesses → the town page),
    // not a label nobody can follow.
    let town_key: Option<String> = row
        .try_get::<Option<String>, _>("town_key")
        .unwrap_or(None)
        .map(|k: String| k.trim().to_lowercase())
        .filter(|k| !k.is_empty());
    let mut town_label_for_schema: Option<String> = None;
    let mut town_biz_html = String::new();
    if let Some(key) = town_key.as_deref() {
        let towns = local_town_entries(pool, dir.id, &dir.city).await;
        if let Some(town) = towns.iter().find(|t| t.key == key) {
            town_label_for_schema = Some(town.label.clone());
            let items = list_town_businesses(pool, dir.id, &town.key, 6).await;
            let lis: String = items
                .iter()
                .map(|b| {
                    let cat = b
                        .category
                        .as_ref()
                        .filter(|c| !c.trim().is_empty())
                        .map(|c| format!(" — {}", h(c)))
                        .unwrap_or_default();
                    format!(
                        "<li><a href=\"{}/{}/businesses/{}\">{}</a>{}</li>",
                        base,
                        h(&dir.slug),
                        h(&b.slug),
                        h(&b.name),
                        cat
                    )
                })
                .collect();
            let all_link = if town.published {
                format!(
                    "<p><a class=\"btn\" href=\"{}/{}/{}\">All {} businesses →</a></p>",
                    base,
                    h(&dir.slug),
                    h(&town.slug),
                    h(&town.label)
                )
            } else {
                String::new()
            };
            if !lis.is_empty() || !all_link.is_empty() {
                town_biz_html = format!(
                    "<section class=\"town-biz\"><h2>Businesses in {}</h2>\n<ul class=\"article-list\">{}</ul>\n{}</section>",
                    h(&town.label),
                    lis,
                    all_link
                );
            }
        }
    }

    // B119: authored blog copy resolves the shared merge vocabulary at render time.
    let mctx = crate::merge_fields::MergeContext::for_directory(pool, dir.id).await;
    let title_raw = crate::merge_fields::render(&title_raw, &mctx).text;
    let content = crate::merge_fields::render(&content, &mctx).text;
    let excerpt = excerpt.map(|e| crate::merge_fields::render(&e, &mctx).text);
    let meta_title = meta_title.map(|t| crate::merge_fields::render(&t, &mctx).text);
    let meta_description = meta_description.map(|d| crate::merge_fields::render(&d, &mctx).text);

    // The post's own canonical_url wins if set (admin-editable per post);
    // otherwise the subfolder form.
    let canonical = canonical_override
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| format!("{}/{}/blog/{}", base, dir.slug, p_slug));

    let ov = seo_override(pool, "blog_post", post_id).await;
    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .or(meta_title)
        .unwrap_or_else(|| format!("{} | {}", title_raw, site));

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(meta_description)
        .or(excerpt)
        .unwrap_or_else(|| strip_tags(&content));
    let description = clip(&description, 158);

    let published = created
        .or(updated)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let robots = robots_meta
        .filter(|r| !r.trim().is_empty())
        .map(|r| {
            if r.contains("noindex") {
                "noindex,nofollow".to_string()
            } else {
                "index,follow,max-image-preview:large".to_string()
            }
        })
        .unwrap_or_else(|| "index,follow,max-image-preview:large".to_string());

    let og_image = ov
        .as_ref()
        .and_then(|o| o.og_image.clone())
        .or(featured.clone());

    let mut jsonld = Vec::new();
    if let Some(cs) = ov.as_ref().and_then(|o| o.custom_schema.clone()) {
        jsonld.push(cs);
    } else {
        jsonld.push(serde_json::json!({
            "@context": "https://schema.org",
            "@type": "Article",
            "headline": clip(&strip_tags(&title_raw), 110),
            "description": description,
            "datePublished": published,
            "dateModified": updated.map(|d| d.to_rfc3339()).unwrap_or_else(|| published.clone()),
            "mainEntityOfPage": {"@type": "WebPage", "@id": canonical},
            "author": {"@type": "Organization", "name": author_name.clone().unwrap_or_else(|| site.clone())},
            "publisher": {"@type": "Organization", "name": site},
            "contentLocation": {
                "@type": "City",
                "name": town_label_for_schema.clone().unwrap_or_else(|| dir.city.clone()),
                "containedInPlace": {"@type": "AdministrativeArea", "name": dir.state},
            },
        }));
        jsonld.push(breadcrumb_ld(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            (&title_raw, &canonical),
        ]));
    }

    let seo = Seo {
        title,
        description: description.clone(),
        canonical: canonical.clone(),
        og_type: "article".into(),
        og_image,
        jsonld,
    };

    let safe_content = crate::template_engine::sanitize_html(&content);
    let date_label = created
        .map(|d| d.format("%B %-d, %Y").to_string())
        .unwrap_or_default();
    let byline = author_name
        .as_ref()
        .map(|a| format!(" · By {}", h(a)))
        .unwrap_or_default();
    let cat_html = category
        .as_ref()
        .filter(|c| !c.is_empty() && c.as_str() != "general")
        .map(|c| format!("<div class=\"row\">🏷️ {}</div>", h(c)))
        .unwrap_or_default();
    let img_html = featured
        .as_ref()
        .map(|f| {
            format!(
                "<img src=\"{}\" alt=\"{}\" style=\"width:100%;border-radius:14px;margin-bottom:18px\" loading=\"lazy\">",
                h(f),
                h(&title_raw)
            )
        })
        .unwrap_or_default();

    let neighbours = neighbour_links(pool, &dir, &base, created, &canonical).await;
    let explore = explore_block(pool, &dir, &base, &canonical, neighbours).await;

    let body = format!(
        r#"{start}{crumbs}
<article class="detail article-body">
{img}
<h1>{h1}</h1>
<p class="lede">{date}{byline}</p>
{cat}
{content}
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← More about {city}</a></p>
</article>
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            (
                &format!("{} Blog", dir.city),
                &format!("{}/{}/blog", base, dir.slug)
            ),
            (&title_raw, &canonical),
        ]),
        img = img_html,
        h1 = h(&title_raw),
        date = h(&date_label),
        byline = byline,
        cat = cat_html,
        content = format!("{}{}", safe_content, town_biz_html),
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );

    // robots_meta is respected by injecting the override into the served head.
    let _ = robots;
    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// City blog index — GET /<slug>/blog  (server-rendered, no JS required)
// ─────────────────────────────────────────────────────────────────────────────

/// The city's published blog posts as real HTML so the page is indexable and
/// usable when JavaScript never runs. The post predicate is deliberately the
/// same one the public feed uses (`GET /api/v1/zaarhub/cities/:slug/blog-posts`,
/// `zaarhub::list_city_blog_posts`) so the no-JS page, the SPA's own city blog
/// view and the JSON feed can never disagree about what is published.
pub async fn blog_list_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;
    let canonical = format!("{}/{}/blog", base, dir.slug);

    let ov = seo_override(pool, "city_blog", dir.id).await;
    let (fb_title, fb_desc) = seo_pattern(pool, dir.id, "city_blog", &[]).await;
    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .or(fb_title)
        .unwrap_or_else(|| format!("{} Local Blog — News & Guides | {}", dir.city, site));

    // A failed read returns None (the caller falls back to the SPA) rather than
    // painting a misleading "no posts yet" page over a query that never ran.
    let rows = sqlx::query(
        // Sponsored/SEO articles (`business_articles`) are part of the city's published
        // content — the public RSS feed already unions them and the sitemap advertises
        // `/<city>/articles/<slug>`. Listing only `blog_posts` here left every sponsored
        // article invisible on the browse surface a visitor actually lands on, so the
        // index and the feed disagreed about what this city publishes.
        "SELECT slug, title, excerpt, scheduled_at, created_at, featured_image_url, \
                author_name, blog_category, town_label, source_type \
         FROM ( \
           SELECT bp.slug, bp.title, bp.excerpt, bp.scheduled_at, bp.created_at, \
                  bp.featured_image_url, bp.author_name, bp.blog_category, \
                  st.town_label, \
                  'blog'::text AS source_type \
           FROM blog_posts bp \
           LEFT JOIN satellite_towns st \
                  ON st.directory_id = bp.directory_id AND st.town_key = bp.town_key \
           WHERE bp.directory_id = $1 AND bp.published = true AND bp.status = 'published' \
             AND (bp.scheduled_at IS NULL OR bp.scheduled_at <= NOW()) \
           UNION ALL \
           SELECT ba.slug, ba.title, ba.meta_description, NULL::timestamptz, ba.created_at, \
                  NULL::text, (SELECT b.name FROM businesses b WHERE b.id = ba.business_id), \
                  ba.keyword, NULL::text AS town_label, 'article'::text \
           FROM business_articles ba \
           WHERE ba.directory_id = $1 AND ba.status = 'published' \
         ) u \
         ORDER BY COALESCE(scheduled_at, created_at) DESC LIMIT 50",
    )
    .bind(dir.id)
    .fetch_all(pool)
    .await
    .ok()?;

    struct BlogItem {
        title: String,
        url: String,
        date: Option<chrono::DateTime<chrono::Utc>>,
        excerpt: Option<String>,
        image: Option<String>,
        meta_bits: Vec<String>,
    }

    let items: Vec<BlogItem> = rows
        .iter()
        .filter_map(|r| {
            let post_slug: String = r.try_get("slug").unwrap_or_default();
            if post_slug.trim().is_empty() {
                return None;
            }
            let title_raw: String = r.try_get("title").unwrap_or_default();
            let excerpt: Option<String> = r.try_get("excerpt").unwrap_or(None);
            let scheduled: Option<chrono::DateTime<chrono::Utc>> =
                r.try_get("scheduled_at").unwrap_or(None);
            let created: Option<chrono::DateTime<chrono::Utc>> =
                r.try_get("created_at").unwrap_or(None);
            let image: Option<String> = r.try_get("featured_image_url").unwrap_or(None);
            let author: Option<String> = r.try_get("author_name").unwrap_or(None);
            let category: Option<String> = r.try_get("blog_category").unwrap_or(None);
            let town_label: Option<String> = r.try_get("town_label").unwrap_or(None);
            let source: String = r
                .try_get("source_type")
                .unwrap_or_else(|_| "blog".to_string());
            let date = scheduled.or(created);

            let mut meta_bits = Vec::new();
            if let Some(d) = date {
                meta_bits.push(format!("📅 {}", d.format("%B %-d, %Y")));
            }
            if let Some(a) = author.filter(|a| !a.trim().is_empty()) {
                meta_bits.push(format!("✍️ {}", a));
            }
            if let Some(c) = category.filter(|c| !c.is_empty() && c.as_str() != "general") {
                meta_bits.push(format!("🏷️ {}", c));
            }
            // W30(b): a town-scoped article says which town it is about, so a reader
            // scanning the city index can see the local ones.
            if let Some(t) = town_label.filter(|t| !t.trim().is_empty()) {
                meta_bits.push(format!("📍 {}", t));
            }

            Some(BlogItem {
                title: title_raw,
                url: if source == "article" {
                    // A sponsored/SEO article's published page.
                    format!("{}/{}/articles/{}", base, dir.slug, post_slug)
                } else {
                    format!("{}/{}/blog/{}", base, dir.slug, post_slug)
                },
                date,
                excerpt: excerpt.filter(|e| !e.trim().is_empty()),
                image: image.filter(|i| !i.trim().is_empty()),
                meta_bits,
            })
        })
        .collect();

    let cards: String = items
        .iter()
        .map(|it| {
            let img = it
                .image
                .as_ref()
                .map(|src| {
                    format!(
                        "<img src=\"{}\" alt=\"{}\" style=\"width:100%;border-radius:10px;margin-bottom:10px\" loading=\"lazy\">",
                        h(src),
                        h(&it.title)
                    )
                })
                .unwrap_or_default();
            let meta = if it.meta_bits.is_empty() {
                String::new()
            } else {
                format!("<div class=\"meta\">{}</div>", h(&it.meta_bits.join(" · ")))
            };
            let excerpt = it
                .excerpt
                .as_ref()
                .map(|e| {
                    format!(
                        "<p class=\"lede\" style=\"margin:10px 0 0\">{}</p>",
                        h(&clip(e, 200))
                    )
                })
                .unwrap_or_default();
            format!(
                "<article class=\"card\">{img}<h3><a href=\"{url}\">{title}</a></h3>{meta}{excerpt}<p style=\"margin-top:10px\"><a href=\"{url}\" style=\"font-weight:600\">Read more →</a></p></article>",
                img = img,
                url = h(&it.url),
                title = h(&it.title),
                meta = meta,
                excerpt = excerpt,
            )
        })
        .collect();

    let list_html = if cards.is_empty() {
        format!(
            "<div class=\"card\" style=\"text-align:center;padding:40px 24px\"><h3>📝 No posts yet</h3><p class=\"lede\" style=\"margin:8px auto 0\">{} has no published stories yet. Check back soon.</p><p style=\"margin-top:14px\"><a href=\"{}/{}\">← Back to {}</a></p></div>",
            h(&dir.city),
            h(&base),
            h(&dir.slug),
            h(&dir.city)
        )
    } else {
        cards
    };

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(fb_desc)
        .unwrap_or_else(|| {
            if items.is_empty() {
                format!(
                    "Local news, guides and stories for {}. No stories published yet — check back soon.",
                    dir.city
                )
            } else {
                format!(
                    "{} local {} for {} — news, guides and stories from local businesses.",
                    items.len(),
                    if items.len() == 1 { "story" } else { "stories" },
                    dir.city
                )
            }
        });
    let description = clip(&strip_tags(&description), 158);

    let mut jsonld = Vec::new();
    if let Some(cs) = ov.as_ref().and_then(|o| o.custom_schema.clone()) {
        jsonld.push(cs);
    } else {
        jsonld.push(serde_json::json!({
            "@context": "https://schema.org",
            "@type": "Blog",
            "name": format!("{} Local Blog", dir.city),
            "description": description.clone(),
            "url": canonical.clone(),
            "about": {
                "@type": "City",
                "name": dir.city,
                "containedInPlace": {"@type": "AdministrativeArea", "name": dir.state},
            },
            "blogPost": items.iter().map(|it| serde_json::json!({
                "@type": "BlogPosting",
                "headline": it.title,
                "url": it.url,
                "datePublished": it.date.map(|d| d.to_rfc3339()),
            })).collect::<Vec<_>>(),
        }));
        jsonld.push(breadcrumb_ld(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Blog", &canonical),
        ]));
    }

    let seo = Seo {
        title,
        description,
        canonical: canonical.clone(),
        og_type: "website".into(),
        og_image: ov.as_ref().and_then(|o| o.og_image.clone()),
        jsonld,
    };

    let label = format!("{} Blog", dir.city);
    let lede = if items.is_empty() {
        format!("Local stories, news and guides from {}.", dir.city)
    } else {
        format!(
            "{} published {} from {} and neighbours.",
            items.len(),
            if items.len() == 1 { "story" } else { "stories" },
            dir.city
        )
    };

    let explore = explore_block(pool, &dir, &base, &canonical, Vec::new()).await;

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">{lede}</p>
<div class="grid">{list}</div>
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← Back to {city}</a></p>
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Blog", &canonical),
        ]),
        label = h(&label),
        lede = h(&lede),
        list = list_html,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// City deals — GET /<slug>/deals  (server-rendered, no JS required)
// ─────────────────────────────────────────────────────────────────────────────

/// The city's active deals as real HTML. Same source and predicate as the city
/// landing page's deal carousel (`active_deals` in
/// `GET /api/v1/zaarhub/cities/:slug`, `zaarhub::get_city_page`): the deals tied
/// to a business in this directory with `status = 'active'`. A city with no
/// active deals renders an honest empty state, not a fabricated one.
pub async fn deals_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    slug: &str,
) -> Option<Response<Body>> {
    let dir = load_directory(pool, slug).await?;
    // Brand tokens for this page — shared with the network homepage.
    let theme = theme_for(pool, &dir).await;
    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, &dir).await;
    let hq = site_headquarters(pool).await;
    let canonical = format!("{}/{}/deals", base, dir.slug);

    let ov = seo_override(pool, "city_deals", dir.id).await;
    let (fb_title, fb_desc) = seo_pattern(pool, dir.id, "city_deals", &[]).await;
    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .or(fb_title)
        .unwrap_or_else(|| format!("Deals & Coupons in {}, {} | {}", dir.city, dir.state, site));

    // B119: deal copy is authored per business — resolve the shared vocabulary per
    // directory so a shared template renders each city's own values.
    let mctx = crate::merge_fields::MergeContext::for_directory(pool, dir.id).await;

    let rows = sqlx::query(
        "SELECT de.title, de.description, de.deal_price, de.original_price, \
                de.discount_percent, de.image_url, de.end_date, de.featured, \
                b.name AS business_name, b.slug AS business_slug \
         FROM deals de JOIN businesses b ON b.id = de.business_id \
         WHERE b.directory_id = $1 AND de.status = 'active' \
                AND (de.start_date IS NULL OR de.start_date <= NOW()) \
                AND (de.end_date IS NULL OR de.end_date >= NOW()) \
         ORDER BY de.featured DESC NULLS LAST, de.created_at DESC LIMIT 60",
    )
    .bind(dir.id)
    .fetch_all(pool)
    .await
    .ok()?;

    struct DealItem {
        title: String,
        biz_name: String,
        biz_url: String,
        price: Option<String>,
        was: Option<String>,
        discount: Option<i32>,
        description: Option<String>,
        image: Option<String>,
        until: Option<String>,
    }

    let items: Vec<DealItem> = rows
        .iter()
        .filter_map(|r| {
            let title_raw: String = crate::merge_fields::render(
                &r.try_get::<String, _>("title").unwrap_or_default(),
                &mctx,
            )
            .text;
            if title_raw.trim().is_empty() {
                return None;
            }
            let biz_name: String = r.try_get("business_name").unwrap_or_default();
            let biz_slug: String = r.try_get("business_slug").unwrap_or_default();
            let biz_url = if biz_slug.trim().is_empty() {
                format!("{}/{}/businesses", base, dir.slug)
            } else {
                format!("{}/{}/businesses/{}", base, dir.slug, biz_slug)
            };
            let end_date: Option<chrono::DateTime<chrono::Utc>> =
                r.try_get("end_date").unwrap_or(None);
            Some(DealItem {
                title: title_raw,
                biz_name,
                biz_url,
                price: r.try_get("deal_price").unwrap_or(None),
                was: r.try_get("original_price").unwrap_or(None),
                discount: r.try_get("discount_percent").unwrap_or(None),
                description: r
                    .try_get::<Option<String>, _>("description")
                    .unwrap_or(None)
                    .map(|d| crate::merge_fields::render(&d, &mctx).text),
                image: r.try_get("image_url").unwrap_or(None),
                until: end_date.map(|d| format!("Until {}", d.format("%B %-d, %Y"))),
            })
        })
        .collect();

    let cards: String = items
        .iter()
        .map(|it| {
            let img = it
                .image
                .as_ref()
                .map(|src| {
                    format!(
                        "<img src=\"{}\" alt=\"{}\" style=\"width:100%;border-radius:10px;margin-bottom:10px\" loading=\"lazy\">",
                        h(src),
                        h(&it.title)
                    )
                })
                .unwrap_or_default();
            let biz = format!(
                "<div class=\"meta\"><a href=\"{}\">{}</a></div>",
                h(&it.biz_url),
                h(&it.biz_name)
            );
            let price = match (&it.price, &it.was) {
                (Some(p), Some(w)) => format!(
                    "<span class=\"chip\">{}</span> <s>{}</s>",
                    h(p),
                    h(w)
                ),
                (Some(p), None) => format!("<span class=\"chip\">{}</span>", h(p)),
                _ => String::new(),
            };
            let discount = it
                .discount
                .map(|d| format!("<span class=\"chip\">-{}%</span>", d))
                .unwrap_or_default();
            let desc = it
                .description
                .as_ref()
                .map(|d| {
                    format!(
                        "<p class=\"lede\" style=\"margin:8px 0 0\">{}</p>",
                        h(&clip(d, 180))
                    )
                })
                .unwrap_or_default();
            let until = it
                .until
                .as_ref()
                .map(|u| format!("<div class=\"meta\">{}</div>", h(u)))
                .unwrap_or_default();
            format!(
                "<article class=\"card\">{img}<h3>{title}</h3>{biz}<div class=\"row\">{price}{discount}</div>{desc}{until}<p style=\"margin-top:10px\"><a href=\"{bizurl}\" style=\"font-weight:600\">View {bizname} →</a></p></article>",
                img = img,
                title = h(&it.title),
                biz = biz,
                price = price,
                discount = discount,
                desc = desc,
                until = until,
                bizurl = h(&it.biz_url),
                bizname = h(&it.biz_name),
            )
        })
        .collect();

    let list_html = if cards.is_empty() {
        format!(
            "<div class=\"card\" style=\"text-align:center;padding:40px 24px\"><h3>🎁 No active deals right now</h3><p class=\"lede\" style=\"margin:8px auto 0\">{} has no active deals at the moment. Local businesses add new offers regularly — check back soon.</p><p style=\"margin-top:14px\"><a href=\"{}/{}/businesses\">Browse businesses in {} →</a></p></div>",
            h(&dir.city),
            h(&base),
            h(&dir.slug),
            h(&dir.city)
        )
    } else {
        cards
    };

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(fb_desc)
        .unwrap_or_else(|| {
            if items.is_empty() {
                format!(
                    "No active deals in {} right now. Browse local businesses and check back for new offers.",
                    dir.city
                )
            } else {
                format!(
                    "{} active {} and coupons from local businesses in {}, {}. Save on dining, services and more.",
                    items.len(),
                    if items.len() == 1 { "deal" } else { "deals" },
                    dir.city,
                    dir.state
                )
            }
        });
    let description = clip(&strip_tags(&description), 158);

    let label = format!("Deals & Coupons in {}, {}", dir.city, dir.state);
    let lede = if items.is_empty() {
        format!("No active deals in {} right now.", dir.city)
    } else {
        format!(
            "{} active {} from local businesses.",
            items.len(),
            if items.len() == 1 { "deal" } else { "deals" }
        )
    };

    let mut jsonld = Vec::new();
    if let Some(cs) = ov.as_ref().and_then(|o| o.custom_schema.clone()) {
        jsonld.push(cs);
    } else {
        jsonld.push(serde_json::json!({
            "@context": "https://schema.org",
            "@type": "ItemList",
            "name": label,
            "numberOfItems": items.len(),
            "itemListElement": items.iter().enumerate().map(|(i, it)| serde_json::json!({
                "@type": "ListItem",
                "position": i + 1,
                "name": it.title,
                "url": it.biz_url,
            })).collect::<Vec<_>>(),
        }));
        jsonld.push(breadcrumb_ld(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Deals", &canonical),
        ]));
    }

    let seo = Seo {
        title,
        description,
        canonical: canonical.clone(),
        og_type: "website".into(),
        og_image: ov.as_ref().and_then(|o| o.og_image.clone()),
        jsonld,
    };

    let explore = explore_block(pool, &dir, &base, &canonical, Vec::new()).await;

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">{lede}</p>
<div class="grid">{list}</div>
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← Back to {city}</a></p>
{explore}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Deals", &canonical),
        ]),
        label = h(&label),
        lede = h(&lede),
        list = list_html,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        explore = explore,
        end = shell_end(&site, hq.as_deref()),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sitemap + robots
// ─────────────────────────────────────────────────────────────────────────────

/// `<lastmod>` for one sitemap entry, taken from the row that actually decides the
/// page (city row / listing / category / article / deal).
///
/// Deliberately NEVER a render-time `now()`: a URL that claims it changed every
/// time the sitemap is fetched teaches Google to distrust the field, which is worse
/// than leaving it off. A page whose data carries no timestamp gets no element —
/// Google reads that as "unknown", which is the truth there.
fn lastmod_xml(ts: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match ts {
        Some(ts) => format!(
            "<lastmod>{}</lastmod>",
            ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ),
        None => String::new(),
    }
}

pub async fn sitemap(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
) -> Response<Body> {
    let base = origin(host, proto, fallback_domain);
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );

    // Site-wide pages carry the freshest timestamp of ANY published content: the
    // homepage is a rollup of that content, so it is the page's own modification date.
    let site_lastmod: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>,)>(
            "SELECT GREATEST(\
                 (SELECT max(updated_at) FROM directories WHERE status = 'active'), \
                 (SELECT max(updated_at) FROM blog_posts WHERE coalesce(published, true)), \
                 (SELECT max(updated_at) FROM business_articles WHERE status = 'published'), \
                 (SELECT max(updated_at) FROM deals))",
        )
        .fetch_one(pool)
        .await
        .map(|(t,)| t)
        .unwrap_or(None);

    xml.push_str(&format!(
        "  <url><loc>{}/</loc>{}</url>\n",
        h(&base),
        lastmod_xml(site_lastmod)
    ));

    // Site-wide landing pages that are real content (200 anonymous, linked from listing pages) but
    // are not derived from a table. /claim was live and linked yet never listed here.
    xml.push_str(&format!(
        "  <url><loc>{}/claim</loc>{}</url>\n",
        h(&base),
        lastmod_xml(site_lastmod)
    ));

    let dirs = sqlx::query(
        "SELECT id, slug, city, updated_at FROM directories WHERE status = 'active' ORDER BY slug",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for d in &dirs {
        let dir_id: Uuid = d.try_get("id").unwrap_or_default();
        let dslug: String = d.try_get("slug").unwrap_or_default();
        let dir_ts: Option<chrono::DateTime<chrono::Utc>> = d.try_get("updated_at").unwrap_or(None);

        // Every per-page timestamp is gathered BEFORE emitting, so each entry carries
        // the date of the rows the page is built from — never the date of the render.
        //
        // Per-city category pages (`/<city>/<category-slug>`). Only categories with
        // at least two active listings are advertised: a one-listing category page is
        // thin content, and the sitemap must never publish a URL that renders empty.
        let cats: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
            "SELECT dc.slug, GREATEST(MAX(b.updated_at), MAX(dc.created_at)) AS lastmod \
             FROM directory_categories dc \
             JOIN businesses b ON b.category_id = dc.id AND b.is_active = true \
             WHERE b.directory_id = $1 AND coalesce(dc.slug,'') <> '' \
             GROUP BY dc.slug HAVING COUNT(b.id) >= 2 ORDER BY dc.slug",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        // Businesses (subfolder URLs).
        let biz: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
            "SELECT slug, updated_at FROM businesses WHERE directory_id = $1 AND is_active = true \
             AND coalesce(slug,'') <> '' ORDER BY rating DESC NULLS LAST LIMIT 20000",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        // Articles (business_articles — the SEO article system). Only PUBLISHED rows: the page
        // route (article_page) serves `status='published'` alone, so an archived or draft slug
        // listed here advertises a URL that renders nothing to a crawler. Archived probe copy
        // reached the live sitemap this way.
        let arts: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
            "SELECT slug, updated_at FROM business_articles WHERE directory_id = $1 \
             AND status = 'published' AND coalesce(slug,'') <> '' \
             ORDER BY created_at DESC LIMIT 5000",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        // Blog posts (blog_posts — the CMS article system). Slugs dedup'd: the
        // table has no unique constraint and duplicates would double-list a URL.
        let blogs: Vec<(
            String,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
        )> = sqlx::query_as(
            "SELECT slug, updated_at, created_at FROM blog_posts WHERE directory_id = $1 \
             AND coalesce(published, true) = true AND coalesce(slug,'') <> '' \
             ORDER BY created_at DESC LIMIT 5000",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        let deals_ts: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>,)>(
                "SELECT GREATEST(MAX(updated_at), MAX(created_at)) FROM deals \
             WHERE directory_id = $1",
            )
            .bind(dir_id)
            .fetch_one(pool)
            .await
            .map(|(t,)| t)
            .unwrap_or(None);

        // A list page changes when something ON it changes, never earlier than the city
        // row itself (`chain(dir_ts)` is the floor).
        let biz_ts = biz.iter().filter_map(|(_, t)| *t).chain(dir_ts).max();
        let blog_ts = blogs
            .iter()
            .filter_map(|(_, updated, created)| updated.or(*created))
            .chain(dir_ts)
            .max();
        let deal_ts = deals_ts.into_iter().chain(dir_ts).max();

        // The city landing page renders the city row AND that city's top-rated listings,
        // so its lastmod is the newer of the two — the city row alone reported "unchanged
        // since August" on six of ten cities whose listings had changed in November.
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}</loc>{lm}</url>\n",
            base = h(&base),
            slug = h(&dslug),
            lm = lastmod_xml(biz_ts)
        ));
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}/businesses</loc>{lm}</url>\n",
            base = h(&base),
            slug = h(&dslug),
            lm = lastmod_xml(biz_ts)
        ));
        // The server-rendered city blog index and deals index — the two
        // sub-paths Google should index per city (subfolder canonical form).
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}/blog</loc>{lm}</url>\n",
            base = h(&base),
            slug = h(&dslug),
            lm = lastmod_xml(blog_ts)
        ));
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}/deals</loc>{lm}</url>\n",
            base = h(&base),
            slug = h(&dslug),
            lm = lastmod_xml(deal_ts)
        ));

        for (cslug, cts) in &cats {
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/{c}</loc>{lm}</url>\n",
                base = h(&base),
                d = h(&dslug),
                c = h(cslug),
                lm = lastmod_xml(*cts)
            ));
        }

        // Satellite town pages (`/<city>/<town-slug>`, W26): one crawlable URL per town
        // inside the city's own local area with enough listings to be a real page. A town
        // below the threshold publishes nothing, so the sitemap never advertises a page
        // that would render thin content or 404. The town rollup carries no timestamp of
        // its own, so it takes the city's — conservative, never fresher than the truth.
        let dcity: String = d
            .try_get::<Option<String>, _>("city")
            .unwrap_or(None)
            .unwrap_or_default();
        if !dcity.trim().is_empty() {
            let towns = local_town_entries(pool, dir_id, &dcity).await;
            for t in towns.iter().filter(|t| !t.is_home && t.published) {
                xml.push_str(&format!(
                    "  <url><loc>{base}/{d}/{t}</loc>{lm}</url>\n",
                    base = h(&base),
                    d = h(&dslug),
                    t = h(&t.slug),
                    lm = lastmod_xml(dir_ts)
                ));
            }
        }

        // Each listing page carries its own last edit.
        for (bslug, bts) in &biz {
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/businesses/{b}</loc>{lm}</url>\n",
                base = h(&base),
                d = h(&dslug),
                b = h(bslug),
                lm = lastmod_xml(*bts)
            ));
        }

        for (aslug, ats) in &arts {
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/articles/{a}</loc>{lm}</url>\n",
                base = h(&base),
                d = h(&dslug),
                a = h(aslug),
                lm = lastmod_xml(*ats)
            ));
        }

        let mut seen = std::collections::HashSet::new();
        for (bslug, bupd, bcreated) in &blogs {
            if !seen.insert(bslug.clone()) {
                continue;
            }
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/blog/{b}</loc>{lm}</url>\n",
                base = h(&base),
                d = h(&dslug),
                b = h(bslug),
                lm = lastmod_xml(bupd.or(*bcreated))
            ));
        }
    }

    xml.push_str("</urlset>\n");
    xml_response(StatusCode::OK, xml)
}

pub fn robots(host: Option<&str>, proto: &str, fallback_domain: &str) -> Response<Body> {
    // kanban t_0b3cce4d — ONE sitemap, advertised by the BRAND host only, and the
    // brand host comes from `networks.root_domain` (data). Each hostname used to
    // answer with a `Sitemap:` line naming its OWN 4,030-url copy of the same
    // sitemap, so three sitemaps competed for the same pages; with no brand host set
    // a directory advertises NONE (fail safe). The `/admin/` disallow stays on every
    // host.
    let mut body = String::from(
        "User-agent: *\n\
         Allow: /\n\
         Disallow: /api/\n\
         Disallow: /admin/\n\
         Disallow: /auth/\n",
    );
    if let Some(brand) = crate::seo_canonical::sitemap_origin(host) {
        body.push_str(&format!("\nSitemap: {}/sitemap.xml\n", brand));
    }
    // `proto`/`fallback_domain` are kept in the signature: every caller shares it
    // and a directory with its own mapped domain still needs an origin here.
    let _ = (proto, fallback_domain);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Programmatic pages — GET /<dir>/<page-slug>   (kanban t_543d51d8)
// ─────────────────────────────────────────────────────────────────────────────
//
// The generators (trap doors / content queue / programmatic generator) publish
// thousands of rows into `programmatic_pages` and the SEO sitemap advertised
// every one of them, but nothing rendered them: `/<dir>/<slug>` was 301'd to a
// non-existent business and the directory-host form fell through to the SPA
// shell — a soft-404 for every advertised URL. `POST /programmatic-pages/:id/track`
// had no caller for the same reason. Both are fixed here.

/// Result of resolving a public path against `programmatic_pages`.
enum PageRender {
    /// A published row exists and was rendered.
    Rendered(Response<Body>),
    /// A row exists for this slug but is not published — answer a real 404,
    /// never the SPA shell.
    Unpublished,
    /// No row with this slug in this directory.
    Absent,
}

/// Resolve the directory a request's own host names, when the host is a
/// directory subdomain (`<dir-slug>.<base_domain>`). Directories are normally
/// addressed as subfolders (`/<dir-slug>/...`), so this returns `None` for the
/// platform host and costs no query there.
async fn directory_from_host(
    pool: &PgPool,
    host: Option<&str>,
    base_domain: &str,
) -> Option<DirectoryRec> {
    let base = base_domain.trim().trim_matches('.');
    if base.is_empty() {
        return None;
    }
    let raw = host?.trim();
    let label = raw
        .split(':')
        .next()
        .unwrap_or(raw)
        .trim_start_matches("www.");
    let label = label.strip_suffix(&format!(".{}", base))?;
    if label.is_empty() || label.contains('.') {
        return None;
    }
    load_directory(pool, label).await
}

/// Server-render a stored programmatic page. The row's own `h1` / `meta_title` /
/// `meta_description` / `content` are the page — nothing is invented here — and
/// the rendered document carries the impression beacon the `/track` endpoint was
/// written for.
async fn render_programmatic_page(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    dir: &DirectoryRec,
    page_slug: &str,
) -> PageRender {
    // One bounded, indexed lookup: `programmatic_pages_directory_id_slug_key`.
    let row = sqlx::query(
        "SELECT pp.id, pp.slug, pp.title, pp.meta_title, pp.meta_description, pp.h1, \
                pp.content, pp.status, pp.updated_at, \
                ds.name AS service_name, dl.name AS location_name \
         FROM programmatic_pages pp \
         LEFT JOIN directory_services ds ON ds.id = pp.service_id \
         LEFT JOIN directory_locations dl ON dl.id = pp.location_id \
         WHERE pp.directory_id = $1 AND pp.slug = $2 LIMIT 1",
    )
    .bind(dir.id)
    .bind(page_slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let row = match row {
        Some(r) => r,
        None => return PageRender::Absent,
    };

    let status: String = row.try_get("status").unwrap_or_default();
    if !status.eq_ignore_ascii_case("published") {
        return PageRender::Unpublished;
    }
    let page_id: Uuid = row.try_get("id").unwrap_or_default();

    let p_slug: String = row.try_get("slug").unwrap_or_default();
    let meta_title: Option<String> = row.try_get("meta_title").unwrap_or(None);
    let meta_description: Option<String> = row.try_get("meta_description").unwrap_or(None);
    let h1_raw: Option<String> = row.try_get("h1").unwrap_or(None);
    let title_raw: Option<String> = row.try_get("title").unwrap_or(None);
    let content: String = row.try_get("content").unwrap_or(None).unwrap_or_default();
    let service_name: Option<String> = row.try_get("service_name").unwrap_or(None);
    let location_name: Option<String> = row.try_get("location_name").unwrap_or(None);
    let updated: Option<chrono::DateTime<chrono::Utc>> = row.try_get("updated_at").unwrap_or(None);

    let base = origin(host, proto, fallback_domain);
    let site = brand_name_for(pool, dir).await;
    let hq = site_headquarters(pool).await;
    let theme = theme_for(pool, dir).await;
    let canonical = format!("{}/{}/{}", base, dir.slug, p_slug);

    let h1 = h1_raw
        .filter(|s| !s.trim().is_empty())
        .or_else(|| title_raw.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| format!("{} in {}, {}", dir.name, dir.city, dir.state));

    let title = meta_title
        .filter(|s| !s.trim().is_empty())
        .or_else(|| title_raw.clone().filter(|s| !s.trim().is_empty()))
        .map(|t| {
            if t.contains(&site) {
                t
            } else {
                format!("{} | {}", t, site)
            }
        })
        .unwrap_or_else(|| format!("{} | {}", h1, site));

    let description = meta_description
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| strip_tags(&content));
    let description = clip(&description, 158);

    let mut jsonld = Vec::new();
    jsonld.push(serde_json::json!({
        "@context": "https://schema.org",
        "@type": "WebPage",
        "name": h1,
        "description": description,
        "url": canonical,
        "dateModified": updated
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        "isPartOf": {"@type": "WebSite", "name": site, "url": format!("{}/", base)},
        "about": {
            "@type": "City",
            "name": dir.city,
            "containedInPlace": {"@type": "AdministrativeArea", "name": dir.state},
        },
        "publisher": {"@type": "Organization", "name": site},
    }));
    jsonld.push(breadcrumb_ld(&[
        (&site, &format!("{}/", base)),
        (&dir.name, &format!("{}/{}/", base, dir.slug)),
        (&h1, &canonical),
    ]));

    let seo = Seo {
        title,
        description: description.clone(),
        canonical: canonical.clone(),
        og_type: "article".into(),
        og_image: None,
        jsonld,
    };

    let lede = match (&service_name, &location_name) {
        (Some(s), Some(l)) => format!(
            "<p class=\"lede\">{} in {}, {} — serving {}.</p>",
            h(s),
            h(l),
            h(&dir.state),
            h(&dir.city)
        ),
        _ => format!("<p class=\"lede\">{}</p>", h(&description)),
    };

    // The beacon `POST /programmatic-pages/:page_id/track` was written for.
    // The endpoint is auth-exempt for POST only (see `auth_guard`).
    let beacon = format!(
        "<script>(function(){{try{{fetch(\"/api/v1/programmatic-pages/{}/track\",\
{{method:\"POST\",headers:{{\"Content-Type\":\"application/json\"}},\
body:JSON.stringify({{event:\"impression\"}}),keepalive:true}}).catch(function(){{}});}}\
catch(e){{}}}})();</script>",
        page_id
    );

    let crumbs_owned = [
        (site.as_str(), format!("{}/", base)),
        (dir.name.as_str(), format!("{}/{}/", base, dir.slug)),
        (h1.as_str(), canonical.clone()),
    ];
    let crumbs_refs: Vec<(&str, &str)> =
        crumbs_owned.iter().map(|(a, b)| (*a, b.as_str())).collect();

    let body = format!(
        r#"{start}{crumbs}
<article class="detail article-body">
<h1>{h1}</h1>
{lede}
{content}
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}/businesses">Browse {city} businesses</a> &middot; <a href="{base}/{dslug}">More about {city}</a></p>
</article>
{beacon}
{end}"#,
        start = shell_start(
            &seo,
            &site,
            Some((dir.slug.as_str(), dir.name.as_str())),
            &theme
        ),
        crumbs = breadcrumbs(&crumbs_refs),
        h1 = h(&h1),
        lede = lede,
        content = crate::template_engine::sanitize_html(&content),
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        beacon = beacon,
        end = shell_end(&site, hq.as_deref()),
    );

    PageRender::Rendered(html_response(StatusCode::OK, body))
}

/// A readable 404 — never the SPA shell, so Google is not handed a 200 for a
/// URL that does not exist. `no-store` so an edge cache cannot pin a 404 for a
/// slug that is published a moment later.
/// The ONE 404 body — `noindex,follow`, no-store, so a junk url can never be
/// indexed even if something links it. Shared with the SPA fallback in `routes`,
/// which now answers 404 instead of the 200 shell for a path that matches no route
/// (kanban t_f97664fe).
pub fn not_found_page(msg: &str) -> Response<Body> {
    let body = format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"robots\" content=\"noindex,follow\"><title>Page not found</title>\
<link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.svg\"></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;max-width:640px;margin:80px auto;padding:0 24px;line-height:1.6\">\
<style>.zh-nf-link{{display:inline-block;padding:6px 12px;border:1px solid #cbd5e1;border-radius:999px;color:#0f172a;text-decoration:none}}.zh-nf-link:hover{{border-color:#f27f2f;color:#f27f2f}}</style>\
<nav class=\"zh-crumbs\" aria-label=\"Breadcrumb\" style=\"font-size:13px;color:#6b7280\"><a href=\"/\" style=\"color:#f27f2f;text-decoration:none\">ZaarHub</a> <span style=\"opacity:.5;margin:0 6px\">&#8250;</span> <span aria-current=\"page\">Page not found</span></nav><h1 style=\"font-size:1.6rem\">Page not found</h1><p>{}</p>\
<ul style=\"list-style:none;padding:0;display:flex;flex-wrap:wrap;gap:8px 10px;margin-top:20px\"><li><a href=\"/\" class=\"zh-nf-link\">Go to the homepage</a></li><li><a href=\"/zaarhub\" class=\"zh-nf-link\">Browse all cities</a></li><li><a href=\"/guides/index.html\" class=\"zh-nf-link\">Read the guides</a></li><li><a href=\"/claim.html\" class=\"zh-nf-link\">Claim your business</a></li></ul></body></html>",
        h(msg)
    );
    let mut r = html_response(StatusCode::NOT_FOUND, body);
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatcher — called from the SPA fallback for unmatched GET paths.
// ─────────────────────────────────────────────────────────────────────────────

/// True when the last path segment looks like a static asset (has an extension),
/// which must never be treated as a directory subfolder.
fn looks_like_asset(path: &str) -> bool {
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && ext.len() <= 6 && !ext.contains('/'),
        None => false,
    }
}

/// Attempt to render a subfolder SEO page for `path`. Returns `None` when the
/// path is not a directory subfolder, so the caller can fall through to the SPA.
pub async fn try_render(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    path: &str,
    spa_html: &str,
) -> Option<Response<Body>> {
    if !(path == "/" || path.is_empty()) {
        // Only GET-ish, no query noise, no assets.
        let clean = path.trim_start_matches('/').trim_end_matches('/');
        if clean.is_empty() || looks_like_asset(clean) {
            return None;
        }
        let segs: Vec<&str> = clean.split('/').collect();

        // Admin/API/known namespaces are handled elsewhere — never claim them.
        if matches!(
            segs[0],
            "api" | "admin" | "auth" | "public" | "assets" | "static" | "d" | "zaarhub" | "legal"
        ) {
            return None;
        }

        let slug = segs[0];
        // A reserved top-level name belongs to a real route — do not shadow it.
        if slug_is_reserved(slug).is_some() {
            return None;
        }

        let resp = match segs.as_slice() {
            [_, "businesses"] => {
                businesses_page(pool, host, proto, fallback_domain, slug, "").await
            }
            [_, "businesses", _] => {
                // `/<slug>/businesses/<ident>` — reject deeper nesting.
                business_detail(pool, host, proto, fallback_domain, slug, segs[2]).await
            }
            [_, "articles", a] => article_page(pool, host, proto, fallback_domain, slug, a).await,
            [_, "blog", b] => blog_post_page(pool, host, proto, fallback_domain, slug, b).await,
            // A single segment is normally a directory home. When no such
            // directory exists and the request host IS a directory
            // (`<dir>.<base_domain>`), the segment addresses a programmatic
            // page — render it, or answer a real 404. Never the SPA shell: that
            // shell is what made thousands of advertised URLs soft-404s.
            [_] => {
                if let Some(resp) =
                    directory_home(pool, host, proto, fallback_domain, slug, spa_html).await
                {
                    Some(resp)
                } else if let Some(dir) = directory_from_host(pool, host, fallback_domain).await {
                    match render_programmatic_page(pool, host, proto, fallback_domain, &dir, slug)
                        .await
                    {
                        PageRender::Rendered(r) => Some(r),
                        PageRender::Unpublished => {
                            Some(not_found_page("That page is not published yet."))
                        }
                        PageRender::Absent => {
                            Some(not_found_page(&format!("There is no page at /{}.", slug)))
                        }
                    }
                } else {
                    None
                }
            }
            // `/dir/blog` and `/dir/deals` are server-rendered indexes (no JS
            // required) so a crawler and a script-free visitor get the city's
            // real posts and deals. `/dir/articles` has no index page yet.
            [_, "blog"] => blog_list_page(pool, host, proto, fallback_domain, slug).await,
            [_, "deals"] => deals_page(pool, host, proto, fallback_domain, slug).await,
            [_, "articles"] => None,
            // `/dir/<page-or-business-slug>`. A published programmatic page
            // (the form the SEO sitemap publishes) renders here; otherwise the
            // legacy business short form still 301s to `/dir/businesses/<slug>`,
            // and an unpublished page is a real 404 rather than a soft one.
            [_, short] => {
                if let Some(dir) = load_directory(pool, slug).await {
                    // A published programmatic page (the form the sitemap
                    // publishes) renders. Otherwise: a URL that names a real
                    // business keeps the legacy 301 to its canonical form, and
                    // anything else is a genuine 404 — never the SPA shell,
                    // which is the soft-404 Google was being handed.
                    let outcome =
                        render_programmatic_page(pool, host, proto, fallback_domain, &dir, short)
                            .await;
                    match outcome {
                        PageRender::Rendered(r) => Some(r),
                        PageRender::Absent | PageRender::Unpublished => {
                            let is_business: i64 = sqlx::query_scalar(
                                "SELECT COUNT(*) FROM businesses \
                                 WHERE directory_id = $1 AND slug = $2",
                            )
                            .bind(dir.id)
                            .bind(short)
                            .fetch_one(pool)
                            .await
                            .unwrap_or(0);
                            if is_business > 0 {
                                Some(redirect_301(&format!(
                                    "{}/{}/businesses/{}",
                                    origin(host, proto, fallback_domain),
                                    slug,
                                    short
                                )))
                            } else if let Some(page) =
                                category_page(pool, host, proto, fallback_domain, slug, short, "")
                                    .await
                            {
                                // `/<city>/<category-slug>` — its own crawlable page.
                                Some(page)
                            } else if let Some(page) =
                                town_page(pool, host, proto, fallback_domain, slug, short).await
                            {
                                // `/<city>/<town-slug>` — a satellite town with enough
                                // listings for its own page (W26). Checked AFTER a category
                                // so an existing category URL never changes meaning.
                                Some(page)
                            } else {
                                Some(not_found_page(&format!(
                                    "There is no page at /{}/{}.",
                                    slug, short
                                )))
                            }
                        }
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        // Every server-rendered page of this directory gets the admin's head/body/footer code.
        return apply_code_injection(pool, slug, resp).await;
    }
    None
}

/// ASCII-case-insensitive `find` that returns a byte offset valid for the ORIGINAL string (so a
/// later `insert_str` lands correctly — `str::to_lowercase` can change byte length on non-ASCII).
fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() || hb.len() < nb.len() {
        return None;
    }
    let start = from.min(hb.len().saturating_sub(nb.len()));
    (start..=hb.len() - nb.len()).find(|&i| hb[i..i + nb.len()].eq_ignore_ascii_case(nb))
}

/// ASCII-case-insensitive `rfind`, byte offsets valid for the original string.
fn rfind_ci(hay: &str, needle: &str) -> Option<usize> {
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() || hb.len() < nb.len() {
        return None;
    }
    (0..=hb.len() - nb.len())
        .rev()
        .find(|&i| hb[i..i + nb.len()].eq_ignore_ascii_case(nb))
}

/// Byte offset just past the `<body …>` opening tag, or `None` when there is no body tag.
fn body_open_end(hay: &str) -> Option<usize> {
    let start = find_ci(hay, "<body", 0)?;
    let gt = hay[start..].find('>')?;
    Some(start + gt + 1)
}

/// Insert the directory's admin-authored head/body/footer code into a rendered page.
///
/// The portal's SEO screen offers "Custom Header HTML (head)", "Custom Body HTML (after <body>)"
/// and "Custom Footer HTML (before </body>)" per directory (saved via PUT /directories/:slug) and
/// promises the code lands on that directory's pages. The fields were stored and editable, but no
/// server-rendered page ever emitted them — only the rarely-reached `/d/<slug>/blog` renderer read
/// them — so the feature was dead. Applied here, in the shared SSR dispatcher, it reaches every page
/// of the directory (home, businesses list/detail, blog list/post, articles, deals, programmatic).
///
/// The code is inserted VERBATIM (no sanitisation): this feature IS code injection by design — the
/// UI's own placeholder is a `<script type="application/ld+json">` — and only an authenticated admin
/// of the directory can set these fields. When nothing is configured the body is not even buffered.
async fn apply_code_injection(
    pool: &PgPool,
    slug: &str,
    resp: Option<Response<Body>>,
) -> Option<Response<Body>> {
    let resp = resp?;

    let is_html = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);
    if !is_html {
        return Some(resp);
    }

    let row = sqlx::query(
        "SELECT head_injection, body_injection, footer_injection FROM directories WHERE slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let head = row
        .try_get::<Option<String>, _>("head_injection")
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty());
    let body = row
        .try_get::<Option<String>, _>("body_injection")
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty());
    let footer = row
        .try_get::<Option<String>, _>("footer_injection")
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty());
    if head.is_none() && body.is_none() && footer.is_none() {
        return Some(resp); // nothing configured — do not even buffer the body
    }

    let (mut parts, stream) = resp.into_parts();
    let bytes = match to_bytes(stream, 4 * 1024 * 1024).await {
        Ok(b) => b,
        // Could not buffer — hand the page back rather than break it.
        Err(_) => return None,
    };
    let mut html = String::from_utf8_lossy(&bytes).to_string();

    inject_code_into_html(
        &mut html,
        head.as_deref(),
        body.as_deref(),
        footer.as_deref(),
    );

    parts.headers.remove(header::CONTENT_LENGTH);
    Some(Response::from_parts(parts, Body::from(html)))
}

/// Insert a directory's admin-authored head/body/footer code into an HTML document.
///
/// Shared by the SSR dispatcher above and the SPA shell: a visitor loading `/city/<slug>`
/// gets the cached `index.html` shell, not the server-rendered page, so without this the
/// owner's analytics/verification tags would only ever run for crawlers.
pub fn inject_code_into_html(
    html: &mut String,
    head: Option<&str>,
    body: Option<&str>,
    footer: Option<&str>,
) {
    if let Some(h) = head.filter(|v| !v.trim().is_empty()) {
        if let Some(pos) = find_ci(html, "</head>", 0) {
            html.insert_str(pos, &format!("\n{h}\n"));
        }
    }
    if let Some(b) = body.filter(|v| !v.trim().is_empty()) {
        if let Some(pos) = body_open_end(html) {
            html.insert_str(pos, &format!("\n{b}\n"));
        }
    }
    if let Some(f) = footer.filter(|v| !v.trim().is_empty()) {
        if let Some(pos) = rfind_ci(html, "</body>") {
            html.insert_str(pos, &format!("\n{f}\n"));
        }
    }
}

/// Same dispatcher but with the raw query string (so `/<slug>/businesses?page=2`
/// works). `try_render` keeps the no-query signature for the fallback hook.
pub async fn try_render_with_query(
    pool: &PgPool,
    host: Option<&str>,
    proto: &str,
    fallback_domain: &str,
    path: &str,
    query: &str,
    spa_html: &str,
) -> Option<Response<Body>> {
    let clean = path.trim_start_matches('/').trim_end_matches('/');
    if clean.is_empty() || looks_like_asset(clean) {
        return None;
    }
    let segs: Vec<&str> = clean.split('/').collect();
    if matches!(
        segs[0],
        "api" | "admin" | "auth" | "public" | "assets" | "static" | "d" | "zaarhub" | "legal"
    ) {
        return None;
    }
    if slug_is_reserved(segs[0]).is_some() {
        return None;
    }
    match segs.as_slice() {
        [_, "businesses"] => {
            let resp = businesses_page(pool, host, proto, fallback_domain, segs[0], query).await;
            // The `_` arm delegates to try_render, which applies the injection itself; this arm
            // renders directly, so the code injection is applied here.
            apply_code_injection(pool, segs[0], resp).await
        }
        _ => try_render(pool, host, proto, fallback_domain, path, spa_html).await,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin report — which directory slugs cannot be reached at /<slug>.
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/seo/subfolder-clashes — report directories whose slug shadows a
/// top-level route (so `/<slug>` cannot serve the directory home).
pub async fn clashes_handler(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> axum::Json<serde_json::Value> {
    let clashes = find_clashes(&state.db).await;
    let list: Vec<serde_json::Value> = clashes
        .iter()
        .map(|(slug, against)| {
            serde_json::json!({
                "slug": slug,
                "shadowed_by": against,
                "url": format!("/{}", slug),
                "fix": format!("rename the directory slug (currently /{} is owned by the '{}' route) or use /d/{}", slug, against, slug),
            })
        })
        .collect();
    axum::Json(serde_json::json!({
        "count": list.len(),
        "clashes": list,
        "reserved": RESERVED_TOP_LEVEL,
    }))
}

/// Extract the effective host + scheme from request headers (nginx sets
/// `X-Forwarded-Proto`; `Host` carries the public domain).
pub fn host_proto(headers: &axum::http::HeaderMap) -> (Option<String>, String) {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https")
        .to_string();
    (host, proto)
}

/// axum handler for `GET /sitemap.xml`.
pub async fn sitemap_handler(
    axum::extract::State(s): axum::extract::State<crate::AppState>,
    headers: axum::http::HeaderMap,
) -> Response<Body> {
    let (host, proto) = host_proto(&headers);
    sitemap(&s.db, host.as_deref(), &proto, &s.config.base_domain).await
}

/// axum handler for `GET /robots.txt`.
pub async fn robots_handler(
    axum::extract::State(s): axum::extract::State<crate::AppState>,
    headers: axum::http::HeaderMap,
) -> Response<Body> {
    let (host, proto) = host_proto(&headers);
    robots(host.as_deref(), &proto, &s.config.base_domain)
}

// ─────────────────────────────────────────────────────────────────────────────
// Small utils
// ─────────────────────────────────────────────────────────────────────────────

fn parse_query(q: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        m.insert(urldecode(k), urldecode(v));
    }
    m
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
