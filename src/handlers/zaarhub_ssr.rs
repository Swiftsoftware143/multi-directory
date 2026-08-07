/// SSR handlers for ZaarHub city landing pages
use axum::extract::{Path, State};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::AppState;

/// Simple HTML escaper
fn h(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Build footer HTML from DB — loads site config + legal pages marked show_in_footer
async fn footer_html(pool: &sqlx::PgPool) -> String {
    let (site_name, copyright_year): (String, String) = sqlx::query_as(
        "SELECT COALESCE(site_name, 'ZaarHub'), COALESCE(copyright_year, '2026') FROM zaarhub_site_config LIMIT 1"
    ).fetch_optional(pool).await.unwrap_or(None).unwrap_or(("ZaarHub".into(), "2026".into()));

    let rows = sqlx::query(
        "SELECT slug, title FROM zaarhub_legal_pages WHERE is_published = true AND show_in_footer = true ORDER BY display_order ASC, title ASC"
    ).fetch_all(pool).await.unwrap_or_default();

    let mut links = String::from("<a href=\"/zaarhub\">Cities</a>");
    for r in &rows {
        let slug: String = r.try_get("slug").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        links.push_str(&format!("<a href=\"/legal/{}\">{}</a>", h(&slug), h(&title)));
    }

    format!(
        "<footer><div class=\"footer-links\">{links}</div><p>&copy; {year} {name}. All rights reserved.</p></footer>",
        year = h(&copyright_year),
        name = h(&site_name),
    )
}

/// Cookie consent banner HTML — shown at page bottom until accepted
const COOKIE_BANNER: &str = r#"<div id="cookie-banner" style="position:fixed;bottom:0;left:0;right:0;background:#1a1a2e;color:white;padding:20px 24px;z-index:9999;display:flex;align-items:center;justify-content:center;gap:20px;flex-wrap:wrap;box-shadow:0 -4px 20px rgba(0,0,0,.3);font-size:13px;line-height:1.5">
<div style="max-width:900px">We use cookies for essential site functionality and analytics to improve your experience. By continuing to use ZaarHub, you accept our <a href="/legal/privacy" style="color:#f27f2f">Privacy Policy</a> and <a href="/legal/terms" style="color:#f27f2f">Terms of Service</a>.</div>
<div style="display:flex;gap:10px">
<button onclick="document.getElementById('cookie-banner').style.display='none';localStorage.setItem('zaarhub_cookies_accepted','1')" style="background:#f27f2f;color:white;border:none;padding:10px 24px;border-radius:8px;font-weight:700;font-size:13px;cursor:pointer;white-space:nowrap">Accept All</button>
<button onclick="document.getElementById('cookie-banner').style.display='none';localStorage.setItem('zaarhub_cookies_accepted','essential')" style="background:transparent;color:white;border:1px solid rgba(255,255,255,.3);padding:10px 24px;border-radius:8px;font-weight:600;font-size:13px;cursor:pointer;white-space:nowrap">Essential Only</button>
</div>
</div>
<script>if(localStorage.getItem('zaarhub_cookies_accepted'))document.getElementById('cookie-banner').style.display='none'</script>"#;

/// Render a full city landing page (SEO-optimized SSR HTML)
pub async fn render_city_page(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let city_row = sqlx::query(
        "SELECT id, city_slug, city_name, state, description, hero_image_url, meta_title, meta_description \
         FROM city_pages WHERE city_slug = $1 AND is_active = true",
    )
    .bind(&slug)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if city_row.is_none() {
        return axum::response::Html(
            r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0"><title>404 — City Not Found | ZaarHub</title><style>body{font-family:system-ui,sans-serif;background:#f8f9fc;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}h1{font-size:48px;color:#2b3255}p{color:#6b7280}a{color:#f27f2f}</style></head><body><div style="text-align:center"><h1>404</h1><p>This city page doesn't exist yet.</p><a href="/zaarhub">Browse all cities →</a></div></body></html>"#
                .to_string(),
        );
    }

    let r = city_row.unwrap();
    let city_name: String = r.try_get("city_name").unwrap_or_default();
    let hero_img: Option<String> = r.try_get("hero_image_url").unwrap_or(None);
    let meta_title: Option<String> = r.try_get("meta_title").unwrap_or(None);
    let meta_desc: Option<String> = r.try_get("meta_description").unwrap_or(None);
    let city_page_id: Uuid = r.try_get("id").unwrap_or_default();

    let page_title = meta_title.unwrap_or_else(|| format!("Best Businesses in {} | ZaarHub", city_name));
    let page_desc = meta_desc.unwrap_or_else(|| format!("Find top-rated local businesses in {}. Browse reviews, deals, and more.", city_name));

    // Editor's Picks for this city
    let city_picks = sqlx::query(
        "SELECT bl.id, bl.business_name, bl.logo_url, bl.rating, bl.review_count, bl.category, bl.editors_pick_note \
         FROM business_listings bl \
         WHERE bl.city_page_id = $1 AND bl.is_editors_pick = true \
         ORDER BY bl.rating DESC NULLS LAST LIMIT 4"
    ).bind(city_page_id).fetch_all(&state.db).await.unwrap_or_default();

    let mut city_picks_html = String::new();
    for p in &city_picks {
        let pname: String = p.try_get("business_name").unwrap_or_default();
        let plogo: Option<String> = p.try_get("logo_url").unwrap_or(None);
        let prating: Option<f64> = p.try_get("rating").unwrap_or_default();
        let previews: i32 = p.try_get("review_count").unwrap_or(0);
        let pcat: Option<String> = p.try_get("category").unwrap_or(None);
        let pnote: Option<String> = p.try_get("editors_pick_note").unwrap_or(None);
        let pid: Uuid = p.try_get("id").unwrap_or_default();
        let pr = prating.unwrap_or(0.0);

        let plogo_html = match &plogo {
            Some(img) if !img.is_empty() => format!("<img src=\"{}\" class=\"pick-logo\" alt=\"{}\" loading=\"lazy\" onerror=\"this.style.display='none'\">", h(img), h(&pname)),
            _ => format!("<div class='pick-logo-placeholder'>{}</div>", h(&pname[..1.min(pname.len())])),
        };

        city_picks_html.push_str(&format!(
            r#"<a href="/zaarhub/{slug}/{id}" class="pick-card">
       {logo}
       <div class="pick-info">
         <h4>{name}</h4>
         <span class="pick-cat">{cat}</span>
         <div class="pick-stars">{stars} {rating:.1} · {reviews} reviews</div>
         {note_html}
       </div>
     </a>"#,
            slug = h(&slug),
            id = pid,
            logo = plogo_html,
            name = h(&pname),
            cat = h(&pcat.unwrap_or_default()),
            stars = String::from("\u{2605}".repeat(pr as usize)) + &"\u{2606}".repeat(5usize.saturating_sub(pr as usize)),
            rating = pr,
            reviews = previews,
            note_html = pnote.as_ref().map(|n| format!("<p class=\"pick-note\">\u{1f525} {}</p>", h(n))).unwrap_or_default(),
        ));
    }

    let city_picks_section = if city_picks_html.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"editors-picks-section\" style=\"max-width:1200px;margin:0 auto 24px;padding:0 20px\"><h2>\u{1f525} Editor's Picks in {}</h2><div class=\"picks-grid\">{}</div></div>",
            h(&city_name), city_picks_html
        )
    };

    // Load top featured listings for this city
    let rows = sqlx::query(
        "SELECT bl.* FROM business_listings bl \
         WHERE bl.city_page_id = $1 AND bl.is_featured = true \
         ORDER BY bl.rating DESC NULLS LAST LIMIT 24",
    )
    .bind(city_page_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut listings_html = String::new();
    for l in &rows {
        let name: String = l.try_get("business_name").unwrap_or_default();
        let cat: Option<String> = l.try_get("category").unwrap_or(None);
        let desc: Option<String> = l.try_get("description").unwrap_or(None);
        let addr: Option<String> = l.try_get("address").unwrap_or(None);
        let logo: Option<String> = l.try_get("logo_url").unwrap_or(None);
        let rating: Option<f64> = l.try_get("rating").unwrap_or_default();
        let reviews: i32 = l.try_get("review_count").unwrap_or(0);
        let lid: Uuid = l.try_get("id").unwrap_or_default();

        // Verified indicator — check via business name match
        let is_verified = sqlx::query(
            "SELECT 1 FROM business_verifications bv \
             JOIN businesses b ON bv.business_id = b.id \
             WHERE b.name = $1 AND bv.status = 'approved' AND (bv.expires_at IS NULL OR bv.expires_at > now()) \
             LIMIT 1"
        ).bind(&name).fetch_optional(&state.db).await.unwrap_or(None).is_some();
        let verified_icon = if is_verified { "<span class=\"verified-icon\" title=\"Verified Business\">\u{2714}</span>" } else { "" };
        let r = rating.unwrap_or(0.0);
        let stars = String::from("★".repeat(r as usize)) + &"☆".repeat(5usize.saturating_sub(r as usize));

        let logo_html = match &logo {
            Some(img) if !img.is_empty() => format!(
                "<img src=\"{}\" class=\"logo-img\" alt=\"\" loading=\"lazy\" onerror=\"this.style.display='none';this.nextElementSibling.style.display='flex'\"><div class=\"logo-placeholder\" style=\"display:none\">{}</div>",
                h(img), h(&name[..1.min(name.len())])
            ),
            _ => format!("<div class=\"logo-placeholder\">{}</div>", h(&name[..1.min(name.len())])),
        };

        let cat_html = cat
            .as_ref()
            .map(|c| format!("<span class=\"category-tag\">{}</span>", h(c)))
            .unwrap_or_default();
        let desc_html = desc
            .as_ref()
            .map(|d| format!("<p class=\"desc\">{}</p>", h(d)))
            .unwrap_or_default();
        let addr_html = addr
            .as_ref()
            .map(|a| format!("<span>📍 {}</span>", h(a)))
            .unwrap_or_default();

        listings_html.push_str(&format!(
            r#"<a href="/zaarhub/{slug}/{lid}" class="listing-card">
      {logo_html}
      <div class="info">
        <h3>{name} {verified_icon}</h3>
        {cat_html}
        {desc_html}
        <div class="meta">
          <span class="stars">{stars}</span>
          <span>{rating:.1}</span>
          <span>{reviews} reviews</span>
          {addr_html}
        </div>
      </div>
    </a>
"#,
            slug = h(&slug),
            lid = lid,
            logo_html = logo_html,
            name = h(&name),
            verified_icon = verified_icon,
            cat_html = cat_html,
            desc_html = desc_html,
            stars = stars,
            rating = r,
            reviews = reviews,
            addr_html = addr_html,
        ));
    }

    let hero_section = match &hero_img {
        Some(img) if !img.is_empty() => format!(
            "<div class=\"hero\" style=\"background:linear-gradient(rgba(43,50,85,.85),rgba(43,50,85,.92)),url({}) center/cover\"><h1>Best Businesses in <span>{}</span></h1><p>{}</p></div>",
            h(img), h(&city_name), h(&page_desc)
        ),
        _ => format!(
            "<div class=\"hero\"><h1>Best Businesses in <span>{}</span></h1><p>{}</p></div>",
            h(&city_name), h(&page_desc)
        ),
    };

    // Schema.org JSON-LD for SEO
    let schema = format!(
        r#"{{"@context":"https://schema.org","@type":"LocalBusiness","name":"{}","description":"{}","address":{{"@type":"PostalAddress","addressRegion":"FL"}},"aggregateRating":{{"@type":"AggregateRating","bestRating":"5"}},"url":"https://zaarhub.com/{}"}}"#,
        h(&city_name), h(&page_desc), h(&slug)
    );

    let footer = footer_html(&state.db).await;
    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>{title}</title>
<meta name="description" content="{desc}">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{desc}">
<meta property="og:type" content="website">
<meta property="twitter:card" content="summary_large_image">
<link rel="canonical" href="https://zaarhub.com/{slug}">
<script type="application/ld+json">{schema}</script>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.5}}
header{{background:#2b3255;color:white;padding:16px 20px;position:sticky;top:0;z-index:100}}
header .inner{{max-width:1200px;margin:0 auto;display:flex;justify-content:space-between;align-items:center}}
header .logo{{font-size:22px;font-weight:800;color:white;text-decoration:none}}header .logo span{{color:#f27f2f}}
.hero{{padding:56px 20px;text-align:center;color:white;background:linear-gradient(135deg,#2b3255,#1a1a3e)}}
.hero h1{{font-size:clamp(26px,5vw,38px);margin-bottom:8px}}.hero h1 span{{color:#f27f2f}}.hero p{{opacity:.85;max-width:600px;margin:0 auto}}
.listing-grid{{max-width:1200px;margin:32px auto;padding:0 20px;display:grid;gap:16px}}
.listing-card{{display:flex;gap:16px;align-items:flex-start;padding:20px;background:white;border-radius:14px;box-shadow:0 1px 3px rgba(0,0,0,.06);text-decoration:none;color:inherit;transition:all .2s;border:2px solid transparent}}
.listing-card:hover{{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}}
.logo-img{{width:64px;height:64px;border-radius:14px;object-fit:cover;flex-shrink:0}}
.logo-placeholder{{width:64px;height:64px;border-radius:14px;background:#f27f2f;color:white;display:flex;align-items:center;justify-content:center;font-size:24px;font-weight:700;flex-shrink:0}}
.info{{flex:1;min-width:0}}
.info h3{{font-size:17px;font-weight:700;margin-bottom:2px}}
.category-tag{{display:inline-block;font-size:11px;font-weight:600;text-transform:uppercase;color:#f27f2f;background:#fff7f0;padding:3px 10px;border-radius:6px;margin-bottom:6px;margin-right:6px}}
.desc{{font-size:13px;color:#6b7280;line-height:1.6;margin-bottom:8px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}
.meta{{display:flex;gap:12px;flex-wrap:wrap;font-size:12px;color:#6b7280;align-items:center}}
.verified-icon{{display:inline-block;color:#16a34a;font-size:14px;margin-left:4px;cursor:help}}
.stars{{color:#f59e0b}}
footer{{text-align:center;padding:48px 20px;color:#6b7280;font-size:13px}}footer a{{color:#f27f2f;text-decoration:none}}
.load-more{{text-align:center;margin:24px 0}}
.load-more a{{display:inline-block;padding:14px 32px;background:#2b3255;color:white;border-radius:100px;text-decoration:none;font-weight:600;font-size:14px;transition:all .2s}}
.load-more a:hover{{background:#f27f2f;transform:translateY(-1px)}}
.editors-picks-section h2{{font-size:20px;font-weight:800;color:#2b3255;margin-bottom:12px}}
.picks-grid{{display:grid;gap:16px;grid-template-columns:repeat(auto-fill,minmax(280px,1fr))}}
.pick-card{{display:flex;gap:14px;padding:18px;background:white;border-radius:14px;box-shadow:0 1px 3px rgba(0,0,0,.06);text-decoration:none;color:inherit;transition:all .2s;border:2px solid transparent}}
.pick-card:hover{{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}}
.pick-logo{{width:56px;height:56px;border-radius:12px;object-fit:cover;flex-shrink:0;background:#f27f2f}}
.pick-logo-placeholder{{width:56px;height:56px;border-radius:12px;background:#f27f2f;color:white;display:flex;align-items:center;justify-content:center;font-size:22px;font-weight:700;flex-shrink:0}}
.pick-info{{flex:1;min-width:0}}
.pick-info h4{{font-size:15px;font-weight:700;margin-bottom:2px}}
.pick-cat{{display:inline-block;font-size:10px;font-weight:600;text-transform:uppercase;color:#f27f2f;background:#fff7f0;padding:2px 8px;border-radius:4px;margin-bottom:4px}}
.pick-stars{{font-size:12px;color:#6b7280;margin-bottom:2px}}
.pick-note{{font-size:12px;color:#f27f2f;font-weight:600;margin-top:4px}}
@media(max-width:600px){{.picks-grid{{grid-template-columns:1fr}}}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">Zaar<span>Hub</span></a><nav><a href="/zaarhub-city.html" style="color:rgba(255,255,255,.8);text-decoration:none;font-size:14px;font-weight:500">🔍 Search</a></nav></div></header>
{hero}
{city_picks}
<div class="listing-grid">{listings}</div>
<div class="load-more"><a href="/zaarhub-city.html?city={slug}">View all {city_name} businesses →</a></div>
{footer}
{cookie_banner}</body>
</html>"#,
        title = h(&page_title),
        desc = h(&page_desc),
        slug = h(&slug),
        city_name = h(&city_name),
        hero = hero_section,
        city_picks = city_picks_section,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
        listings = listings_html,
        schema = schema,
    ))
}

/// Render the all-cities index page (SSR)
pub async fn render_cities_index(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT cp.city_slug, cp.city_name, cp.description, \
                (SELECT COUNT(*) FROM business_listings WHERE city_page_id = cp.id) AS listing_count \
         FROM city_pages cp WHERE cp.is_active = true ORDER BY cp.city_name",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut cities_html = String::new();
    for r in &rows {
        let slug: String = r.try_get("city_slug").unwrap_or_default();
        let name: String = r.try_get("city_name").unwrap_or_default();
        let desc: Option<String> = r.try_get("description").unwrap_or(None);
        let count: i64 = r.try_get("listing_count").unwrap_or(0);
        cities_html.push_str(&format!(
            r#"<a href="/zaarhub/{slug}" class="city-card">
      <h2>{name} <span class="count">{count}+</span></h2>
      <p>{desc}</p>
      <span class="arrow">Browse →</span>
    </a>
"#,
            slug = h(&slug),
            name = h(&name),
            count = count,
            desc = h(&desc.unwrap_or_default()),
        ));
    }

    // Editor's Picks for homepage
    let pick_rows = sqlx::query(
        "SELECT bl.id, bl.business_name, bl.logo_url, bl.rating, bl.review_count, bl.category, bl.editors_pick_note, cp.city_slug, cp.city_name \
         FROM business_listings bl \
         JOIN city_pages cp ON bl.city_page_id = cp.id \
         WHERE bl.is_editors_pick = true \
         ORDER BY bl.rating DESC NULLS LAST LIMIT 6"
    ).fetch_all(&state.db).await.unwrap_or_default();

    let mut picks_html = String::new();
    for p in &pick_rows {
        let pname: String = p.try_get("business_name").unwrap_or_default();
        let plogo: Option<String> = p.try_get("logo_url").unwrap_or(None);
        let prating: Option<f64> = p.try_get("rating").unwrap_or_default();
        let previews: i32 = p.try_get("review_count").unwrap_or(0);
        let pcat: Option<String> = p.try_get("category").unwrap_or(None);
        let pnote: Option<String> = p.try_get("editors_pick_note").unwrap_or(None);
        let pslug: String = p.try_get("city_slug").unwrap_or_default();
        let puname: String = p.try_get("city_name").unwrap_or_default();
        let pid: Uuid = p.try_get("id").unwrap_or_default();
        let pr = prating.unwrap_or(0.0);
        let pstars = String::from("\u{2605}".repeat(pr as usize)) + &"\u{2606}".repeat(5usize.saturating_sub(pr as usize));

        let plogo_html = match &plogo {
            Some(img) if !img.is_empty() => format!("<img src=\"{}\" alt=\"{}\" class=\"pick-logo\" loading=\"lazy\" onerror=\"this.style.display='none'\">", h(img), h(&pname)),
            _ => format!("<div class='pick-logo-placeholder'>{}</div>", h(&pname[..1.min(pname.len())])),
        };

        picks_html.push_str(&format!(
            r#"<a href="/zaarhub/{slug}/{id}" class="pick-card">
       {logo}
       <div class="pick-info">
         <h4>{name}</h4>
         <span class="pick-cat">{cat}</span>
         <div class="pick-stars">{stars} {rating:.1} · {reviews} reviews</div>
         {note_html}
         <span class="pick-city">{city}</span>
       </div>
     </a>"#,
            slug = h(&pslug),
            id = pid,
            logo = plogo_html,
            name = h(&pname),
            cat = h(&pcat.unwrap_or_default()),
            stars = pstars,
            rating = pr,
            reviews = previews,
            note_html = pnote.as_ref().map(|n| format!("<p class=\"pick-note\">\u{1f525} {}</p>", h(n))).unwrap_or_default(),
            city = h(&puname),
        ));
    }

    let picks_section = if picks_html.is_empty() {
        String::new()
    } else {
        format!("<div class=\"editors-picks-section\"><h2>\u{1f525} Editor's Picks</h2><p class=\"editors-picks-sub\">Hand-picked businesses our editors love</p><div class=\"picks-grid\">{}</div></div>", picks_html)
    };

    let footer = footer_html(&state.db).await;
    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>ZaarHub — Florida Local Business Directory</title>
<meta name="description" content="Browse 9 Florida cities with thousands of top-rated local businesses. Find restaurants, services, shops, and more.">
<meta property="og:title" content="ZaarHub — Florida Local Business Directory">
<meta property="og:type" content="website">
<meta property="twitter:card" content="summary_large_image">
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.5}}
header{{background:#2b3255;color:white;padding:16px 20px;text-align:center}}
header .logo{{font-size:22px;font-weight:800}}header .logo span{{color:#f27f2f}}
.hero{{padding:64px 20px 48px;text-align:center;background:linear-gradient(135deg,#2b3255,#1a1a3e);color:white}}
.hero h1{{font-size:clamp(28px,5vw,42px);margin-bottom:8px}}.hero h1 span{{color:#f27f2f}}
.hero p{{opacity:.85;max-width:600px;margin:0 auto;font-size:16px}}
.editors-picks-section{{max-width:1100px;margin:32px auto 0;padding:0 20px}}
.editors-picks-section h2{{font-size:22px;font-weight:800;color:#2b3255;margin-bottom:4px}}
.editors-picks-sub{{font-size:14px;color:#6b7280;margin-bottom:20px}}
.picks-grid{{display:grid;gap:16px;grid-template-columns:repeat(auto-fill,minmax(300px,1fr))}}
.pick-card{{display:flex;gap:14px;padding:18px;background:white;border-radius:14px;box-shadow:0 1px 3px rgba(0,0,0,.06);text-decoration:none;color:inherit;transition:all .2s;border:2px solid transparent}}
.pick-card:hover{{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}}
.pick-logo{{width:56px;height:56px;border-radius:12px;object-fit:cover;flex-shrink:0;background:#f27f2f}}
.pick-logo-placeholder{{width:56px;height:56px;border-radius:12px;background:#f27f2f;color:white;display:flex;align-items:center;justify-content:center;font-size:22px;font-weight:700;flex-shrink:0}}
.pick-info{{flex:1;min-width:0}}
.pick-info h4{{font-size:15px;font-weight:700;margin-bottom:2px}}
.pick-cat{{display:inline-block;font-size:10px;font-weight:600;text-transform:uppercase;color:#f27f2f;background:#fff7f0;padding:2px 8px;border-radius:4px;margin-bottom:4px}}
.pick-stars{{font-size:12px;color:#6b7280;margin-bottom:2px}}
.pick-note{{font-size:12px;color:#f27f2f;font-weight:600;margin-top:4px}}
.pick-city{{font-size:11px;color:#9ca3af;display:block;margin-top:4px}}
.city-grid{{max-width:1000px;margin:32px auto;padding:0 20px;display:grid;gap:20px;grid-template-columns:repeat(auto-fill,minmax(280px,1fr))}}
.city-card{{display:block;padding:24px;background:white;border-radius:14px;box-shadow:0 1px 3px rgba(0,0,0,.06);text-decoration:none;color:inherit;transition:all .2s;border:2px solid transparent}}
.city-card:hover{{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}}
.city-card h2{{font-size:20px;font-weight:700;margin-bottom:4px}}
.city-card h2 .count{{display:inline-block;background:#fff7f0;color:#f27f2f;font-size:12px;padding:3px 10px;border-radius:10px;margin-left:8px;vertical-align:middle}}
.city-card p{{font-size:14px;color:#6b7280;margin-bottom:12px;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}}
.city-card .arrow{{font-size:13px;color:#f27f2f;font-weight:600}}
footer{{text-align:center;padding:48px 20px;color:#6b7280;font-size:13px}}footer a{{color:#f27f2f;text-decoration:none}}
@media(max-width:600px){{.picks-grid{{grid-template-columns:1fr}}.city-grid{{grid-template-columns:1fr}}}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><span class="logo">Zaar<span>Hub</span></span></header>
<div class="hero"><h1>Florida <span>Business Directory</span></h1><p>Browse top-rated local businesses across 9 Florida cities with thousands of listings, reviews, and deals.</p></div>
{picks}
<div class="city-grid">{cities}</div>
{footer}
{cookie_banner}
</body>
</html>"#,
        picks = picks_section,
        cities = cities_html,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
    ))
}

/// Render a single business listing detail page (SSR, SEO-optimized)
pub async fn render_listing_page(
    Path((slug, listing_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let row = sqlx::query(
        "SELECT bl.*, cp.city_name \
         FROM business_listings bl \
         JOIN city_pages cp ON bl.city_page_id = cp.id \
         WHERE cp.city_slug = $1 AND bl.id = $2"
    ).bind(&slug).bind(listing_id)
     .fetch_optional(&state.db).await.unwrap_or(None);

    if row.is_none() {
        return axum::response::Html(format!(
            "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1.0\"><title>Not Found | ZaarHub</title><style>body{{font-family:system-ui,sans-serif;background:#f8f9fc;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}h1{{font-size:48px;color:#2b3255}}a{{color:#f27f2f}}</style></head><body><div style=\"text-align:center\"><h1>404</h1><p>Business not found.</p><a href=\"/zaarhub/{s}\">← Back to directory</a></div></body></html>",
            s = h(&slug)
        ));
    }

    let r = row.unwrap();
    let name: String = r.try_get("business_name").unwrap_or_default();
    let cat: Option<String> = r.try_get("category").unwrap_or(None);
    let desc: Option<String> = r.try_get("description").unwrap_or(None);
    let addr: Option<String> = r.try_get("address").unwrap_or(None);
    let phone: Option<String> = r.try_get("phone").unwrap_or(None);
    let web: Option<String> = r.try_get("website").unwrap_or(None);
    let logo: Option<String> = r.try_get("logo_url").unwrap_or(None);
    let rating: Option<f64> = r.try_get("rating").unwrap_or_default();
    let reviews: i32 = r.try_get("review_count").unwrap_or(0);
    let coordinates_lat: Option<f64> = r.try_get("coordinates_lat").unwrap_or_default();
    let coordinates_lng: Option<f64> = r.try_get("coordinates_lng").unwrap_or_default();
    let is_featured: bool = r.try_get("is_featured").unwrap_or(false);
    let city_name: String = r.try_get("city_name").unwrap_or_default();

    let rv = rating.unwrap_or(0.0);
    let stars = String::from("★".repeat(rv as usize)) + &"☆".repeat(5usize.saturating_sub(rv as usize));
    let page_title = format!("{} — {} | ZaarHub", name, cat.as_deref().unwrap_or("Business"));

    let logo_html = match &logo {
        Some(img) if !img.is_empty() => format!("<img src=\"{}\" alt=\"{}\" class=\"detail-logo\" onerror=\"this.style.display='none'\">", h(img), h(&name)),
        _ => format!("<div class='detail-logo-placeholder'>{}</div>", h(&name[..1.min(name.len())])),
    };
    let fb = if is_featured { "<span class=\"featured-badge\">⭐ Featured</span>" } else { "" };
    let cat_html = cat.as_ref().map(|c| format!("<span class=\"category-tag\">{}</span>", h(c))).unwrap_or_default();
    let desc_html = desc.as_ref().map(|d| format!("<p class=\"desc\">{}</p>", h(d))).unwrap_or_default();
    let addr_html = addr.as_ref().map(|a| format!("<div class='meta-row'><span class='icon'>📍</span><span>{}</span></div>", h(a))).unwrap_or_default();
    let phone_html = phone.as_ref().map(|p| format!("<div class='meta-row'><span class='icon'>📞</span><a href='tel:{}'>{}</a></div>", h(p), h(p))).unwrap_or_default();
    let web_html = web.as_ref().map(|w| format!("<div class='meta-row'><span class='icon'>🌐</span><a href='{}' target='_blank' rel='noopener'>{}</a></div>", h(w), h(w))).unwrap_or_default();
    let maps_html = match (coordinates_lat, coordinates_lng) {
        (Some(lat), Some(lng)) => format!("<div class='meta-row'><span class='icon'>🗺️</span><a href='https://maps.google.com/?q={},{}' target='_blank'>View on Google Maps</a></div>", lat, lng),
        _ => String::new(),
    };

    // Verification badge — query business_verifications matched via business name
    let verified_badge = sqlx::query(
        "SELECT bv.status FROM business_verifications bv \
         JOIN businesses b ON bv.business_id = b.id \
         WHERE b.name = $1 AND bv.status = 'approved' AND (bv.expires_at IS NULL OR bv.expires_at > now()) \
         LIMIT 1"
    ).bind(&name).fetch_optional(&state.db).await.unwrap_or(None)
     .is_some();

    let verified_html = if verified_badge {
        "<div class=\"verified-badge\">\u{2705} Verified Business</div>"
    } else {
        ""
    };

    // Data freshness — get most recent verification date or listing updated_at
    let freshness: Option<String> = {
        let ver_date: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
            "SELECT MAX(bv.created_at)::timestamp FROM business_verifications bv \
             JOIN businesses b ON bv.business_id = b.id \
             WHERE b.name = $1 AND bv.status = 'approved'"
        ).bind(&name).fetch_optional(&state.db).await.unwrap_or(None).flatten();

        let updated: Option<chrono::NaiveDateTime> = r.try_get("updated_at").unwrap_or(None);
        let latest = match (ver_date, updated) {
            (Some(v), Some(u)) => Some(if v > u { v } else { u }),
            (Some(v), None) => Some(v),
            (None, Some(u)) => Some(u),
            (None, None) => None,
        };
        latest.map(|dt| dt.format("%B %d, %Y").to_string())
    };

    let is_claimed: bool = r.try_get("is_claimed").unwrap_or(false);

    let freshness_html = match (&freshness, is_claimed) {
        (Some(date), _) => format!("<div class=\"freshness\">\u{1f4c5} Data last verified: {}</div>", h(date)),
        (None, false) => "<div class=\"freshness freshness-cta\">\u{1f4cc} <a href=\"/zaarhub-claim.html\">Claim this listing</a> to keep data fresh</div>".to_string(),
        (None, true) => String::new(),
    };

    // Offers
    let offers = sqlx::query(
        "SELECT offer_title, offer_type, discount_value, offer_description \
         FROM claim_offers WHERE listing_id = $1 AND is_active = true"
    ).bind(listing_id).fetch_all(&state.db).await.unwrap_or_default();

    let offers_html = if offers.is_empty() { String::new() } else {
        let mut htm = String::new();
        for o in &offers {
            let ot: String = o.try_get("offer_title").unwrap_or_default();
            let od: Option<String> = o.try_get("offer_description").unwrap_or(None);
            let dv: Option<String> = o.try_get("discount_value").unwrap_or(None);
            let otype: String = o.try_get("offer_type").unwrap_or_default();
            htm.push_str(&format!(
                "<div class=\"offer-card\"><div class=\"offer-type-tag\">{}</div><h3>{}</h3>{}{}<a href=\"/zaarhub-offer.html?listing_id={}\" class=\"claim-btn\">Claim →</a></div>",
                h(&otype), h(&ot),
                od.as_ref().map(|d| format!("<p>{}</p>", h(d))).unwrap_or_default(),
                dv.as_ref().map(|d| format!("<span class=\"discount-value\">{}</span>", h(d))).unwrap_or_default(),
                listing_id,
            ));
        }
        format!("<div class=\"offers-section\"><h2>🎁 Deals &amp; Offers</h2><div class=\"offer-grid\">{}</div></div>", htm)
    };

    // JSON-LD
    let schema = format!(
        r#"{{"@context":"https://schema.org","@type":"LocalBusiness","name":"{}","description":"{}","address":{{"@type":"PostalAddress","streetAddress":"{}"}},"aggregateRating":{{"@type":"AggregateRating","ratingValue":"{}","reviewCount":"{}"}},"url":"https://zaarhub.com/zaarhub/{}/{}"}}"#,
        h(&name), h(&desc.clone().unwrap_or_default()), h(&addr.unwrap_or_default()), rv, reviews, h(&slug), listing_id,
    );
    let footer = footer_html(&state.db).await;

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>{title}</title>
<meta name="description" content="{desc}">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{desc}">
<meta property="og:type" content="business.business">
<meta property="twitter:card" content="summary_large_image">
<link rel="canonical" href="https://zaarhub.com/zaarhub/{slug}/{id}">
<script type="application/ld+json">{schema}</script>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.5}}
header{{background:#2b3255;color:white;padding:16px 24px;position:sticky;top:0;z-index:100}}
header .inner{{max-width:1000px;margin:0 auto;display:flex;justify-content:space-between;align-items:center}}
header .logo{{font-size:20px;font-weight:800;color:white;text-decoration:none}}header .logo span{{color:#f27f2f}}
header nav a{{color:rgba(255,255,255,.75);text-decoration:none;font-size:13px;font-weight:500}}
.page{{max-width:1000px;margin:0 auto;padding:32px 20px}}
.breadcrumb{{font-size:13px;color:#6b7280;margin-bottom:24px}}.breadcrumb a{{color:#f27f2f;text-decoration:none}}
.detail-header{{display:flex;gap:20px;align-items:flex-start;margin-bottom:32px;flex-wrap:wrap}}
.detail-logo{{width:96px;height:96px;border-radius:20px;object-fit:cover;flex-shrink:0;background:#f27f2f}}
.detail-logo-placeholder{{width:96px;height:96px;border-radius:20px;background:#f27f2f;color:white;display:flex;align-items:center;justify-content:center;font-size:40px;font-weight:700;flex-shrink:0}}
.detail-info{{flex:1;min-width:250px}}
.detail-info h1{{font-size:28px;font-weight:800;margin-bottom:4px}}
.featured-badge{{display:inline-block;background:linear-gradient(135deg,#f27f2f,#e06e1a);color:white;font-size:11px;font-weight:700;padding:4px 10px;border-radius:6px;margin-bottom:8px}}
.category-tag{{display:inline-block;font-size:11px;font-weight:600;text-transform:uppercase;color:#f27f2f;background:#fff7f0;padding:3px 10px;border-radius:6px;margin-right:8px;margin-bottom:8px}}
.stars-row{{font-size:16px;margin:8px 0}}.stars-row .stars{{color:#f59e0b;font-size:20px}}
.desc{{font-size:15px;color:#4b5563;line-height:1.7;margin:16px 0}}
.detail-meta{{background:white;border-radius:14px;padding:24px;box-shadow:0 1px 3px rgba(0,0,0,.06);margin-bottom:24px}}
.meta-row{{display:flex;align-items:center;gap:10px;padding:10px 0;border-bottom:1px solid #f3f4f6;font-size:14px}}
.meta-row:last-child{{border-bottom:none}}.meta-row .icon{{font-size:18px;width:24px;text-align:center}}
.meta-row a{{color:#f27f2f;text-decoration:none;font-weight:500}}
.offers-section{{margin-bottom:24px}}.offers-section h2{{font-size:20px;font-weight:700;margin-bottom:16px}}
.verified-badge{{display:inline-block;background:#dcfce7;color:#166534;font-size:12px;font-weight:700;padding:4px 12px;border-radius:8px;margin-bottom:8px}}
.freshness{{font-size:12px;color:#9ca3af;margin-top:8px}}.freshness a{{color:#f27f2f;font-weight:600}}.freshness-cta{{font-size:12px}}
.offer-grid{{display:grid;gap:16px;grid-template-columns:repeat(auto-fill,minmax(280px,1fr))}}
.offer-card{{background:white;border-radius:14px;padding:20px;box-shadow:0 1px 3px rgba(0,0,0,.06);border-left:3px solid #16a34a}}
.offer-card h3{{font-size:16px;font-weight:700;margin-bottom:4px}}.offer-card p{{font-size:13px;color:#6b7280;margin-bottom:8px}}
.offer-type-tag{{display:inline-block;font-size:10px;font-weight:700;text-transform:uppercase;background:#dcfce7;color:#166534;padding:2px 8px;border-radius:4px;margin-bottom:6px}}
.discount-value{{display:inline-block;font-size:18px;font-weight:800;color:#16a34a;margin:4px 0 8px}}
.claim-btn{{display:inline-block;padding:8px 20px;background:#f27f2f;color:white;border-radius:8px;text-decoration:none;font-size:13px;font-weight:700;transition:all .15s}}
.claim-btn:hover{{background:#e06e1a;transform:translateY(-1px)}}
footer{{text-align:center;padding:32px;color:#6b7280;font-size:13px}}footer a{{color:#f27f2f;text-decoration:none}}
@media(max-width:600px){{.detail-header{{flex-direction:column;align-items:center;text-align:center}}.offer-grid{{grid-template-columns:1fr}}}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">Zaar<span>Hub</span></a><nav><a href="/zaarhub/{slug}">← Back to {city_name}</a></nav></div></header>
<div class="page">
<div class="breadcrumb"><a href="/zaarhub">Cities</a> › <a href="/zaarhub/{slug}">{city_name}</a> › {name}</div>
<div class="detail-header">{logo_html}<div class="detail-info">{fb}<h1>{name}</h1>{verified_html}{cat_html}<div class="stars-row"><span class="stars">{stars}</span> {rating:.1} · {reviews} reviews</div>{desc_html}{freshness_html}</div></div>
<div class="detail-meta">{addr_html}{phone_html}{web_html}{maps_html}</div>
{offers_html}
</div>
{footer}
{cookie_banner}
</body></html>"#,
        title = h(&page_title), desc = h(&desc.unwrap_or_default()),
        slug = h(&slug), id = listing_id, schema = schema,
        name = h(&name), city_name = h(&city_name),
        logo_html = logo_html, fb = fb, verified_html = verified_html, cat_html = cat_html,
        stars = stars, rating = rv, reviews = reviews,
        desc_html = desc_html, freshness_html = freshness_html, addr_html = addr_html,
        phone_html = phone_html, web_html = web_html,
        maps_html = maps_html, offers_html = offers_html,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
    ))
}

/// SSR — render a legal page (public)
pub async fn render_legal_page(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let row = sqlx::query(
        "SELECT title, content FROM zaarhub_legal_pages WHERE slug = $1 AND is_published = true"
    ).bind(&slug).fetch_optional(&state.db).await.unwrap_or(None);

    let (title, content) = match row {
        Some(r) => (
            r.try_get::<String,_>("title").unwrap_or_default(),
            r.try_get::<String,_>("content").unwrap_or_default(),
        ),
        None => {
            return axum::response::Html(format!(
                "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1.0\"><title>Page Not Found | ZaarHub</title><style>body{{font-family:system-ui,sans-serif;background:#f8f9fc;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}h1{{font-size:48px;color:#2b3255}}a{{color:#f27f2f}}</style></head><body><div style=\"text-align:center\"><h1>404</h1><p>Page not found.</p><a href=\"/zaarhub\">← Back to ZaarHub</a></div></body></html>",
            ));
        }
    };

    let footer = footer_html(&state.db).await;
    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>{title} | ZaarHub</title>
<meta name="description" content="{title} — ZaarHub">
<meta property="og:title" content="{title} | ZaarHub">
<meta property="og:type" content="website">
<link rel="canonical" href="https://zaarhub.com/legal/{slug}">
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.7}}
header{{background:#2b3255;color:white;padding:16px 24px;position:sticky;top:0;z-index:100}}
header .inner{{max-width:900px;margin:0 auto;display:flex;justify-content:space-between;align-items:center}}
header .logo{{font-size:20px;font-weight:800;color:white;text-decoration:none}}header .logo span{{color:#f27f2f}}
header nav a{{color:rgba(255,255,255,.75);text-decoration:none;font-size:13px;font-weight:500;margin-left:16px}}
.page{{max-width:900px;margin:0 auto;padding:40px 20px}}
.page h2{{font-size:28px;font-weight:800;margin-bottom:24px;color:#2b3255}}
.page h3{{font-size:18px;font-weight:700;margin:24px 0 8px;color:#1a1a2e}}
.page p{{margin-bottom:16px;color:#4b5563;font-size:15px}}
.page a{{color:#f27f2f}}
footer{{text-align:center;padding:48px 20px;color:#6b7280;font-size:13px}}
footer a{{color:#f27f2f;text-decoration:none}}
.footer-links{{display:flex;gap:20px;justify-content:center;margin-bottom:12px;flex-wrap:wrap}}
.footer-links a{{font-weight:500}}
</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">Zaar<span>Hub</span></a><nav><a href="/zaarhub">← All Cities</a></nav></div></header>
<div class="page">{content}</div>
{footer}
{cookie_banner}
</body></html>"#,
        title = h(&title), slug = h(&slug), content = content,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
    ))
}
