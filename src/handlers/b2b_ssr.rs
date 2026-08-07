/// SSR handlers for B2B feature pages: RFQ Marketplace, Co-op Hub, Lead Exchange
use axum::extract::State;
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

/// Shared footer HTML
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

/// Cookie consent banner
const COOKIE_BANNER: &str = r#"<div id="cookie-banner" style="position:fixed;bottom:0;left:0;right:0;background:#1a1a2e;color:white;padding:20px 24px;z-index:9999;display:flex;align-items:center;justify-content:center;gap:20px;flex-wrap:wrap;box-shadow:0 -4px 20px rgba(0,0,0,.3);font-size:13px;line-height:1.5">
<div style="max-width:900px">We use cookies for essential site functionality and analytics to improve your experience. By continuing to use ZaarHub, you accept our <a href="/legal/privacy" style="color:#f27f2f">Privacy Policy</a> and <a href="/legal/terms" style="color:#f27f2f">Terms of Service</a>.</div>
<div style="display:flex;gap:10px">
<button onclick="document.getElementById('cookie-banner').style.display='none';localStorage.setItem('zaarhub_cookies_accepted','1')" style="background:#f27f2f;color:white;border:none;padding:10px 24px;border-radius:8px;font-weight:700;font-size:13px;cursor:pointer;white-space:nowrap">Accept All</button>
<button onclick="document.getElementById('cookie-banner').style.display='none';localStorage.setItem('zaarhub_cookies_accepted','essential')" style="background:transparent;color:white;border:1px solid rgba(255,255,255,.3);padding:10px 24px;border-radius:8px;font-weight:600;font-size:13px;cursor:pointer;white-space:nowrap">Essential Only</button>
</div>
</div>
<script>if(localStorage.getItem('zaarhub_cookies_accepted'))document.getElementById('cookie-banner').style.display='none'</script>"#;

const SHARED_CSS: &str = r#"
*{margin:0;padding:0;box-sizing:border-box}
body{font-family:Inter,system-ui,sans-serif;background:#f8f9fc;color:#1a1a2e;line-height:1.5}
header{background:#2b3255;color:white;padding:16px 20px;position:sticky;top:0;z-index:100}
header .inner{max-width:1200px;margin:0 auto;display:flex;justify-content:space-between;align-items:center}
header .logo{font-size:22px;font-weight:800;color:white;text-decoration:none}header .logo span{color:#f27f2f}
header nav a{color:rgba(255,255,255,.8);text-decoration:none;font-size:14px;font-weight:500;margin-left:20px}
.hero{background:linear-gradient(135deg,#2b3255,#1a1a3e);color:white;padding:48px 20px;text-align:center}
.hero h1{font-size:clamp(24px,5vw,36px);margin-bottom:8px}.hero h1 span{color:#f27f2f}
.hero p{opacity:.85;max-width:600px;margin:0 auto;font-size:15px}
.container{max-width:1100px;margin:0 auto;padding:32px 20px}
.page-title{font-size:24px;font-weight:800;color:#2b3255;margin-bottom:24px}
.card{background:white;border-radius:14px;padding:20px;box-shadow:0 1px 3px rgba(0,0,0,.06);border:1px solid #f3f4f6;transition:all .2s;margin-bottom:16px;text-decoration:none;color:inherit;display:block}
.card:hover{box-shadow:0 10px 40px rgba(0,0,0,.08);transform:translateY(-2px);border-color:#f27f2f}
.card h3{font-size:17px;font-weight:700;margin-bottom:4px;color:#1a1a2e}
.card .tag{display:inline-block;font-size:10px;font-weight:700;text-transform:uppercase;padding:3px 10px;border-radius:6px;margin-right:6px;margin-bottom:8px}
.tag-open{background:#dcfce7;color:#166534}.tag-closed{background:#f3f4f6;color:#6b7280}.tag-recruiting{background:#ede9fe;color:#7c3aed}
.cat-tag{display:inline-block;font-size:11px;font-weight:600;text-transform:uppercase;color:#f27f2f;background:#fff7f0;padding:3px 10px;border-radius:6px;margin-right:6px;margin-bottom:8px}
.desc{font-size:13px;color:#6b7280;line-height:1.6;display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden;margin-bottom:8px}
.meta{display:flex;gap:12px;flex-wrap:wrap;font-size:12px;color:#9ca3af;align-items:center}
.meta-row{display:flex;align-items:center;gap:6px}
.badge{display:inline-block;font-size:11px;font-weight:700;padding:3px 10px;border-radius:6px}
.badge-green{background:#dcfce7;color:#166534}.badge-orange{background:#fff7ed;color:#ea580c}
.empty-state{text-align:center;padding:48px 20px;color:#9ca3af}
.empty-state h3{font-size:20px;color:#6b7280;margin-bottom:8px}
.btn{display:inline-block;padding:10px 24px;border-radius:8px;font-size:14px;font-weight:700;border:none;cursor:pointer;transition:all .2s;text-decoration:none}
.btn-primary{background:#f27f2f;color:white}
.btn-primary:hover{background:#e06e1a;transform:translateY(-1px)}
.btn-secondary{background:#2b3255;color:white}
.btn-secondary:hover{background:#1a1a3e;transform:translateY(-1px)}
.btn-outline{background:transparent;border:2px solid #f27f2f;color:#f27f2f}
.btn-outline:hover{background:#fff7f0}
.btn-group{display:flex;gap:8px;flex-wrap:wrap}
.grid-2{display:grid;gap:16px;grid-template-columns:repeat(auto-fill,minmax(320px,1fr))}
footer{text-align:center;padding:48px 20px;color:#6b7280;font-size:13px}footer a{color:#f27f2f;text-decoration:none}
.footer-links{display:flex;gap:20px;justify-content:center;margin-bottom:12px;flex-wrap:wrap}.footer-links a{font-weight:500}
@media(max-width:600px){.grid-2{grid-template-columns:1fr}}
"#;

/// Render RFQ Marketplace page (public SSR)
pub async fn render_rfq_marketplace(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT r.id, r.title, r.category, r.description, r.budget_min, r.budget_max, \
                r.quantity, r.status, r.created_at, r.deadline, \
                (SELECT COUNT(*) FROM rfq_bids WHERE rfq_id = r.id)::bigint AS bid_count, \
                COALESCE(b.name, 'Unknown') as poster_name \
         FROM rfqs r \
         LEFT JOIN businesses b ON r.poster_business_id = b.id \
         WHERE r.is_public = true \
         ORDER BY r.created_at DESC LIMIT 50"
    ).fetch_all(&state.db).await.unwrap_or_default();

    let mut cards_html = String::new();
    for r in &rows {
        let rid: Uuid = r.try_get("id").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        let cat: Option<String> = r.try_get("category").unwrap_or(None);
        let desc: Option<String> = r.try_get("description").unwrap_or(None);
        let budget_min: Option<rust_decimal::Decimal> = r.try_get("budget_min").unwrap_or_default();
        let budget_max: Option<rust_decimal::Decimal> = r.try_get("budget_max").unwrap_or_default();
        let quantity: Option<String> = r.try_get("quantity").unwrap_or(None);
        let status: String = r.try_get("status").unwrap_or_default();
        let deadline: Option<String> = r.try_get("deadline").unwrap_or(None);
        let bid_count: i64 = r.try_get("bid_count").unwrap_or(0);
        let poster: String = r.try_get("poster_name").unwrap_or_default();

        let status_class = if status == "open" { "tag-open" } else { "tag-closed" };
        let budget_str = match (budget_min, budget_max) {
            (Some(min), Some(max)) => format!("${:.0} – ${:.0}", min, max),
            (Some(min), None) => format!("From ${:.0}", min),
            _ => String::new(),
        };

        cards_html.push_str(&format!(
            r#"<a href="/rfq-marketplace?rfq={id}" class="card">
    <h3>{title}</h3>
    {cat_html}
    {desc_html}
    <div class="meta">
      <span class="tag {status_class}">{status}</span>
      {budget_span}
      {qty_span}
      <span>📦 {bid_count} bids</span>
      {deadline_span}
      <span>Posted by {poster}</span>
    </div>
  </a>"#,
            id = rid,
            title = h(&title),
            cat_html = cat.as_ref().map(|c| format!("<span class=\"cat-tag\">{}</span>", h(c))).unwrap_or_default(),
            desc_html = desc.as_ref().map(|d| format!("<p class=\"desc\">{}</p>", h(d))).unwrap_or_default(),
            status_class = status_class,
            status = h(&status),
            budget_span = if !budget_str.is_empty() { format!("<span>💰 {}</span>", h(&budget_str)) } else { String::new() },
            qty_span = quantity.as_ref().map(|q| format!("<span>📏 {}</span>", h(q))).unwrap_or_default(),
            bid_count = bid_count,
            deadline_span = deadline.as_ref().map(|d| format!("<span>⏰ {}</span>", h(d))).unwrap_or_default(),
            poster = h(&poster),
        ));
    }

    let content = if cards_html.is_empty() {
        "<div class=\"empty-state\"><h3>No RFQs Yet</h3><p>Be the first to post a request for quotes. Businesses post what they need, and suppliers bid to win the work.</p></div>".to_string()
    } else {
        format!("<div class=\"grid-2\">{}</div>", cards_html)
    };

    let footer = footer_html(&state.db).await;
    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>RFQ Marketplace — Request for Quotes | ZaarHub B2B</title>
<meta name="description" content="Browse RFQs from businesses across Florida. Post your own request for quotes and get competitive bids from verified suppliers.">
<meta property="og:title" content="RFQ Marketplace | ZaarHub B2B">
<meta property="og:type" content="website">
<link rel="canonical" href="https://zaarhub.com/rfq-marketplace">
<style>{css}</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">🌐 Zaar<span>Hub</span> <span style="font-weight:300;font-size:.85rem;opacity:.6;letter-spacing:0">B2B</span></a><nav><a href="/b2b-marketplace.html">Marketplace</a><a href="/coop-hub">Co-op Hub</a><a href="/lead-exchange">Lead Exchange</a></nav></div></header>
<div class="hero"><h1>📋 <span>RFQ</span> Marketplace</h1><p>Businesses post what they need. Suppliers browse and bid. A B2B lead exchange that Google can't replicate.</p></div>
<div class="container">
  <div class="btn-group" style="margin-bottom:24px">
    <a href="/supplier" class="btn btn-primary">Post an RFQ</a>
    <a href="/b2b-marketplace.html" class="btn btn-outline">Browse Marketplace</a>
  </div>
  {content}
</div>
{footer}
{cookie_banner}
</body>
</html>"#,
        css = SHARED_CSS,
        content = content,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
    ))
}

/// Render Co-op Buying Groups page (public SSR)
pub async fn render_coop_hub(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT g.id, g.name, g.description, g.category, g.status, g.member_count, \
                g.min_members, g.max_members, \
                (SELECT COUNT(*) FROM buying_group_deals WHERE group_id = g.id AND status = 'active')::bigint AS active_deals, \
                g.created_at, COALESCE(b.name, 'Unknown') as founder_name \
         FROM buying_groups g \
         LEFT JOIN businesses b ON g.founder_business_id = b.id \
         WHERE g.status != 'archived' \
         ORDER BY g.created_at DESC LIMIT 50"
    ).fetch_all(&state.db).await.unwrap_or_default();

    let mut cards_html = String::new();
    for r in &rows {
        let gid: Uuid = r.try_get("id").unwrap_or_default();
        let name: String = r.try_get("name").unwrap_or_default();
        let desc: Option<String> = r.try_get("description").unwrap_or(None);
        let cat: Option<String> = r.try_get("category").unwrap_or(None);
        let status: String = r.try_get("status").unwrap_or_default();
        let member_count: i32 = r.try_get("member_count").unwrap_or(0);
        let min_members: i32 = r.try_get("min_members").unwrap_or(0);
        let max_members: i32 = r.try_get("max_members").unwrap_or(0);
        let active_deals: i64 = r.try_get("active_deals").unwrap_or(0);
        let founder: String = r.try_get("founder_name").unwrap_or_default();

        let status_class = match status.as_str() {
            "recruiting" => "tag-recruiting",
            "negotiating" => "badge badge-orange",
            "active" => "badge badge-green",
            _ => "tag-closed",
        };

        cards_html.push_str(&format!(
            r#"<a href="/coop-hub?group={id}" class="card">
    <h3>{name}</h3>
    {cat_html}
    {desc_html}
    <div class="meta">
      {status_span}
      <span>👥 {members}/{max} members (need {min})</span>
      <span>🤝 {deals} active deals</span>
      <span>Founded by {founder}</span>
    </div>
  </a>"#,
            id = gid,
            name = h(&name),
            cat_html = cat.as_ref().map(|c| format!("<span class=\"cat-tag\">{}</span>", h(c))).unwrap_or_default(),
            desc_html = desc.as_ref().map(|d| format!("<p class=\"desc\">{}</p>", h(d))).unwrap_or_default(),
            status_span = format!("<span class=\"tag {}\">{}</span>", status_class, h(&status)),
            members = member_count,
            max = max_members,
            min = min_members,
            deals = active_deals,
            founder = h(&founder),
        ));
    }

    let content = if cards_html.is_empty() {
        "<div class=\"empty-state\"><h3>No Co-op Groups Yet</h3><p>Form a buying group with other businesses and negotiate better pricing together. Group purchasing = collective bargaining power.</p></div>".to_string()
    } else {
        format!("<div class=\"grid-2\">{}</div>", cards_html)
    };

    let footer = footer_html(&state.db).await;
    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Co-op Buying Groups — Bulk Purchasing Power | ZaarHub B2B</title>
<meta name="description" content="Join buying groups with other businesses to negotiate bulk pricing. Pool purchasing power and save on supplies, equipment, and more.">
<meta property="og:title" content="Co-op Buying Groups | ZaarHub B2B">
<meta property="og:type" content="website">
<link rel="canonical" href="https://zaarhub.com/coop-hub">
<style>{css}</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">🌐 Zaar<span>Hub</span> <span style="font-weight:300;font-size:.85rem;opacity:.6;letter-spacing:0">B2B</span></a><nav><a href="/rfq-marketplace">RFQ</a><a href="/b2b-marketplace.html">Marketplace</a><a href="/lead-exchange">Lead Exchange</a></nav></div></header>
<div class="hero"><h1>🤝 <span>Co-op</span> Buying Hub</h1><p>Pool purchasing power with other businesses. Group buying = lower prices, better terms. A marketplace Google can't touch.</p></div>
<div class="container">
  <div class="btn-group" style="margin-bottom:24px">
    <a href="/supplier" class="btn btn-primary">Start a Co-op</a>
    <a href="/b2b-marketplace.html" class="btn btn-outline">Browse Marketplace</a>
  </div>
  {content}
</div>
{footer}
{cookie_banner}
</body>
</html>"#,
        css = SHARED_CSS,
        content = content,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
    ))
}

/// Render Lead Exchange page (public SSR)
pub async fn render_lead_exchange(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT l.id, l.title, l.description, l.category, l.location, \
                l.estimated_value, l.status, l.created_at, l.expires_at, l.source, \
                COALESCE(b.name, 'Unknown') as poster_name \
         FROM shared_leads l \
         LEFT JOIN businesses b ON l.poster_business_id = b.id \
         WHERE l.status = 'available' \
         ORDER BY l.created_at DESC LIMIT 50"
    ).fetch_all(&state.db).await.unwrap_or_default();

    let mut cards_html = String::new();
    for r in &rows {
        let lid: Uuid = r.try_get("id").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        let desc: Option<String> = r.try_get("description").unwrap_or(None);
        let cat: Option<String> = r.try_get("category").unwrap_or(None);
        let location: Option<String> = r.try_get("location").unwrap_or(None);
        let est_value: Option<rust_decimal::Decimal> = r.try_get("estimated_value").unwrap_or_default();
        let source: Option<String> = r.try_get("source").unwrap_or(None);
        let poster: String = r.try_get("poster_name").unwrap_or_default();
        let expires_at: Option<String> = r.try_get("expires_at").unwrap_or(None);

        cards_html.push_str(&format!(
            r#"<a href="/lead-exchange?lead={id}" class="card">
    <h3>{title}</h3>
    {cat_html}
    {desc_html}
    <div class="meta">
      <span class="tag tag-open">Available</span>
      {value_span}
      {loc_span}
      {source_span}
      {expires_span}
      <span>Posted by {poster}</span>
    </div>
  </a>"#,
            id = lid,
            title = h(&title),
            cat_html = cat.as_ref().map(|c| format!("<span class=\"cat-tag\">{}</span>", h(c))).unwrap_or_default(),
            desc_html = desc.as_ref().map(|d| format!("<p class=\"desc\">{}</p>", h(d))).unwrap_or_default(),
            value_span = est_value.map(|v| format!("<span>💵 ${:.0}</span>", v)).unwrap_or_default(),
            loc_span = location.as_ref().map(|l| format!("<span>📍 {}</span>", h(l))).unwrap_or_default(),
            source_span = source.as_ref().map(|s| format!("<span>📬 {}</span>", h(s))).unwrap_or_default(),
            expires_span = expires_at.as_ref().map(|e| format!("<span>⏰ Expires {}</span>", h(e))).unwrap_or_default(),
            poster = h(&poster),
        ));
    }

    let content = if cards_html.is_empty() {
        "<div class=\"empty-state\"><h3>No Leads Available</h3><p>Share leads you can't fulfill with other businesses. The referral economy creates value that no search engine can replicate.</p></div>".to_string()
    } else {
        cards_html
    };

    let footer = footer_html(&state.db).await;
    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Lead Exchange — Share & Claim Business Leads | ZaarHub B2B</title>
<meta name="description" content="Share leads you can't fulfill with other businesses, and claim leads others have posted. A referral economy that no generic search engine can replicate.">
<meta property="og:title" content="Lead Exchange | ZaarHub B2B">
<meta property="og:type" content="website">
<link rel="canonical" href="https://zaarhub.com/lead-exchange">
<style>{css}</style>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;600;700;800&display=swap" rel="stylesheet">
</head>
<body>
<header><div class="inner"><a href="/zaarhub" class="logo">🌐 Zaar<span>Hub</span> <span style="font-weight:300;font-size:.85rem;opacity:.6;letter-spacing:0">B2B</span></a><nav><a href="/rfq-marketplace">RFQ</a><a href="/coop-hub">Co-op Hub</a><a href="/b2b-marketplace.html">Marketplace</a></nav></div></header>
<div class="hero"><h1>📬 <span>Lead</span> Exchange</h1><p>Share leads you can't fulfill. Claim leads others can't handle. A referral economy built into the directory — untouchable by Google.</p></div>
<div class="container">
  <div class="btn-group" style="margin-bottom:24px">
    <a href="/supplier" class="btn btn-primary">Share a Lead</a>
    <a href="/b2b-marketplace.html" class="btn btn-outline">Browse Marketplace</a>
  </div>
  {content}
</div>
{footer}
{cookie_banner}
</body>
</html>"#,
        css = SHARED_CSS,
        content = content,
        footer = footer,
        cookie_banner = COOKIE_BANNER,
    ))
}
