/// SEO handlers: the legacy sitemap redirect and robots.txt.
///
/// The site has exactly ONE sitemap: `/sitemap.xml` (served by `subfolder::sitemap_handler`),
/// whose business entries are the human-slug canonical URLs.
///
/// This older `/zaarhub-sitemap.xml` listed 3,988 `/zaarhub/<city>/<uuid>` URLs — every one of
/// which 301-redirects to the canonical slug page. A sitemap made up entirely of redirects wastes
/// crawl budget and contradicts the canonical signal, so it is now a permanent redirect to the
/// real file instead of a second competing document (2026-11-11).
pub async fn sitemap_xml() -> impl axum::response::IntoResponse {
    axum::response::Redirect::permanent("/sitemap.xml")
}

/// Serve robots.txt
pub async fn robots_txt() -> impl axum::response::IntoResponse {
    let body = "User-agent: *\nAllow: /\nAllow: /zaarhub/\nAllow: /zaarhub-city.html\nAllow: /zaarhub-offer.html\nAllow: /rfq-marketplace\nAllow: /coop-hub\nAllow: /lead-exchange\nAllow: /b2b-marketplace.html\nAllow: /supplier-portal.html\nAllow: /legal/\n\n# Sitemaps — ONE sitemap for the whole site (kanban t_f97664fe). This secondary\n# robots document used to advertise /zaarhub-sitemap.xml, a second file listing the\n# same pages; two sitemaps for one site split the crawl budget.\nSitemap: https://zaarhub.com/sitemap.xml\n\n# Crawl delay — be nice to our server\nCrawl-delay: 2\n\n# Disallow API endpoints from indexing\nDisallow: /api/\nDisallow: /auth/\nDisallow: /admin/\n";

    axum::response::Response::builder()
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Cache-Control", "public, max-age=86400")
        .body(axum::body::Body::from(body))
        .unwrap()
}
