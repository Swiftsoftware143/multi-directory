//! Instruments every public HTML page with the visitor-tracking script.
//!
//! Why this exists: `crate::tracking_script` was only wired into the blog and directory
//! render paths, so the 35 static pages (home, city, business detail, industry templates,
//! offer pages …) shipped **untracked** — which is why `visitor_events` sat at a handful of
//! rows and the demand-analytics product had no data source. Doing the injection here, as a
//! response middleware, means a newly added page cannot ship untracked.
//!
//! Deliberately excluded: the platform console, role dashboards and business-side portals —
//! their traffic is not consumer demand and would pollute the analytics.

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, Method};
use axum::middleware::Next;
use axum::response::Response;

/// Path prefixes whose traffic must never be counted as consumer demand.
const EXCLUDED_PREFIXES: &[&str] = &[
    "/api/",
    "/admin",
    "/pricing-admin",
    "/blog-features-admin",
    "/content-research",
    "/business-portal",
    "/business-dashboard",
    "/supplier-portal",
    "/scanner",
    "/login",
    "/thank-you",
];

/// Large enough for the biggest page this app serves, small enough to refuse a runaway body.
const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;

pub async fn inject_tracking_beacon(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let is_get = req.method() == Method::GET;
    let res = next.run(req).await;

    if !is_get || EXCLUDED_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return res;
    }

    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);
    if !is_html {
        return res;
    }

    let (mut parts, body) = res.into_parts();
    let bytes = match to_bytes(body, MAX_HTML_BYTES).await {
        Ok(b) => b,
        // Body could not be buffered — hand the original response back untouched rather
        // than breaking a page we were only trying to instrument.
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    let html = String::from_utf8_lossy(&bytes);
    if html.contains("window._vt") || html.contains("tracking-beacon") {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let injected = crate::tracking_script::inject_tracking_script(&html);
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(injected))
}
