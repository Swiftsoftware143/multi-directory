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

use crate::brand_theme::BrandTheme;
use axum::body::Body;
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
fn origin(host: Option<&str>, proto: &str, fallback: &str) -> String {
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
    "admin-seo.js",
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

/// Render a city page by taking the **homepage document** (`frontend/index.html`,
/// the shared component library) and giving it the city's own SEO head. The
/// markup, CSS, navigation and component set are therefore byte-identical to the
/// homepage — only the data (loaded by the SPA from the city endpoint) and the
/// head tags are city-scoped. This is what stops the two surfaces drifting.
fn inject_city_document(spa_html: &str, seo: &Seo, site_name: &str, theme: &BrandTheme) -> String {
    let head = head_html(seo, site_name);
    let theme_tag = format!("<style id=\"brand-theme\">{}</style>", theme.css_block());
    // Drop the homepage's own <title> (the city's is the first line of `head`).
    let base = strip_first_between(spa_html, "<title", "</title>");
    match base.rfind("</head>") {
        Some(pos) => {
            let mut out = String::with_capacity(base.len() + head.len() + theme_tag.len());
            out.push_str(&base[..pos]);
            out.push_str(&head);
            out.push_str(&theme_tag);
            out.push_str(&base[pos..]);
            out
        }
        None => base,
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
.btn{background:var(--primary);color:#fff;border-radius:var(--radius);padding:10px 18px;font-weight:600}
.detail{background:var(--card);border:1px solid var(--border);border-radius:var(--radius-lg);padding:28px}
.detail h1{margin-bottom:6px}
.row{margin:8px 0;color:var(--text)}
.article-body h2{font-size:1.25rem;margin:22px 0 10px;color:var(--secondary)}
.article-body p{margin-bottom:14px}
.article-body ul,.article-body ol{margin:0 0 14px 22px}
footer{background:var(--dark);color:rgba(255,255,255,.7);padding:28px 24px;margin-top:48px;font-size:13px;text-align:center}
footer a{color:var(--primary-light)}
"#;

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
        None => "<nav><a href=\"/\">Home</a></nav>".to_string(),
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
{head}<style>{theme_root}
{css}</style>
</head>
<body>
<header><div class="inner"><a class="logo" href="/">{site}</a>{nav}</div></header>
<div class="wrap">"#,
        head = head_html(seo, site_name),
        theme_root = theme.css_block(),
        css = PAGE_CSS,
        site = h(site_name),
        nav = nav,
    )
}

fn shell_end(site_name: &str) -> String {
    format!(
        "</div><footer>© {} {} — local business directory</footer></body></html>",
        chrono::Utc::now().format("%Y"),
        h(site_name)
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
    logo: Option<String>,
    description: Option<String>,
    id: Uuid,
}

async fn list_businesses(
    pool: &PgPool,
    dir: &DirectoryRec,
    category: Option<&str>,
    limit: i64,
    offset: i64,
) -> Vec<BizCard> {
    const COLS: &str = "b.id, b.name, b.slug, b.description, b.logo_url, b.rating, \
                        b.review_count, b.address, b.city, b.state, dc.name AS category";
    let rows = if let Some(cat) = category {
        sqlx::query(&format!(
            "SELECT {COLS} FROM businesses b \
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
        sqlx::query(&format!(
            "SELECT {COLS} FROM businesses b \
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
        let id: Uuid = match r.try_get("id") {
            Ok(v) => v,
            Err(_) => continue,
        };
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
            logo: r.try_get("logo_url").unwrap_or(None),
            description: r.try_get("description").unwrap_or(None),
            id,
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
        "SELECT meta_title, meta_description, hero_image_url FROM city_pages \
         WHERE city_slug = $1 AND is_active = true LIMIT 1",
    )
    .bind(&dir.slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let (city_title, city_desc, hero) = match city_row {
        Some(r) => (
            r.try_get::<Option<String>, _>("meta_title").unwrap_or(None),
            r.try_get::<Option<String>, _>("meta_description")
                .unwrap_or(None),
            r.try_get::<Option<String>, _>("hero_image_url")
                .unwrap_or(None),
        ),
        None => (None, None, None),
    };

    let ov = seo_override(pool, "city", dir.id).await;

    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .or(city_title)
        .unwrap_or_else(|| format!("Local Businesses in {}, {} | {}", dir.city, dir.state, site));

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
        .or(city_desc)
        .or_else(|| dir.description.clone())
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
    // rendered by the shared homepage template from the city API.)
    let top = list_businesses(pool, &dir, None, 12, 0).await;

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
    // components — carrying only the city's own <head>. The SPA scopes the data
    // to this city from `/api/v1/zaarhub/cities/<slug>`, keyed off the path.
    let body = inject_city_document(spa_html, &seo, &site, &theme);

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
    let title = format!("{} | {}", label, site);
    let description = clip(
        &format!(
            "Browse {} of {} local businesses in {}, {}. Ratings, reviews, addresses and contact details.",
            items.len(),
            total,
            dir.city,
            dir.state
        ),
        158,
    );

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

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">{n} businesses · page {page} of {pages}</p>
<div class="grid">{cards}</div>
{pager}
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
        end = shell_end(&site),
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

    let by_uuid = Uuid::parse_str(ident).ok();
    let row = sqlx::query(
        "SELECT id, name, slug, description, category_id, address, city, state, zip, phone, \
                website, logo_url, cover_url, rating, review_count, latitude, longitude, featured \
         FROM businesses WHERE directory_id = $1 AND is_active = true \
           AND (slug = $2 OR ($3::uuid IS NOT NULL AND id = $3::uuid)) LIMIT 1",
    )
    .bind(dir.id)
    .bind(ident)
    .bind(by_uuid)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let biz_id: Uuid = row.try_get("id").ok()?;
    let name: String = row.try_get("name").unwrap_or_default();
    let biz_slug: String = row.try_get("slug").unwrap_or_default();
    let description: String = row
        .try_get::<Option<String>, _>("description")
        .unwrap_or(None)
        .unwrap_or_default();
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
    let category: Option<String> = {
        let cid: Option<Uuid> = row.try_get("category_id").unwrap_or(None);
        if let Some(cid) = cid {
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

    let canonical = format!("{}/{}/businesses/{}", base, dir.slug, biz_slug);
    let ov = seo_override(pool, "business", biz_id).await;

    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .unwrap_or_else(|| match &category {
            Some(c) => format!("{} — {} in {}, {} | {}", name, c, city, state, site),
            None => format!("{} in {}, {} | {}", name, city, state, site),
        });

    let description = ov
        .as_ref()
        .and_then(|o| o.description.clone())
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
        .or_else(|| logo.clone())
        .or_else(|| cover.clone());

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
        jsonld.push(lb);
        jsonld.push(breadcrumb_ld(&[
            (&site, &format!("{}/", base)),
            (&dir.name, &format!("{}/{}/", base, dir.slug)),
            ("Businesses", &format!("{}/{}/businesses", base, dir.slug)),
            (&name, &canonical),
        ]));
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
            "<div class=\"row\"><span class=\"stars\">★ {:.1}</span> · {} reviews</div>",
            rating, reviews
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

    let body = format!(
        r#"{start}{crumbs}
<div class="detail">
{logo}
<h1>{name}</h1>
{rating_html}
{desc_html}
<div class="row">{cat}</div>
{rows}
<p class="row"><a href="{base}/{dslug}/businesses">← All businesses in {city}</a></p>
</div>
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
            (&name, &canonical),
        ]),
        logo = logo_html,
        name = h(&name),
        rating_html = rating_html,
        desc_html = desc_html,
        cat = category
            .as_ref()
            .map(|c| format!("<div class=\"row\">🏷️ {}</div>", h(c)))
            .unwrap_or_default(),
        rows = rows,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&city),
        end = shell_end(&site),
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

    let row = sqlx::query(
        "SELECT id, title, slug, meta_description, content, keyword, business_id, created_at, updated_at \
         FROM business_articles WHERE directory_id = $1 AND slug = $2 \
           AND status IN ('published','draft') LIMIT 1",
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

    let body = format!(
        r#"{start}{crumbs}
<article class="detail article-body">
<h1>{h1}</h1>
<p class="lede">{date}</p>
{biz}
{content}
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← More about {city}</a></p>
</article>
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
            (&title_raw, &canonical),
        ]),
        h1 = h(&title_raw),
        date = h(&date_label),
        biz = biz_html,
        content = safe_content,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        end = shell_end(&site),
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

    let row = sqlx::query(
        "SELECT id, title, slug, excerpt, content, meta_title, meta_description, canonical_url, \
                robots_meta, featured_image_url, author_name, blog_category, created_at, updated_at \
         FROM blog_posts WHERE directory_id = $1 AND slug = $2 \
           AND coalesce(published, true) = true LIMIT 1",
    )
    .bind(dir.id)
    .bind(post_slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

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
            (&title_raw, &canonical),
        ]),
        img = img_html,
        h1 = h(&title_raw),
        date = h(&date_label),
        byline = byline,
        cat = cat_html,
        content = safe_content,
        base = h(&base),
        dslug = h(&dir.slug),
        city = h(&dir.city),
        end = shell_end(&site),
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
    let canonical = format!("{}/{}/blog", base, dir.slug);

    let ov = seo_override(pool, "city_blog", dir.id).await;
    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .unwrap_or_else(|| format!("{} Local Blog — News & Guides | {}", dir.city, site));

    // A failed read returns None (the caller falls back to the SPA) rather than
    // painting a misleading "no posts yet" page over a query that never ran.
    let rows = sqlx::query(
        "SELECT slug, title, excerpt, scheduled_at, created_at, featured_image_url, \
                author_name, blog_category \
         FROM blog_posts \
         WHERE directory_id = $1 AND published = true AND status = 'published' \
           AND (scheduled_at IS NULL OR scheduled_at <= NOW()) \
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

            Some(BlogItem {
                title: title_raw,
                url: format!("{}/{}/blog/{}", base, dir.slug, post_slug),
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

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">{lede}</p>
<div class="grid">{list}</div>
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← Back to {city}</a></p>
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
        end = shell_end(&site),
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
    let canonical = format!("{}/{}/deals", base, dir.slug);

    let ov = seo_override(pool, "city_deals", dir.id).await;
    let title = ov
        .as_ref()
        .and_then(|o| o.title.clone())
        .unwrap_or_else(|| format!("Deals & Coupons in {}, {} | {}", dir.city, dir.state, site));

    let rows = sqlx::query(
        "SELECT de.title, de.description, de.deal_price, de.original_price, \
                de.discount_percent, de.image_url, de.end_date, de.featured, \
                b.name AS business_name, b.slug AS business_slug \
         FROM deals de JOIN businesses b ON b.id = de.business_id \
         WHERE b.directory_id = $1 AND de.status = 'active' \
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
            let title_raw: String = r.try_get("title").unwrap_or_default();
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
                description: r.try_get("description").unwrap_or(None),
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

    let body = format!(
        r#"{start}{crumbs}
<h1>{label}</h1>
<p class="lede">{lede}</p>
<div class="grid">{list}</div>
<p class="row" style="margin-top:24px"><a href="{base}/{dslug}">← Back to {city}</a></p>
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
        end = shell_end(&site),
    );

    Some(html_response(StatusCode::OK, body))
}

// ─────────────────────────────────────────────────────────────────────────────
// Sitemap + robots
// ─────────────────────────────────────────────────────────────────────────────

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
    xml.push_str(&format!(
        "  <url><loc>{}/</loc><changefreq>daily</changefreq><priority>1.0</priority></url>\n",
        h(&base)
    ));

    let dirs =
        sqlx::query("SELECT id, slug FROM directories WHERE status = 'active' ORDER BY slug")
            .fetch_all(pool)
            .await
            .unwrap_or_default();

    for d in &dirs {
        let dir_id: Uuid = d.try_get("id").unwrap_or_default();
        let dslug: String = d.try_get("slug").unwrap_or_default();
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}</loc><changefreq>daily</changefreq><priority>0.9</priority></url>\n",
            base = h(&base),
            slug = h(&dslug)
        ));
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}/businesses</loc><changefreq>daily</changefreq><priority>0.8</priority></url>\n",
            base = h(&base),
            slug = h(&dslug)
        ));
        // The server-rendered city blog index and deals index — the two
        // sub-paths Google should index per city (subfolder canonical form).
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}/blog</loc><changefreq>weekly</changefreq><priority>0.7</priority></url>\n",
            base = h(&base),
            slug = h(&dslug)
        ));
        xml.push_str(&format!(
            "  <url><loc>{base}/{slug}/deals</loc><changefreq>weekly</changefreq><priority>0.7</priority></url>\n",
            base = h(&base),
            slug = h(&dslug)
        ));

        // Businesses (subfolder URLs).
        let biz = sqlx::query(
            "SELECT slug FROM businesses WHERE directory_id = $1 AND is_active = true \
             AND coalesce(slug,'') <> '' ORDER BY rating DESC NULLS LAST LIMIT 20000",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for b in &biz {
            let bslug: String = b.try_get("slug").unwrap_or_default();
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/businesses/{b}</loc><changefreq>weekly</changefreq><priority>0.7</priority></url>\n",
                base = h(&base),
                d = h(&dslug),
                b = h(&bslug)
            ));
        }

        // Articles (business_articles — the SEO article system).
        let arts = sqlx::query(
            "SELECT slug FROM business_articles WHERE directory_id = $1 \
             AND coalesce(slug,'') <> '' ORDER BY created_at DESC LIMIT 5000",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for a in &arts {
            let aslug: String = a.try_get("slug").unwrap_or_default();
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/articles/{a}</loc><changefreq>monthly</changefreq><priority>0.6</priority></url>\n",
                base = h(&base),
                d = h(&dslug),
                a = h(&aslug)
            ));
        }

        // Blog posts (blog_posts — the CMS article system). Slugs dedup'd: the
        // table has no unique constraint and duplicates would double-list a URL.
        let blogs = sqlx::query(
            "SELECT slug FROM blog_posts WHERE directory_id = $1 \
             AND coalesce(published, true) = true AND coalesce(slug,'') <> '' \
             ORDER BY created_at DESC LIMIT 5000",
        )
        .bind(dir_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        for b in &blogs {
            let bslug: String = b.try_get("slug").unwrap_or_default();
            if !seen.insert(bslug.clone()) {
                continue;
            }
            xml.push_str(&format!(
                "  <url><loc>{base}/{d}/blog/{b}</loc><changefreq>monthly</changefreq><priority>0.6</priority></url>\n",
                base = h(&base),
                d = h(&dslug),
                b = h(&bslug)
            ));
        }
    }

    xml.push_str("</urlset>\n");
    xml_response(StatusCode::OK, xml)
}

pub fn robots(host: Option<&str>, proto: &str, fallback_domain: &str) -> Response<Body> {
    let base = origin(host, proto, fallback_domain);
    let body = format!(
        "User-agent: *\n\
         Allow: /\n\
         Disallow: /api/\n\
         Disallow: /admin/\n\
         Disallow: /auth/\n\
         \n\
         Sitemap: {}/sitemap.xml\n",
        base
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
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

        return match segs.as_slice() {
            [_, "businesses"] => {
                businesses_page(pool, host, proto, fallback_domain, slug, "").await
            }
            [_, "businesses", _] => {
                // `/<slug>/businesses/<ident>` — reject deeper nesting.
                business_detail(pool, host, proto, fallback_domain, slug, segs[2]).await
            }
            [_, "articles", a] => article_page(pool, host, proto, fallback_domain, slug, a).await,
            [_, "blog", b] => blog_post_page(pool, host, proto, fallback_domain, slug, b).await,
            [_] => directory_home(pool, host, proto, fallback_domain, slug, spa_html).await,
            // `/dir/blog` and `/dir/deals` are server-rendered indexes (no JS
            // required) so a crawler and a script-free visitor get the city's
            // real posts and deals. `/dir/articles` has no index page yet.
            [_, "blog"] => blog_list_page(pool, host, proto, fallback_domain, slug).await,
            [_, "deals"] => deals_page(pool, host, proto, fallback_domain, slug).await,
            [_, "articles"] => None,
            // `/dir/<business-slug>` (the short form) — 301 to the canonical
            // `/dir/businesses/<slug>` so a legacy/inferred URL is not a soft-404.
            // Guarded on a real directory: a path like `/city/palm-bay` must fall
            // through to the SPA untouched, not be redirected to a bogus URL.
            [_, short] => {
                if load_directory(pool, slug).await.is_some() {
                    Some(redirect_301(&format!(
                        "{}/{}/businesses/{}",
                        origin(host, proto, fallback_domain),
                        slug,
                        short
                    )))
                } else {
                    None
                }
            }
            _ => None,
        };
    }
    None
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
            businesses_page(pool, host, proto, fallback_domain, segs[0], query).await
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
