//! ONE canonical public host per directory — resolved from DATA, not from a constant.
//!
//! David's rule (2026-11-10): every directory the factory creates has its OWN brand
//! host (ZaarHub = `zaarhub.com`). That brand host is the only host that ranks; every
//! public page canonicalises absolutely to it no matter which host served the page;
//! only the brand host advertises a sitemap. The factory host
//! (`directory.swiftsoftware.net`) is the factory's home: public pages there are
//! `noindex` and it never advertises a sitemap, while `/admin*` and `/api/` are
//! untouched so the admin console keeps working.
//!
//! The ZaarHub fix pinned this rule with two constants. That is exactly the failure
//! this module removes: directory #2 would either inherit ZaarHub's origin or need
//! another code change. The brand host now lives where it belongs — in
//! `networks.root_domain` ([`BrandHosts`]) — read here, cached, refreshed in the
//! background.
//!
//! FAIL SAFE, NOT FAIL OPEN ([`Target::Unresolved`]): a directory with no brand host
//! set produces NO canonical pointing at somebody else's brand and NO advertised
//! sitemap, and its public pages on the factory host carry `X-Robots-Tag: noindex`.
//! Forgetting to fill the field must mean "not indexed", never "indexed on three
//! hostnames".

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use sqlx::PgPool;
use std::sync::{OnceLock, RwLock};

/// The FACTORY host: the directory-creation software and its admin console live
/// here. It never ranks (public paths are `noindex`) but must keep working.
pub const FACTORY_HOST: &str = "directory.swiftsoftware.net";

// ─────────────────────────────────────────────────────────────────────────────
// Brand hosts (data)
// ─────────────────────────────────────────────────────────────────────────────

/// The brand hosts of this deployment, read from `networks.root_domain`.
///
/// `networks` maps a network slug to its brand host; `directories` maps a directory
/// slug to the brand host of the network that owns it (that is how a path on the
/// factory host — `/<directory-slug>/…` — finds its own brand instead of a guess).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrandHosts {
    pub networks: Vec<(String, String)>,
    pub directories: Vec<(String, String)>,
}

impl BrandHosts {
    /// Every distinct brand host, sorted.
    pub fn brands(&self) -> Vec<String> {
        let mut out: Vec<String> = self.networks.iter().map(|(_, h)| h.clone()).collect();
        out.sort();
        out.dedup();
        out
    }

    /// The brand host of a network, by network slug.
    pub fn brand_for_network(&self, slug: &str) -> Option<&str> {
        self.networks
            .iter()
            .find(|(s, _)| s.eq_ignore_ascii_case(slug))
            .map(|(_, h)| h.as_str())
    }

    /// The brand host of a directory, by directory slug (through its network).
    pub fn brand_for_directory(&self, slug: &str) -> Option<&str> {
        self.directories
            .iter()
            .find(|(s, _)| s.eq_ignore_ascii_case(slug))
            .map(|(_, h)| h.as_str())
    }

    /// The brand host a request host names: exact, or `www.` of it.
    pub fn brand_of_host(&self, host: &str) -> Option<String> {
        let h = hostname(host);
        self.brands()
            .into_iter()
            .find(|b| h == *b || h == format!("www.{b}"))
    }
}

/// `https://ZaarHub.com/` and `zaarhub.com.` both normalise to `zaarhub.com`.
fn normalize_root_domain(raw: &str) -> Option<String> {
    let h = hostname(raw);
    let h = h.trim_start_matches("www.").to_string();
    if h.is_empty() || !h.contains('.') {
        return None;
    }
    Some(h)
}

/// The brand-host snapshot for this process.
static BRAND_HOSTS: OnceLock<RwLock<BrandHosts>> = OnceLock::new();

fn cell() -> &'static RwLock<BrandHosts> {
    BRAND_HOSTS.get_or_init(|| RwLock::new(BrandHosts::default()))
}

/// Current snapshot (cheap: a handful of rows).
pub fn brand_hosts() -> BrandHosts {
    cell()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|e| e.into_inner().clone())
}

/// Replace the snapshot — used at boot, by the refresher and by tests.
pub fn set_brand_hosts(next: BrandHosts) {
    let mut guard = cell().write().unwrap_or_else(|e| e.into_inner());
    *guard = next;
}

/// Read `networks.root_domain` (and the directory→network mapping) into a snapshot.
pub async fn load_brand_hosts(pool: &PgPool) -> Result<BrandHosts, sqlx::Error> {
    let networks: Vec<(String, String)> = sqlx::query_as(
        "SELECT slug, root_domain FROM networks \
         WHERE root_domain IS NOT NULL AND btrim(root_domain) <> ''",
    )
    .fetch_all(pool)
    .await?;
    let directories: Vec<(String, String)> = sqlx::query_as(
        "SELECT d.slug, n.root_domain FROM directories d \
         JOIN networks n ON n.id = d.network_id \
         WHERE n.root_domain IS NOT NULL AND btrim(n.root_domain) <> ''",
    )
    .fetch_all(pool)
    .await?;

    Ok(BrandHosts {
        networks: networks
            .into_iter()
            .filter_map(|(slug, root)| normalize_root_domain(&root).map(|h| (slug, h)))
            .collect(),
        directories: directories
            .into_iter()
            .filter_map(|(slug, root)| normalize_root_domain(&root).map(|h| (slug, h)))
            .collect(),
    })
}

/// Re-read the brand hosts and publish the snapshot. Returns the network row count.
pub async fn refresh_brand_hosts(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let next = load_brand_hosts(pool).await?;
    let n = next.networks.len();
    set_brand_hosts(next);
    Ok(n)
}

/// Keep [`brand_hosts`] current: refresh at once, then every 60 s, logging only when
/// the set changes (a directory gaining its brand host is the interesting event).
pub fn start_brand_host_refresh(pool: PgPool) {
    tokio::spawn(async move {
        let mut last: Option<BrandHosts> = None;
        loop {
            match load_brand_hosts(&pool).await {
                Ok(next) => {
                    if last.as_ref() != Some(&next) {
                        tracing::info!(
                            "brand hosts: {:?} ({} network row(s), {} directory row(s))",
                            next.brands(),
                            next.networks.len(),
                            next.directories.len()
                        );
                        last = Some(next.clone());
                    }
                    set_brand_hosts(next);
                }
                Err(e) => tracing::warn!("brand hosts: refresh failed: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Host / target resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Lower-cased hostname without scheme, port, path or trailing dot.
fn hostname(host: &str) -> String {
    let h = host.trim();
    let h = h.split("://").last().unwrap_or(h);
    let h = h.split('/').next().unwrap_or(h);
    let h = h.split(':').next().unwrap_or(h);
    h.trim_matches('.').to_ascii_lowercase()
}

fn is_private_host(h: &str) -> bool {
    h == "localhost"
        || h.starts_with("127.")
        || h.starts_with("192.168.")
        || h.starts_with("10.")
        || h.starts_with("172.")
}

/// Which origin a request's public pages must canonicalise to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A brand host (the host itself, `www.` of it, or a path on the factory host
    /// that resolves to the owning directory's brand). Canonicalise to
    /// `https://<host>`.
    Brand(String),
    /// The host is its own origin: a directory mapped to its own domain, or a dev /
    /// loopback host used by the probe harness. Behaviour unchanged from before.
    Own,
    /// A public host with no brand host in data: FAIL SAFE — no canonical (never
    /// another directory's brand) and no advertised sitemap; the factory host also
    /// answers `X-Robots-Tag: noindex` on public paths.
    Unresolved,
}

/// Resolve the canonical target for a request. Pure, so the fail-safe is unit-tested.
pub fn resolve(host: Option<&str>, path: &str, brands: &BrandHosts) -> Target {
    let raw = match host {
        Some(h) if !h.trim().is_empty() => h,
        _ => return Target::Own,
    };
    let h = hostname(raw);
    if h.is_empty() || is_private_host(&h) {
        return Target::Own;
    }

    // A brand host (or its `www`) names the directory it belongs to — in data.
    if let Some(brand) = brands.brand_of_host(&h) {
        return Target::Brand(brand);
    }

    if h == FACTORY_HOST || h == format!("www.{FACTORY_HOST}") {
        // The factory host serves public pages for whichever directory the path
        // names: `/<directory-slug>/…` or `/<network-slug>/…`. That is the only way
        // a page here reaches the right brand once a second directory exists.
        if let Some(seg) = path.trim_matches('/').split('/').next() {
            if !seg.is_empty() {
                if let Some(brand) = brands.brand_for_directory(seg) {
                    return Target::Brand(brand.to_string());
                }
                if let Some(brand) = brands.brand_for_network(seg) {
                    return Target::Brand(brand.to_string());
                }
            }
        }
        // Root and un-prefixed paths: only a single-directory deployment has one
        // unambiguous answer. With two brands it is Unresolved — never a guess.
        let all = brands.brands();
        if all.len() == 1 {
            return Target::Brand(all[0].clone());
        }
        return Target::Unresolved;
    }

    // Unknown host: a directory mapped to its own domain, or a dev host. Keeps its
    // own origin, exactly as before.
    Target::Own
}

/// `Some("https://<brand>")` for a host that names a brand (host-only callers such
/// as the SSR url builders); `None` means "keep the request host".
pub fn brand_origin_for_host(host: Option<&str>) -> Option<String> {
    match resolve(host, "", &brand_hosts()) {
        Target::Brand(b) => Some(format!("https://{b}")),
        _ => None,
    }
}

/// The ONE host allowed to advertise a sitemap: an exact brand host, in data.
pub fn sitemap_host(host: Option<&str>, brands: &BrandHosts) -> Option<String> {
    let h = hostname(host?);
    if brands.brands().iter().any(|b| *b == h) {
        Some(h)
    } else {
        None
    }
}

/// `Some("https://<brand-host>")` when this request host may advertise a sitemap.
pub fn sitemap_origin(host: Option<&str>) -> Option<String> {
    sitemap_host(host, &brand_hosts()).map(|h| format!("https://{h}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// URL + document rewriting
// ─────────────────────────────────────────────────────────────────────────────

/// `<origin>` + a normalized path (query preserved, trailing slash dropped except
/// at the root, so `/apopka/` and `/apopka` are one URL).
pub fn canonical_url(origin: &str, path_and_query: &str) -> String {
    let origin = origin.trim_end_matches('/');
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
        Some(q) if !q.is_empty() => format!("{}{}?{}", origin, p, q),
        _ => format!("{}{}", origin, p),
    }
}

/// Re-point an existing absolute or root-relative url at `origin`, keeping its path
/// and query exactly (a page whose canonical deliberately names another path — a
/// legacy alias canonicalising to its new url — keeps that path).
fn pin_url(origin: &str, existing: &str) -> String {
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
    let origin = origin.trim_end_matches('/');
    let mut out = String::with_capacity(origin.len() + rest.len());
    out.push_str(origin);
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

/// The `<…>` element containing `marker`, as (start, end_exclusive) byte offsets.
fn tag_span(html: &str, marker: &str) -> Option<(usize, usize)> {
    let m = find_ci(html, marker)?;
    let start = html[..m].rfind('<')?;
    let end = start + html[start..].find('>')? + 1;
    Some((start, end))
}

/// Rewrite the `attr` value of the first `<tag_name … marker …>` element in
/// `html`. Returns `None` when no such element (or no such attribute) exists, so
/// the caller can decide to insert one instead.
fn rewrite_tag_attr(
    html: &str,
    tag_name: &str,
    marker: &str,
    attr: &str,
    origin: &str,
) -> Option<String> {
    let (tag_start, tag_end) = tag_span(html, marker)?;
    let tag = &html[tag_start..tag_end];
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
    let pinned = pin_url(origin, &html[abs_vs..abs_ve]);
    let mut out = String::with_capacity(html.len() + pinned.len());
    out.push_str(&html[..abs_vs]);
    out.push_str(&pinned);
    out.push_str(&html[abs_ve..]);
    Some(out)
}

/// The canonical signals this module owns: (tag, marker, attribute).
const CANONICAL_TAGS: &[(&str, &str, &str)] = &[
    ("link", "rel=\"canonical\"", "href"),
    ("link", "rel='canonical'", "href"),
    ("meta", "property=\"og:url\"", "content"),
    ("meta", "property='og:url'", "content"),
    ("meta", "name=\"twitter:url\"", "content"),
    ("meta", "name='twitter:url'", "content"),
];

/// Pin every canonical signal in one HTML document to `origin` (absolute, never the
/// serving host). Replaces an existing `<link rel="canonical">`, `<meta
/// property="og:url">` and `<meta name="twitter:url">` (origin only — the path is
/// preserved), and inserts the three tags when the document has none, which is the
/// case for the SPA shell (`/`) and the business portal (`/portal`).
pub fn pin_html(html: &str, path_and_query: &str, origin: &str) -> String {
    let mut out = html.to_string();
    let url = canonical_url(origin, path_and_query);

    for (tag, marker, attr) in CANONICAL_TAGS {
        if let Some(next) = rewrite_tag_attr(&out, tag, marker, attr, origin) {
            out = next;
        }
    }

    let has = |needle: &str| find_ci(&out, needle).is_some();
    let mut block = String::new();
    if !has("rel=\"canonical\"") && !has("rel='canonical'") {
        block.push_str(&format!("<link rel=\"canonical\" href=\"{url}\">"));
    }
    if !has("property=\"og:url\"") && !has("property='og:url'") {
        block.push_str(&format!("<meta property=\"og:url\" content=\"{url}\">"));
    }
    if !has("name=\"twitter:url\"") && !has("name='twitter:url'") {
        block.push_str(&format!("<meta name=\"twitter:url\" content=\"{url}\">"));
    }
    if block.is_empty() {
        return out;
    }
    if let Some(pos) = find_ci(&out, "</head>") {
        out.insert_str(pos, &block);
    }
    out
}

/// Remove every canonical signal from a document — the FAIL-SAFE shape: a public
/// page with no brand host set must not point a canonical at any host (least of all
/// another directory's brand).
pub fn strip_canonical(html: &str) -> String {
    let mut out = html.to_string();
    for (_, marker, _) in CANONICAL_TAGS {
        while let Some((start, end)) = tag_span(&out, marker) {
            out.replace_range(start..end, "");
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Response middleware
// ─────────────────────────────────────────────────────────────────────────────

/// Paths that are account/claim surfaces sitting on the PUBLIC directory host but
/// carrying no content of their own — Google already listed them under "Crawled -
/// currently not indexed" (CEO board, 2026-11-10). They answer `X-Robots-Tag:
/// noindex` so they stop competing; `/claim`, the `/zaarhub/<city>/<listing>` pages
/// and `/guides/participant-guide.html` stay indexable because they ARE content.
const NOINDEX_PATHS: &[&str] = &["/visitor", "/visitor/", "/zaarhub-offer.html"];

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

/// Response middleware: point every public HTML page at its OWN directory's brand
/// host (whatever host served it), keep the factory's public pages out of the index,
/// and fail safe when the brand host has not been set.
pub async fn seo_headers(req: Request, next: Next) -> Response {
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
    let noindex = NOINDEX_PATHS.contains(&path.as_str());

    let brands = brand_hosts();
    let target = resolve(host.as_deref(), &path, &brands);
    let public_path = !excluded(&path);
    let on_factory = matches!(host.as_deref(), Some(h) if hostname(h) == FACTORY_HOST);
    let factory_public = on_factory && public_path;
    // FAIL SAFE: a public path on a host whose directory has no brand host in data
    // gets no canonical and no sitemap — and no index at all on the factory host.
    let unresolved_public = matches!(target, Target::Unresolved) && public_path;

    let mut res = next.run(req).await;

    // The FACTORY host is never the indexed copy — the brand host is (David,
    // 2026-11-10). Applied to every public response, including a 404, and never to
    // `/admin*` or `/api/`: the factory's own console has to keep working, and the
    // admin SPA's requests must look exactly as they did before.
    if factory_public || unresolved_public {
        res.headers_mut().append(
            "x-robots-tag",
            header::HeaderValue::from_static("noindex, follow"),
        );
    }

    // Account/claim surfaces: noindex at the response level, so the directive holds
    // even for a crawler that reaches the page through a link.
    if noindex {
        res.headers_mut()
            .insert("x-robots-tag", header::HeaderValue::from_static("noindex"));
        return res;
    }

    if !is_get || !public_path || matches!(target, Target::Own) {
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
    let rewritten = match &target {
        Target::Brand(brand) => pin_html(&html, &path_and_query, &format!("https://{brand}")),
        Target::Unresolved => strip_canonical(&html),
        Target::Own => return Response::from_parts(parts, Body::from(bytes)),
    };
    if rewritten == html {
        return Response::from_parts(parts, Body::from(bytes));
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(rewritten))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zaarhub_only() -> BrandHosts {
        BrandHosts {
            networks: vec![("zaarhub".into(), "zaarhub.com".into())],
            directories: vec![
                ("apopka".into(), "zaarhub.com".into()),
                ("palm-bay".into(), "zaarhub.com".into()),
            ],
        }
    }

    /// Two directories, each with its own brand host — the state the factory is in
    /// the moment directory #2 launches.
    fn two_brands() -> BrandHosts {
        BrandHosts {
            networks: vec![
                ("zaarhub".into(), "zaarhub.com".into()),
                ("swiftlist".into(), "swiftlist.com".into()),
            ],
            directories: vec![
                ("apopka".into(), "zaarhub.com".into()),
                ("palm-bay".into(), "zaarhub.com".into()),
                ("naples".into(), "swiftlist.com".into()),
            ],
        }
    }

    #[test]
    fn canonical_url_normalizes() {
        assert_eq!(
            canonical_url("https://zaarhub.com", "/"),
            "https://zaarhub.com/"
        );
        assert_eq!(
            canonical_url("https://zaarhub.com/", "/apopka"),
            "https://zaarhub.com/apopka"
        );
        assert_eq!(
            canonical_url("https://zaarhub.com", "/apopka/"),
            "https://zaarhub.com/apopka"
        );
        assert_eq!(
            canonical_url("https://zaarhub.com", "/apopka/businesses?page=2"),
            "https://zaarhub.com/apopka/businesses?page=2"
        );
        assert_eq!(
            canonical_url("https://zaarhub.com", ""),
            "https://zaarhub.com/"
        );
    }

    #[test]
    fn brand_hosts_resolve_to_their_own_brand() {
        let b = zaarhub_only();
        // The brand host, its www, and the factory host all name the same brand.
        assert_eq!(
            resolve(Some("zaarhub.com"), "/apopka", &b),
            Target::Brand("zaarhub.com".into())
        );
        assert_eq!(
            resolve(Some("WWW.ZaarHub.com:443"), "/apopka", &b),
            Target::Brand("zaarhub.com".into())
        );
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/apopka", &b),
            Target::Brand("zaarhub.com".into())
        );
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/", &b),
            Target::Brand("zaarhub.com".into())
        );
        // Mapped domains and dev hosts keep their own origin.
        assert_eq!(resolve(Some("palmbay.com"), "/apopka", &b), Target::Own);
        assert_eq!(resolve(Some("127.0.0.1:8089"), "/apopka", &b), Target::Own);
        assert_eq!(resolve(None, "/apopka", &b), Target::Own);
    }

    #[test]
    fn directory_two_never_inherits_another_brands_host() {
        let b = two_brands();
        // Each brand host is its own.
        assert_eq!(
            resolve(Some("swiftlist.com"), "/naples", &b),
            Target::Brand("swiftlist.com".into())
        );
        // On the factory host, the path decides — never a default.
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/naples", &b),
            Target::Brand("swiftlist.com".into())
        );
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/apopka", &b),
            Target::Brand("zaarhub.com".into())
        );
        // Ambiguous path on the factory host: fail safe, not a guess.
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/", &b),
            Target::Unresolved
        );
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/unknown-thing", &b),
            Target::Unresolved
        );
    }

    #[test]
    fn no_brand_host_is_fail_safe() {
        let none = BrandHosts::default();
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/apopka", &none),
            Target::Unresolved
        );
        assert_eq!(
            resolve(Some("directory.swiftsoftware.net"), "/", &none),
            Target::Unresolved
        );
        // No host advertises a sitemap when no brand host is set.
        assert_eq!(
            sitemap_host(Some("directory.swiftsoftware.net"), &none),
            None
        );
        assert_eq!(sitemap_host(Some("zaarhub.com"), &none), None);
        // …and no canonical is emitted at all.
        let html =
            "<head><link rel=\"canonical\" href=\"https://directory.swiftsoftware.net/apopka\">\
                    <meta property=\"og:url\" content=\"https://zaarhub.com/apopka\"></head>";
        let out = strip_canonical(html);
        assert!(!out.contains("canonical"));
        assert!(!out.contains("og:url"));
    }

    #[test]
    fn only_the_brand_host_advertises_a_sitemap() {
        let b = zaarhub_only();
        assert_eq!(
            sitemap_host(Some("zaarhub.com"), &b),
            Some("zaarhub.com".to_string())
        );
        assert_eq!(sitemap_host(Some("www.zaarhub.com"), &b), None);
        assert_eq!(sitemap_host(Some("directory.swiftsoftware.net"), &b), None);
        assert_eq!(sitemap_host(Some("127.0.0.1"), &b), None);
        assert_eq!(sitemap_host(None, &b), None);
    }

    #[test]
    fn inserts_when_absent() {
        let html = "<html><head><title>t</title></head><body></body></html>";
        let out = pin_html(html, "/portal", "https://zaarhub.com");
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
        let out = pin_html(html, "/apopka", "https://zaarhub.com");
        assert!(out.contains("href=\"https://zaarhub.com/apopka\""));
        assert!(!out.contains("directory.swiftsoftware.net"));
        // no duplicate tags
        assert_eq!(out.matches("rel=\"canonical\"").count(), 1);
        assert_eq!(out.matches("og:url").count(), 1);
    }

    #[test]
    fn pins_to_the_directories_own_brand_not_zaarhub() {
        let html = "<head><link rel=\"canonical\" href=\"https://zaarhub.com/naples\"></head>";
        let out = pin_html(html, "/naples", "https://swiftlist.com");
        assert!(out.contains("href=\"https://swiftlist.com/naples\""));
        assert!(!out.contains("zaarhub.com"));
    }

    #[test]
    fn keeps_a_deliberate_cross_path_canonical() {
        let html = "<head><link rel=\"canonical\" href=\"https://zaarhub.com/new-path\"></head>";
        let out = pin_html(html, "/legacy-alias", "https://zaarhub.com");
        assert!(out.contains("href=\"https://zaarhub.com/new-path\""));
    }
}
