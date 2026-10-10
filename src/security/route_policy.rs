//! Default-deny routing for Multi-Directory (kanban t_8bfbbf7f).
//!
//! # The rule
//!
//! **A mounted route is PRIVATE unless it appears in [`PUBLIC_ROUTES`].**
//!
//! Before this module the policy was a ~210-line inline `is_public` predicate inside
//! `routes::auth_guard`: it could only be read by whoever scrolled to the middle of a 3,800-line
//! file, nothing tested it, and each clause was a *path test* rather than a statement about a
//! route. It also carried clauses that no route matched (dead policy is policy nobody trusts) and
//! several clauses broad enough to cover routes that must never be anonymous.
//!
//! This module is the committed list. `auth_guard` (mounted on every `/api/v1` route) and the
//! outer SSR router both ask it; a route that is not on the list must present a credential.
//!
//! # Where the list came from (measured 2026-10-06, commit 5c915cd)
//!
//! The census is mechanical, not editorial: 652 mounted `.route(..)` calls were extracted from
//! `src/routes.rs` (634 inside `nest("/api/v1", ..)`, 18 on the outer SSR router), every route
//! was classified by transcribing the old predicate clause for clause, and the result was
//! cross-checked against a live anonymous GET sweep of the running binary
//! (`/opt/swift/audits/t_8bfbbf7f/anon-before.json`: 374 GET legs, 86 answering an anonymous
//! caller 200). 163 of the 865 method legs were public; 13 of them are **not** public here:
//!
//! ```text
//! POST /spotlight/:id/feature                     anonymous write of a commercial placement flag
//!                                                 (`zaarhub::toggle_spotlight_featured` reads no
//!                                                 credential at all; no caller in the front-end)
//! POST /pipeline/ingest                           anonymous bulk INSERT/UPDATE of businesses
//!                                                 into any directory (`pipeline::pipeline_ingest`
//!                                                 reads no credential; `admin-ops.html` sends a
//!                                                 bearer token with every call)
//! POST /businesses/:id/images                     anonymous base64 file upload onto ANY business
//!                                                 (`upload_business_images` never checks the
//!                                                 caller; `business-portal.html` sends a token)
//! POST /directories/:slug/subscribers/import      anonymous bulk newsletter INSERT into any
//!                                                 directory (admin-panel action)
//! POST /bookings/:id/status                       the handler already requires a session
//! POST /bookings/:id/cancel                       the handler already requires a session
//! POST /events/:id/edit                           the handler already requires a session
//! GET  /zaarhub/claims/:visitor_id                the handler already requires the owner
//! GET  /b2b/rfqs/my                               supplier-scoped
//! GET  /b2b/rfqs/:id/messages                     "private to the poster and its bidders, so
//!                                                 they sit behind auth_guard" (src/routes.rs) --
//!                                                 the old public clause defeated that guard
//! GET  /b2b/rfqs/:id/bids                         bidders only
//! GET  /b2b/products/my                           supplier-scoped
//! GET  /b2b/products/export                       supplier-scoped
//! ```
//!
//! The last nine change nothing observable for an anonymous caller (the handlers refuse them
//! today, measured: they answer 401); they become middleware-private so a handler refactor can
//! never quietly expose them. The first four were answering anonymous callers 200 or 4xx-only-on-
//! bad-input and are the actual behaviour change this card ships.
//!
//! Dead clauses dropped: the old predicate's `/api/v1/public/`, `/api/v1/places/*` and
//! `/api/v1/zaarhub/*` entries can never match (axum's `nest` strips the prefix, proved live from
//! the container log -- `AUTH_GUARD: path=/health`), and `/notifications/` was a prefix with no
//! route under it. They are gone rather than carried forward: a clause with no route is a clause
//! that will one day be a hole.
//!
//! # Adding a route
//!
//! Leave it out of [`PUBLIC_ROUTES`] and it is private. Do NOT add an entry unless the route must
//! answer a caller with no credential -- and then add a test leg for it in this module's test
//! block so the decision is recorded with its reason.
//!
//! # Matching
//!
//! Templates use axum's spelling: `:param` matches exactly one non-empty segment, `*rest`
//! matches a non-empty remainder. Matching is per SEGMENT, so `/play/:id` accepts
//! `/play/anything` but not `/play/a/b`. A `GET` entry also admits `HEAD` (axum answers HEAD from
//! a `get(..)` handler, and monitors use it).

use axum::http::{HeaderMap, Method};

/// One committed allowlist entry: `(method, axum path template, why)`. Keeping the reason in the
/// list is deliberate -- an entry whose reason cannot be stated is an entry that should not be
/// there. The method is `"GET"`, `"POST"`, `"PUT"`, `"DELETE"`, `"PATCH"`, or `"*"` for any.
pub type PublicRoute = (&'static str, &'static str, &'static str);

fn entry_matches(method: &Method, path: &str, entry: &PublicRoute) -> bool {
    let (m, template, _why) = *entry;
    let method_ok = m == "*"
        || m == method.as_str()
        // axum answers HEAD from a `get(..)` handler, and monitors use it.
        || (m == "GET" && method == Method::HEAD);
    method_ok && template_matches(template, path)
}

/// Routes the internal-key arm admits: a sibling service or cron presents the fleet's shared key
/// instead of a session. Everything here is still refused an anonymous caller.
pub const INTERNAL_KEY_PATHS: &[&str] = &["/cron/"];

/// The header `routes::cron_internal_key_ok` reads.
pub const INTERNAL_KEY_HEADER: &str = "x-internal-key";

/// Routes that may be reached with NO credential at all.
pub const PUBLIC_ROUTES: &[PublicRoute] = &[
    // --- /ads --------------------------------------------------------
    ("GET", "/ads/active/:directory_id", "ads"),
    // --- /api-keys ---------------------------------------------------
    // The endpoint's own credential IS the API key (read from the Authorization header), so it
    // cannot also require a session JWT — that made it uncallable (401 for every caller). The
    // hash-only lookup below means only the full key verifies; the prefix alone never does.
    ("POST", "/api-keys/verify", "api-key-verify"),
    // --- /auth -------------------------------------------------------
    ("GET", "/auth/linked-accounts", "linked-accounts"),
    ("POST", "/auth/forgot-password", "auth"),
    ("POST", "/auth/login", "auth"),
    ("POST", "/auth/register", "auth"),
    ("POST", "/auth/reset-password", "auth"),
    ("POST", "/auth/switch-role", "switch-role"),
    // --- /b2b --------------------------------------------------------
    ("GET", "/b2b/co-op/deals/active", "coop-deals"),
    ("GET", "/b2b/co-op/groups", "coop-groups"),
    ("GET", "/b2b/co-op/groups/:id", "coop-group-id"),
    ("GET", "/b2b/discover", "b2b-discover"),
    ("GET", "/b2b/leads/available", "b2b-leads"),
    ("GET", "/b2b/marketplace", "b2b-marketplace"),
    ("GET", "/b2b/products", "b2b-products"),
    ("GET", "/b2b/products/:id", "b2b-product-id"),
    ("GET", "/b2b/rfqs", "rfq-list"),
    ("GET", "/b2b/rfqs/:id", "rfq-id"),
    ("GET", "/b2b/rfqs/stats", "rfq-stats"),
    ("GET", "/b2b/suppliers", "b2b-suppliers"),
    ("GET", "/b2b/suppliers/:id/detail", "b2b-supplier-detail"),
    ("POST", "/b2b/register", "b2b-register"),
    // --- /blog -------------------------------------------------------
    ("GET", "/blog/internal-links/suggestions", "suggestions"),
    // --- /book -------------------------------------------------------
    ("GET", "/book/:slug/:business_id", "booking-page"),
    // --- /bookings ---------------------------------------------------
    ("POST", "/bookings", "booking-create"),
    // --- /bookmarks --------------------------------------------------
    ("GET", "/bookmarks", "bookmarks"),
    ("GET", "/bookmarks/count/:business_id", "bookmarks-count"),
    ("POST", "/bookmarks/toggle", "bookmarks-toggle"),
    // --- /business-articles ------------------------------------------
    ("POST", "/business-articles/:id/track", "article-track"),
    // --- /businesses -------------------------------------------------
    ("GET", "/businesses/:id/questions", "business-questions"),
    ("POST", "/businesses/:id/claim", "business-claim"),
    ("POST", "/businesses/:id/questions", "business-questions"),
    ("POST", "/businesses/:id/report", "business-report"),
    // --- /categories -------------------------------------------------
    ("GET", "/categories", "listings"),
    // --- /checkout ---------------------------------------------------
    ("GET", "/checkout/session/:id", "checkout-session"),
    // --- /city-requests ----------------------------------------------
    ("GET", "/city-requests", "city-requests"),
    ("POST", "/city-requests", "city-requests"),
    // --- /claims -----------------------------------------------------
    ("GET", "/claims/verify/:token", "claim-verify"),
    // --- /community --------------------------------------------------
    ("GET", "/community/posts", "community-read"),
    ("GET", "/community/posts/:id", "community-read-id"),
    // --- /coop-hub ---------------------------------------------------
    ("GET", "/coop-hub", "outer-ssr"),
    ("GET", "/coop-hub/", "outer-ssr"),
    // --- /d ----------------------------------------------------------
    ("GET", "/d/:slug/blog", "d-prefix"),
    ("GET", "/d/:slug/blog/:post_slug", "d-prefix"),
    // --- /deals ------------------------------------------------------
    ("GET", "/deals", "deals-list"),
    ("GET", "/deals/:id", "deal-detail"),
    ("GET", "/deals/:id/page", "deal-page"),
    ("GET", "/deals/featured", "deals-featured"),
    ("GET", "/deals/redemptions/code/:code", "deal-code-lookup"),
    ("POST", "/deals/:id/claim", "deal-claim"),
    ("POST", "/deals/:id/redeem", "deal-redeem"),
    // --- /directories ------------------------------------------------
    ("GET", "/directories/:id/features", "ends-features"),
    ("GET", "/directories/:id/topics/suggestions", "suggestions"),
    (
        "GET",
        "/directories/:slug/businesses",
        "public-city-business-list",
    ),
    (
        "GET",
        "/directories/:slug/businesses/:business_id/available-slots",
        "available-slots",
    ),
    (
        "GET",
        "/directories/:slug/businesses/suggestions",
        "suggestions",
    ),
    (
        "POST",
        "/directories/:slug/businesses/:business_id/book",
        "booking-create",
    ),
    (
        "POST",
        "/directories/:slug/subscribers",
        "newsletter-signup",
    ),
    (
        "POST",
        "/directories/:slug/subscribers/:id/unsubscribe",
        "newsletter-signup",
    ),
    ("POST", "/directories/newsletter", "newsletter-signup"),
    // --- /directory --------------------------------------------------
    (
        "GET",
        "/directory/:slug/businesses",
        "public-city-business-list",
    ),
    // --- /events -----------------------------------------------------
    ("GET", "/events", "events-list"),
    ("GET", "/events/:id", "events-id"),
    ("POST", "/events/:id/cancel", "events-cancel"),
    ("POST", "/events/:id/rsvp", "events-rsvp"),
    // --- /events-page ------------------------------------------------
    ("GET", "/events-page", "events-page"),
    // --- /feed -------------------------------------------------------
    ("GET", "/feed", "feed"),
    // --- /feed-page --------------------------------------------------
    ("GET", "/feed-page", "feed-page"),
    // --- /health -----------------------------------------------------
    ("GET", "/health", "health"),
    // --- /homepage ---------------------------------------------------
    ("GET", "/homepage/config", "homepage-config"),
    // --- /indexnow-key.txt -------------------------------------------
    // B142 — the IndexNow ownership key file, served at the host ROOT (IndexNow
    // refuses a nested keyLocation). Fetched unauthenticated by the engine; returns
    // 404 when no key is set, so it can expose nothing but the public key.
    ("GET", "/indexnow-key.txt", "indexnow-key"),
    // --- /l ----------------------------------------------------------
    ("GET", "/l/:short_code", "outer-ssr"),
    // --- /lead-exchange ----------------------------------------------
    ("GET", "/lead-exchange", "outer-ssr"),
    ("GET", "/lead-exchange/", "outer-ssr"),
    // --- /legal ------------------------------------------------------
    ("GET", "/legal/:slug", "outer-ssr"),
    // --- /listing-invites --------------------------------------------
    ("GET", "/listing-invites/:token", "listing-invite-token"),
    (
        "POST",
        "/listing-invites/:token/complete",
        "listing-invite-token",
    ),
    // --- /listings ---------------------------------------------------
    ("GET", "/listings", "listings"),
    // --- /loyalty ----------------------------------------------------
    ("GET", "/loyalty/messaging", "loyalty-messaging"),
    // --- /messages ---------------------------------------------------
    ("POST", "/messages/:business_id", "guest-message"),
    // --- /my-bookings ------------------------------------------------
    ("GET", "/my-bookings", "my-bookings"),
    // --- /notifications ----------------------------------------------
    ("GET", "/notifications/:directory_id", "notifications"),
    // --- /p ----------------------------------------------------------
    ("GET", "/p/:slug", "outer-ssr"),
    // --- /places -----------------------------------------------------
    ("GET", "/places/autocomplete", "places"),
    ("GET", "/places/details", "places"),
    // --- /polls ------------------------------------------------------
    ("GET", "/polls", "polls-list"),
    ("GET", "/polls/:id", "polls-id"),
    ("POST", "/polls/:id/close", "poll-close"),
    ("POST", "/polls/:id/vote", "poll-vote"),
    // --- /pricing ----------------------------------------------------
    ("GET", "/pricing/public", "pricing"),
    // --- /programmatic-pages -----------------------------------------
    (
        "POST",
        "/programmatic-pages/:page_id/track",
        "programmatic-track",
    ),
    // --- /public -----------------------------------------------------
    ("GET", "/public/:slug", "public-prefix"),
    ("GET", "/public/:slug/:business_id", "public-prefix"),
    (
        "GET",
        "/public/directories/:slug/articles.xml",
        "public-prefix",
    ),
    (
        "GET",
        "/public/directories/:slug/blog/feed.xml",
        "public-prefix",
    ),
    (
        "GET",
        "/public/directories/:slug/news-sitemap.xml",
        "public-prefix",
    ),
    ("GET", "/public/directories/:slug/survey", "public-prefix"),
    ("GET", "/public/homepage", "public-prefix"),
    ("GET", "/public/og/:page_type/:page_id", "outer-ssr"),
    ("GET", "/public/onboarding", "public-prefix"),
    (
        "POST",
        "/public/directories/:slug/survey/respond",
        "public-prefix",
    ),
    // --- /quotes -----------------------------------------------------
    ("POST", "/quotes/broadcast", "multi-quote"),
    // --- /reviews ----------------------------------------------------
    ("GET", "/reviews", "reviews-read"),
    ("GET", "/reviews/stats/:business_id", "reviews-stats"),
    // --- /rfq-marketplace --------------------------------------------
    ("GET", "/rfq-marketplace", "outer-ssr"),
    ("GET", "/rfq-marketplace/", "outer-ssr"),
    // --- /robots.txt -------------------------------------------------
    ("GET", "/robots.txt", "outer-ssr"),
    // --- /saved-places -----------------------------------------------
    ("GET", "/saved-places", "saved-places"),
    // --- /scraper ----------------------------------------------------
    ("GET", "/scraper/providers", "scraper-providers"),
    // --- /search -----------------------------------------------------
    ("GET", "/search", "listings"),
    // --- /:slug/blog/feed.xml ----------------------------------------
    // The blog RSS feed, served at the pretty `<origin>/<dir-slug>/blog/feed.xml`
    // url the feed itself advertises (channel link + atom self). Public: it is an
    // RSS document, the same content the `/public/directories/:slug/blog/feed.xml`
    // arm serves (added 2026-10-10; the pretty path used to 404 as a "missing asset").
    ("GET", "/:slug/blog/feed.xml", "outer-ssr"),
    // --- /sitemap.xml ------------------------------------------------
    ("GET", "/sitemap.xml", "outer-ssr"),
    // --- /spotlight --------------------------------------------------
    ("GET", "/spotlight/:directory_id", "spotlight"),
    // --- /submissions ------------------------------------------------
    ("POST", "/submissions", "public-submission"),
    // --- /subscriptions ----------------------------------------------
    ("GET", "/subscriptions/features", "plans"),
    ("GET", "/subscriptions/plans", "plans"),
    // --- /uploads ----------------------------------------------------
    ("GET", "/uploads/*path", "outer-ssr"),
    // --- /visitor ----------------------------------------------------
    ("GET", "/visitor/favorites", "favorites"),
    (
        "GET",
        "/visitor/favorites/check/:business_id",
        "favorites-check",
    ),
    ("GET", "/visitor/follows", "follows"),
    (
        "GET",
        "/visitor/follows/check/:business_id",
        "follows-check",
    ),
    ("GET", "/visitor/recommendations", "rec-list"),
    (
        "GET",
        "/visitor/recommendations/check/:business_id",
        "rec-check",
    ),
    (
        "POST",
        "/visitor/favorites/:business_id",
        "favorites-toggle",
    ),
    ("POST", "/visitor/follows/:business_id", "follows-toggle"),
    ("POST", "/visitor/login", "visitor-auth"),
    ("POST", "/visitor/recommendations/:business_id", "rec-post"),
    ("POST", "/visitor/register", "visitor-auth"),
    // --- /visitors ---------------------------------------------------
    ("POST", "/visitors/event", "visitor-beacon"),
    ("POST", "/visitors/page-view", "visitor-beacon"),
    ("POST", "/visitors/session/:id/end", "visitor-session-end"),
    ("POST", "/visitors/track", "visitor-beacon"),
    // --- /webhooks ---------------------------------------------------
    ("POST", "/webhooks/paypal", "payment-webhook"),
    ("POST", "/webhooks/stripe", "payment-webhook"),
    // --- /zaarhub ----------------------------------------------------
    ("GET", "/zaarhub", "outer-ssr"),
    ("GET", "/zaarhub/:slug", "outer-ssr"),
    ("GET", "/zaarhub/:slug/:id", "outer-ssr"),
    ("GET", "/zaarhub/activity", "zaarhub-public"),
    ("GET", "/zaarhub/analytics/categories", "zaarhub-public"),
    ("GET", "/zaarhub/analytics/cities", "zaarhub-public"),
    ("GET", "/zaarhub/analytics/claims", "zaarhub-public"),
    ("GET", "/zaarhub/analytics/offers", "zaarhub-public"),
    ("GET", "/zaarhub/analytics/overview", "zaarhub-public"),
    ("GET", "/zaarhub/business/:slug/:id", "zaarhub-public"),
    ("GET", "/zaarhub/categories", "zaarhub-public"),
    ("GET", "/zaarhub/cities", "zaarhub-public"),
    ("GET", "/zaarhub/cities/:slug", "zaarhub-public"),
    ("GET", "/zaarhub/cities/:slug/blog-posts", "zaarhub-public"),
    ("GET", "/zaarhub/cities/:slug/listings", "zaarhub-public"),
    ("GET", "/zaarhub/deals", "zaarhub-public"),
    ("GET", "/zaarhub/editors-picks", "zaarhub-public"),
    ("GET", "/zaarhub/events", "zaarhub-public"),
    ("GET", "/zaarhub/featured", "zaarhub-public"),
    ("GET", "/zaarhub/homepage", "zaarhub-public"),
    ("GET", "/zaarhub/listings/:id", "zaarhub-public"),
    ("GET", "/zaarhub/listings/:id/offers", "zaarhub-public"),
    ("GET", "/zaarhub/offers/:id", "zaarhub-public"),
    ("GET", "/zaarhub/search", "zaarhub-public"),
    ("POST", "/zaarhub/offers/:id/claim", "zaarhub-public"),
    // --- /zaarhub-robots.txt -----------------------------------------
    ("GET", "/zaarhub-robots.txt", "outer-ssr"),
    // --- /zaarhub-sitemap.xml ----------------------------------------
    ("GET", "/zaarhub-sitemap.xml", "outer-ssr"),
];

/// Templates that must stay PRIVATE even though a broader [`PUBLIC_ROUTES`] template also matches
/// them. Checked BEFORE the public list, so an entry here always wins.
///
/// A committed list of *templates* has one failure mode a list of concrete routes does not: a
/// `:param` can swallow a path that was meant to be private. Measured while making this module --
/// `GET /zaarhub/:slug/:id` (the public SSR listing page) also matches `GET /zaarhub/admin/legal`
/// and `GET /zaarhub/claims/:visitor_id`; `GET /b2b/rfqs/:id` also matches `GET /b2b/rfqs/my`; and
/// `GET /b2b/products/:id` (the public catalogue) also matches `GET /b2b/products/my` and
/// `GET /b2b/products/export`. The old inline predicate dodged the first of those with
/// `!path.contains("/admin/")`; the committed list says it out loud instead, where it can be
/// tested.
///
/// These entries can only ever REMOVE a path from the public set, so a mistake here refuses a
/// caller the handler would have refused anyway -- it cannot expose anything. An entry is a
/// `(method, template, why)` triple, exactly like [`PUBLIC_ROUTES`]; `"*"` means any method.
pub const RESERVED_PRIVATE: &[PublicRoute] = &[
    (
        "*",
        "/zaarhub/admin/*rest",
        "the ZaarHub /admin/ namespace is never anonymous (T6)",
    ),
    (
        "GET",
        "/zaarhub/claims/:visitor_id",
        "a visitor's own claims; the handler owner-checks",
    ),
    ("GET", "/b2b/rfqs/my", "supplier-scoped"),
    ("GET", "/b2b/products/my", "supplier-scoped"),
    ("GET", "/b2b/products/export", "supplier-scoped"),
];

/// True when `method path` is public in its own right (`headers` only matter for the internal-key
/// arm). A path that is public here is NOT authorisation: `auth_guard` still hands the request to
/// the handler, which scopes the rows it returns.
///
/// `auth_guard` runs on BOTH routers. Inside the `/api/v1` nest axum's `StripPrefix` has already
/// removed the prefix (proved live: the guard logs `path=/health`), while the outer router sees
/// the full `/api/v1/...` path for anything nested. Accepting both spellings is what lets ONE
/// committed list serve both mounts.
pub fn is_public(method: &Method, path: &str, headers: &HeaderMap) -> bool {
    let path = path.strip_prefix("/api/v1").unwrap_or(path);
    if INTERNAL_KEY_PATHS.iter().any(|p| path.starts_with(p)) {
        return headers.contains_key(INTERNAL_KEY_HEADER);
    }
    // A path reserved as private wins over any public template that would otherwise match it.
    if RESERVED_PRIVATE
        .iter()
        .any(|r| entry_matches(method, path, r))
    {
        return false;
    }
    PUBLIC_ROUTES.iter().any(|r| entry_matches(method, path, r))
}

/// Segment-wise template match. `:param` is one non-empty segment; `*rest` the non-empty
/// remainder. A trailing empty segment is significant, so `/coop-hub` and `/coop-hub/` are
/// different templates -- exactly as axum treats them.
fn template_matches(template: &str, path: &str) -> bool {
    let mut tpl = template.split('/');
    let mut seg = path.split('/');
    loop {
        match (tpl.next(), seg.next()) {
            (None, None) => return true,
            (Some(t), Some(s)) => {
                if let Some(_rest) = t.strip_prefix('*') {
                    return !s.is_empty();
                }
                if t.starts_with(':') {
                    if s.is_empty() {
                        return false;
                    }
                } else if t != s {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pub_route(method: &str, path: &str) -> bool {
        is_public(
            &Method::from_bytes(method.as_bytes()).unwrap(),
            path,
            &HeaderMap::new(),
        )
    }

    fn count_where(why: &str) -> usize {
        PUBLIC_ROUTES.iter().filter(|r| r.2 == why).count()
    }

    // ── the rule itself ────────────────────────────────────────────────────────────────────────
    #[test]
    fn private_by_default() {
        // Not on the list -> private, including the routes this card narrowed.
        for (m, p) in [
            ("POST", "/spotlight/123/feature"),
            ("POST", "/pipeline/ingest"),
            ("POST", "/businesses/123/images"),
            ("POST", "/directories/x/subscribers/import"),
            ("POST", "/bookings/123/status"),
            ("POST", "/bookings/123/cancel"),
            ("POST", "/events/123/edit"),
            ("GET", "/zaarhub/claims/123"),
            ("GET", "/b2b/rfqs/my"),
            ("GET", "/b2b/rfqs/123/messages"),
            ("GET", "/b2b/rfqs/123/bids"),
            ("GET", "/b2b/products/my"),
            ("GET", "/b2b/products/export"),
            // A route nobody has written yet.
            ("POST", "/admin/totally-new-route"),
            ("GET", "/admin/members"),
        ] {
            assert!(!pub_route(m, p), "{} {} must be private", m, p);
        }
    }

    #[test]
    fn public_routes_answer() {
        // The outer router sees the full path; the nest sees it stripped. Both spellings match.
        for p in ["/health", "/api/v1/health"] {
            assert!(pub_route("GET", p), "{} must be public", p);
        }
        // The outer SSR router's public pages (guarded by this same list).
        for p in [
            "/zaarhub/winter-garden",
            "/legal/terms",
            "/sitemap.xml",
            "/robots.txt",
            "/rfq-marketplace",
            "/coop-hub",
            "/lead-exchange",
            "/uploads/a.png",
            "/p/trap-door",
            "/l/abc123",
            "/zaarhub-sitemap.xml",
        ] {
            assert!(pub_route("GET", p), "{} must be public", p);
        }
        // Anonymous writes that stay anonymous by design.
        for (m, p) in [
            ("POST", "/visitors/track"),
            ("POST", "/submissions"),
            ("POST", "/webhooks/stripe"),
            ("POST", "/messages/123"),
            ("POST", "/visitor/register"),
            ("GET", "/visitor/favorites/check/123"),
        ] {
            assert!(pub_route(m, p), "{} {} must stay public", m, p);
        }
        // A guarded router must not be widened by the prefix form.
        assert!(!pub_route("GET", "/api/v1/admin/members"));
        assert!(!pub_route("POST", "/api/v1/pipeline/ingest"));
    }

    #[test]
    fn method_is_part_of_the_decision() {
        assert!(pub_route("GET", "/directories/winter-garden/businesses"));
        assert!(!pub_route("POST", "/directories/winter-garden/businesses"));
        assert!(pub_route("POST", "/submissions"));
        assert!(!pub_route("GET", "/submissions"));
        assert!(!pub_route("PUT", "/health"));
    }

    #[test]
    fn head_follows_get() {
        assert!(pub_route("HEAD", "/health"));
        assert!(pub_route("GET", "/health"));
        assert!(!pub_route("HEAD", "/admin/members"));
    }

    #[test]
    fn segments_not_prefixes() {
        // `:param` is exactly one segment: `/zaarhub/:slug` is the city page, and
        // `/zaarhub/:slug/:id` (the listing page) is a DIFFERENT template -- the two-segment
        // template does not swallow the three-segment path.
        assert!(pub_route("GET", "/zaarhub/winter-garden"));
        assert!(pub_route("GET", "/zaarhub/winter-garden/1234"));
        assert!(!pub_route("GET", "/zaarhub/winter-garden/1234/deeper"));
        // ...and a template never matches a longer path.
        assert!(pub_route("GET", "/deals"));
        assert!(pub_route("GET", "/deals/abc"));
        assert!(!pub_route("GET", "/deals/abc/deeper"));
        // trailing slashes are distinct templates
        assert!(pub_route("GET", "/coop-hub"));
        assert!(pub_route("GET", "/coop-hub/"));
        assert!(!pub_route("GET", "/coop-hub/extra"));
    }

    #[test]
    fn internal_key_arm() {
        let mut h = HeaderMap::new();
        assert!(!is_public(&Method::GET, "/cron/content-queue-worker", &h));
        h.insert(INTERNAL_KEY_HEADER, "x".parse().unwrap());
        assert!(is_public(&Method::GET, "/cron/content-queue-worker", &h));
        // the key arm covers /cron/ only — it cannot open anything else
        assert!(!is_public(&Method::POST, "/pipeline/ingest", &h));
    }

    /// The route templates `src/routes.rs` mounts, read from the source at compile time. A route
    /// joining or leaving the router changes this set, which is what makes the parity test below
    /// fail loudly instead of the allowlist silently rotting. (The IncentiveSwift precedent reads
    /// `main.rs`; Multi-Directory mounts its router in `routes.rs`.)
    fn mounted_routes() -> Vec<String> {
        const SRC: &str = include_str!("../routes.rs");
        let bytes = SRC.as_bytes();
        let needle = b".route(";
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                let mut j = i + needle.len();
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    let mut k = j + 1;
                    while k < bytes.len() && bytes[k] != b'"' {
                        k += 1;
                    }
                    out.push(SRC[j + 1..k].to_string());
                }
                i = j;
            } else {
                i += 1;
            }
        }
        out
    }

    /// Every committed template must name a route the router really mounts. A typo, or an entry
    /// left behind after a route is renamed, is a live hole the moment some other route takes that
    /// path -- so it fails here instead.
    #[test]
    fn every_entry_names_a_mounted_route() {
        let mounted = mounted_routes();
        assert!(
            mounted.len() > 600,
            "route census found only {} routes -- the extractor is broken, not the allowlist",
            mounted.len()
        );
        for entry in PUBLIC_ROUTES {
            assert!(
                mounted.iter().any(|m| m == entry.1),
                "PUBLIC_ROUTES entry {:?} is not a mounted route",
                entry.1
            );
        }
        for entry in RESERVED_PRIVATE {
            if entry.1.contains('*') {
                // A synthetic prefix pattern ("/zaarhub/admin/*rest"), not a mounted path.
                continue;
            }
            assert!(
                mounted.iter().any(|m| m == entry.1),
                "RESERVED_PRIVATE entry {:?} is not a mounted route",
                entry.1
            );
        }
    }

    /// A reserved-private path beats any broader public template, and the templates it shadows
    /// still serve their real public paths.
    #[test]
    fn reserved_private_beats_a_broad_public_template() {
        // Control arm: the templates these entries shadow are still public.
        for p in [
            "/zaarhub/winter-garden/1234",
            "/zaarhub/analytics/overview",
            "/zaarhub/cities/atlanta",
            "/zaarhub/listings/abc",
            "/b2b/rfqs/abc",
            "/b2b/products/abc",
        ] {
            assert!(pub_route("GET", p), "{p} must stay public");
        }
        // Each of these is matched by one of the templates above and must still be private.
        for (m, p) in [
            ("GET", "/zaarhub/admin/legal"),
            ("POST", "/zaarhub/admin/legal"),
            ("GET", "/zaarhub/admin/config"),
            ("GET", "/zaarhub/admin/merge-fields"),
            ("GET", "/zaarhub/admin/places/search"),
            ("GET", "/zaarhub/admin/provider-keys/google-places"),
            ("GET", "/api/v1/zaarhub/admin/legal"),
            ("GET", "/zaarhub/claims/00000000"),
            ("GET", "/b2b/rfqs/my"),
            ("GET", "/b2b/products/my"),
            ("GET", "/b2b/products/export"),
        ] {
            assert!(!pub_route(m, p), "{} {} must stay private", m, p);
        }
        for r in RESERVED_PRIVATE {
            assert!(!r.2.trim().is_empty(), "{} {} has no reason", r.0, r.1);
        }
    }

    #[test]
    fn the_narrowed_four_are_absent_by_motivation() {
        // 4 anonymous writers + 9 in-handler-auth tightenings == 13 legs off the list.
        let narrowed = count_where("narrowed");
        assert_eq!(
            narrowed, 0,
            "narrowed legs must not appear in PUBLIC_ROUTES"
        );
    }

    #[test]
    fn every_entry_has_a_reason() {
        assert!(!PUBLIC_ROUTES.is_empty());
        for r in PUBLIC_ROUTES {
            assert!(!r.2.trim().is_empty(), "{} {} has no reason", r.0, r.1);
            assert!(r.1.starts_with('/'), "{} is not a path template", r.1);
            assert!(
                r.1.split('/').all(|s| !s.contains(' ')),
                "{} has whitespace",
                r.1
            );
        }
        // The count is the measured census result minus the 13 narrowed legs, plus the
        // B142 IndexNow key file (a public root path), plus the B187 api-key verify
        // endpoint (POST /api-keys/verify, made callable), plus the pretty blog RSS
        // (GET /:slug/blog/feed.xml, so the feed's own advertised self url serves);
        // if this changes, the census wants a look.
        assert_eq!(
            PUBLIC_ROUTES.len(),
            161,
            "allowlist size moved off the measured 158 (+1 IndexNow key file, +1 B187 api-key verify, +1 blog RSS self url)"
        );
    }
}
