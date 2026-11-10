//! ONE canonical public host for the ZaarHub directory content.
//!
//! Three hostnames are served by one app and one vhost: `zaarhub.com`,
//! `www.zaarhub.com` and `directory.swiftsoftware.net`. The last one is also the
//! FACTORY's home — it is where the software that creates directories and its
//! admin console live — so it must keep answering; it is not redirected away.
//!
//! Every public page used to derive its `<link rel="canonical">` from the
//! request `Host`, so all three hostnames self-canonicalised the same content and
//! Search Console reported 8 "Duplicate without user-selected canonical" rows.
//! Canonical URLs are now PINNED to [`CANONICAL_ORIGIN`] for those three hosts.
//! Any other host keeps its own origin: a directory with its own mapped domain,
//! and localhost / 127.x used by the probe harness and local tests, must not be
//! rewritten to ZaarHub.
//!
//! Two pieces:
//!   * [`is_canonical_host`] + [`CANONICAL_ORIGIN`] — the rule, shared with the
//!     URL builder in `handlers::subfolder` (which pins canonical, og:url,
//!     JSON-LD and sitemap `<loc>` values) and with the robots.txt handler.
//!   * [`pin_canonical`] — a response middleware that pins the same tags on the
//!     documents the app serves as static files (the SPA shell at `/`, the
//!     business portal at `/portal`, the other static pages), which build no
//!     url server-side at all.

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

/// The canonical public origin for the ZaarHub directory.
pub const CANONICAL_ORIGIN: &str = "https://zaarhub.com";

/// Hosts that serve the SAME public directory and therefore canonicalise to
/// [`CANONICAL_ORIGIN`]. `www` is 301'd to the apex at the edge; it is listed so
/// a request that bypasses the edge still self-canonicalises correctly.
pub const CANONICAL_HOSTS: &[&str] = &[
    "zaarhub.com",
    "www.zaarhub.com",
    "directory.swiftsoftware.net",
];

/// Lower-cased hostname without the port.
fn hostname(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// True when this request host serves the ZaarHub directory itself.
pub fn is_canonical_host(host: Option<&str>) -> bool {
    match host {
        Some(h) => CANONICAL_HOSTS.contains(&hostname(h).as_str()),
        None => false,
    }
}

/// The ONE host `robots.txt` may advertise a sitemap from: the primary host, and
/// the local/dev hosts (which nobody crawls) so local probes still see the line.
pub fn is_primary_host(host: Option<&str>) -> bool {
    match host {
        None => true,
        Some(h) => {
            let h = hostname(h);
            h == "zaarhub.com"
                || h == "localhost"
                || h.starts_with("127.")
                || h.starts_with("10.")
                || h.starts_with("192.168.")
                || h.starts_with("172.")
        }
    }
}

/// `https://zaarhub.com` + a normalized path (query preserved, trailing slash
/// dropped except at the root, so `/apopka/` and `/apopka` are one URL).
pub fn canonical_url(path_and_query: &str) -> String {
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };
    let mut p = if path.is_empty() { "/" } else { path };
    if p.len() > 1 {
        p = p.trim_end_matches('/');
    }
    if !p.starts_with('/') {
        p = "/";
    }
    match query {
        Some(q) if !q.is_empty() => format!("{}{}?{}", CANONICAL_ORIGIN, p, q),
        _ => format!("{}{}", CANONICAL_ORIGIN, p),
    }
}

/// Re-point an existing absolute or root-relative url at [`CANONICAL_ORIGIN`],
/// keeping its path and query exactly (a page whose canonical deliberately names
/// another path — a legacy alias canonicalising to its new url — keeps that path).
fn pin_url(existing: &str) -> String {
    let rest = match existing.find("://") {
        Some(i) => {
            let after = &existing[i + 3..];
            match after.find('/') {
                Some(j) => &after[j..],
                None => "/",
            }
        }
        None if existing.starts_with('/') => existing,
        None => return existing.to_string(),
    };
    let mut out = String::with_capacity(CANONICAL_ORIGIN.len() + rest.len());
    out.push_str(CANONICAL_ORIGIN);
    out.push_str(rest);
    out
}

/// Case-insensitive `find`, returning a byte offset into `hay`.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    let first = n[0].to_ascii_lowercase();
    for i in 0..=(h.len() - n.len()) {
        if h[i].to_ascii_lowercase() != first {
            continue;
        }
        let mut ok = true;
        for (j, nb) in n.iter().enumerate().skip(1) {
            if h[i + j].to_ascii_lowercase() != nb.to_ascii_lowercase() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }
    None
}

/// Rewrite the `attr` value of the first `<tag_name … marker …>` element in
/// `html`. Returns `None` when no such element (or no such attribute) exists, so
/// the caller can decide to insert one instead.
fn rewrite_tag_attr(html: &str, tag_name: &str, marker: &str, attr: &str) -> Option<String> {
    let m = find_ci(html, marker)?;
    let tag_start = html[..m].rfind('<')?;
    let tag_end = m + html[m..].find('>')?;
    let tag = &html[tag_start..=tag_end];
    if !tag[..1 + tag_name.len()].eq_ignore_ascii_case(&format!("<{tag_name}")) {
        return None;
    }
    let a = find_ci(tag, attr)?;
    let bytes = tag.as_bytes();
    let mut i = a + attr.len();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    let (vstart, vend) = if quote == b'"' || quote == b'\'' {
        let vs = i + 1;
        let ve = vs + tag[vs..].find(quote as char)?;
        (vs, ve)
    } else {
        let vs = i;
        let ve = vs
            + tag[vs..]
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(tag.len() - vs);
        (vs, ve)
    };
    let (abs_vs, abs_ve) = (tag_start + vstart, tag_start + vend);
    let pinned = pin_url(&html[abs_vs..abs_ve]);
    let mut out = String::with_capacity(html.len() + pinned.len());
    out.push_str(&html[..abs_vs]);
    out.push_str(&pinned);
    out.push_str(&html[abs_ve..]);
    Some(out)
}

/// Pin every canonical signal in one HTML document to [`CANONICAL_ORIGIN`].
///
/// Replaces an existing `<link rel="canonical">`, `<meta property="og:url">` and
/// `<meta name="twitter:url">` (origin only — the path is preserved), and inserts
/// the three tags when the document has none, which is the case for the SPA shell
/// (`/`) and the business portal (`/portal`).
pub fn pin_html(html: &str, path_and_query: &str) -> String {
    let mut out = html.to_string();
    let url = canonical_url(path_and_query);

    if let Some(next) = rewrite_tag_attr(&out, "link", "rel=\"canonical\"", "href")
        .or_else(|| rewrite_tag_attr(&out, "link", "rel='canonical'", "href"))
    {
        out = next;
    }
    if let Some(next) = rewrite_tag_attr(&out, "meta", "property=\"og:url\"", "content")
        .or_else(|| rewrite_tag_attr(&out, "meta", "property='og:url'", "content"))
    {
        out = next;
    }
    if let Some(next) = rewrite_tag_attr(&out, "meta", "name=\"twitter:url\"", "content")
        .or_else(|| rewrite_tag_attr(&out, "meta", "name='twitter:url'", "content"))
    {
        out = next;
    }

    let has = |needle: &str| find_ci(&out, needle).is_some();
    let mut block = String::new();
    if !has("rel=\"canonical\"") && !has("rel='canonical'") {
        block.push_str(&format!("<link rel=\"canonical\" href=\"{url}\">"));
    }
    if !has("property=\"og:url\"") {
        block.push_str(&format!("<meta property=\"og:url\" content=\"{url}\">"));
    }
    if !has("name=\"twitter:url\"") {
        block.push_str(&format!("<meta name=\"twitter:url\" content=\"{url}\">"));
    }
    if block.is_empty() {
        return out;
    }
    match find_ci(&out, "</head>") {
        Some(pos) => out.insert_str(pos, &block),
        None => {}
    }
    out
}

/// Paths that are private consoles or non-page surfaces: never canonicalised.
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
    "/logout",
    "/visitor",
    "/thank-you",
    "/zaarhub-admin",
    "/zaarhub-analytics",
    "/uploads/",
];

fn excluded(path: &str) -> bool {
    EXCLUDED_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// Largest document this middleware will buffer — the same ceiling the beacon
/// middleware uses; the biggest page this app serves is well under it.
const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;

/// Response middleware: pin the canonical url of every public HTML page served
/// from the ZaarHub hostnames, whatever host served it.
pub async fn pin_canonical(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| path.clone());
    let host = req
        .headers()
        .get("x-forwarded-host")
        .or_else(|| req.headers().get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let is_get = req.method() == Method::GET;

    let res = next.run(req).await;

    if !is_get || excluded(&path) || !is_canonical_host(host.as_deref()) {
        return res;
    }
    if res.status() != StatusCode::OK {
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
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };
    let html = String::from_utf8_lossy(&bytes);
    let pinned = pin_html(&html, &path_and_query);
    if pinned == html {
        return Response::from_parts(parts, Body::from(bytes));
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(pinned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_url_normalizes() {
        assert_eq!(canonical_url("/"), "https://zaarhub.com/");
        assert_eq!(canonical_url("/apopka"), "https://zaarhub.com/apopka");
        assert_eq!(canonical_url("/apopka/"), "https://zaarhub.com/apopka");
        assert_eq!(
            canonical_url("/apopka/businesses?page=2"),
            "https://zaarhub.com/apopka/businesses?page=2"
        );
        assert_eq!(canonical_url(""), "https://zaarhub.com/");
    }

    #[test]
    fn host_rule() {
        assert!(is_canonical_host(Some("zaarhub.com")));
        assert!(is_canonical_host(Some("WWW.ZaarHub.com:443")));
        assert!(is_canonical_host(Some("directory.swiftsoftware.net")));
        assert!(!is_canonical_host(Some("palmbay.com")));
        assert!(!is_canonical_host(Some("127.0.0.1:8089")));
        assert!(!is_canonical_host(None));
        assert!(is_primary_host(Some("zaarhub.com")));
        assert!(!is_primary_host(Some("www.zaarhub.com")));
        assert!(!is_primary_host(Some("directory.swiftsoftware.net")));
    }

    #[test]
    fn inserts_when_absent() {
        let html = "<html><head><title>t</title></head><body></body></html>";
        let out = pin_html(html, "/portal");
        assert!(out.contains("<link rel=\"canonical\" href=\"https://zaarhub.com/portal\">"));
        assert!(out.contains("<meta property=\"og:url\" content=\"https://zaarhub.com/portal\">"));
        assert!(out.contains("<meta name=\"twitter:url\" content=\"https://zaarhub.com/portal\">"));
    }

    #[test]
    fn repoints_existing_host_derived_canonical() {
        let html = concat!(
            "<head><link rel=\"canonical\" href=\"https://directory.swiftsoftware.net/apopka\">",
            "<meta property=\"og:url\" content=\"https://directory.swiftsoftware.net/apopka\"></head>"
        );
        let out = pin_html(html, "/apopka");
        assert!(out.contains("href=\"https://zaarhub.com/apopka\""));
        assert!(!out.contains("directory.swiftsoftware.net"));
        // no duplicate tags
        assert_eq!(out.matches("rel=\"canonical\"").count(), 1);
        assert_eq!(out.matches("og:url").count(), 1);
    }

    #[test]
    fn keeps_a_deliberate_cross_path_canonical() {
        let html = "<head><link rel=\"canonical\" href=\"https://zaarhub.com/new-path\"></head>";
        let out = pin_html(html, "/legacy-alias");
        assert!(out.contains("href=\"https://zaarhub.com/new-path\""));
    }
}
