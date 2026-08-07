/// SEO handlers: sitemaps, robots.txt, schema.org markup
use axum::extract::State;
use sqlx::Row;

use crate::AppState;

/// Serve sitemap.xml — all city pages + individual listing pages
pub async fn sitemap_xml(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let cities = sqlx::query(
        "SELECT city_slug FROM city_pages WHERE is_active = true ORDER BY city_name",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let listings = sqlx::query(
        "SELECT bl.id, cp.city_slug FROM business_listings bl \
         JOIN city_pages cp ON bl.city_page_id = cp.id \
         WHERE cp.is_active = true ORDER BY bl.is_featured DESC, bl.rating DESC LIMIT 5000",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let host = "https://zaarhub.com";

    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"
        xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">
"#,
    );

    // Homepage
    xml.push_str(&format!(
        "  <url><loc>{host}/</loc><changefreq>daily</changefreq><priority>1.0</priority></url>\n"
    ));

    // All cities index
    xml.push_str(&format!(
        "  <url><loc>{host}/zaarhub</loc><changefreq>daily</changefreq><priority>0.9</priority></url>\n"
    ));

    // City pages
    for city in &cities {
        let slug: String = city.try_get("city_slug").unwrap_or_default();
        xml.push_str(&format!(
            "  <url><loc>{host}/zaarhub/{slug}</loc><changefreq>daily</changefreq><priority>0.8</priority></url>\n"
        ));
    }

    // Business listing pages
    for listing in &listings {
        let id: uuid::Uuid = listing.try_get("id").unwrap_or_default();
        let slug: String = listing.try_get("city_slug").unwrap_or_default();
        xml.push_str(&format!(
            "  <url><loc>{host}/zaarhub/{slug}/{id}</loc><changefreq>weekly</changefreq><priority>0.6</priority></url>\n"
        ));
    }

    xml.push_str("</urlset>\n");

    axum::response::Response::builder()
        .header("Content-Type", "application/xml; charset=utf-8")
        .header("Cache-Control", "public, max-age=3600")
        .body(axum::body::Body::from(xml))
        .unwrap()
}

/// Serve robots.txt
pub async fn robots_txt() -> impl axum::response::IntoResponse {
    let body = "User-agent: *
Allow: /
Allow: /zaarhub/
Allow: /zaarhub-city.html
Allow: /zaarhub-offer.html

# Sitemaps
Sitemap: https://zaarhub.com/sitemap.xml

# Crawl delay — be nice to our server
Crawl-delay: 2

# Disallow API endpoints from indexing
Disallow: /api/
Disallow: /auth/
Disallow: /admin/
";

    axum::response::Response::builder()
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400")
        .body(axum::body::Body::from(body))
        .unwrap()
}
