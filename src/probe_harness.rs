//! `probe_harness` — attribution for harness-created rows (kanban t_d5e5af7e).
//!
//! Fleet policy `/opt/swift/docs/fleet-probe-residue-policy-2026-09-28.md` convention (c): a row
//! minted by an automated harness through a REAL product route may be marked with the
//! `X-Swift-Harness` request HEADER, so the residue sweeper can tell fleet machinery from a
//! customer. Multi-Directory's anonymous `visitors` table has no attribution class at all
//! (fingerprint / user_agent / ip / geo / timestamps only), and the measured 2026-10-02
//! headless-Chrome supplier-portal run minted `businesses` + `visitor_accounts` +
//! `claimed_businesses` rows that were byte-indistinguishable from a real supplier's.
//!
//! The marker is read from the header ONLY — never from the request body and never from a query
//! field. A body/query field would be client-controllable data that lands in customer-visible
//! columns, so a customer could sign their own row up as fleet machinery; a header is not part of
//! any customer-facing form and is the same channel the rest of the fleet's tooling uses.
//!
//! Storage contract: a row created by a real customer is byte-identical to what it was before
//! this module existed, because the marker is `NULL` unless the header is present AND well-formed.
//!
//! Shipped identically for FunnelSwift (t_fc88ec2a) and CoreSwift-CRM (t_66db3251 / t_3492e3d9).

use axum::http::HeaderMap;

/// The request header a harness uses to mark the rows it is about to create.
pub const PROBE_HARNESS_HEADER: &str = "x-swift-harness";

/// `^[a-z0-9][a-z0-9._-]{2,63}$` — 3 to 64 characters, so the first char plus at most 63 more.
const MIN_LEN: usize = 3;
const MAX_LEN: usize = 64;

/// Read `X-Swift-Harness` off the request headers and return the marker to store, if any.
///
/// Trim, then lowercase, then accept ONLY a value matching `^[a-z0-9][a-z0-9._-]{2,63}$`.
/// Everything else returns `None`, and the row is created exactly as every row was created before
/// this column existed:
///
/// * no header at all — the ordinary customer request;
/// * a value shorter than 3 or longer than 64 characters;
/// * a first character that is not `[a-z0-9]`, or any character outside `[a-z0-9._-]`
///   (this rejects SQL/HTML metacharacters, whitespace inside the value, and every non-ASCII
///   character, so `'\";DROP TABLE users;--` can never reach the column);
/// * a header that is not valid UTF-8 (`HeaderValue::to_str`).
///
/// The accepted value is stored verbatim after trim+lowercase and is never interpreted — the
/// caller binds it as a query parameter, so even a hostile value could not alter the statement.
pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(PROBE_HARNESS_HEADER)?.to_str().ok()?;
    normalise(raw)
}

/// The validation half of [`from_headers`], split out so it can be tested without building a
/// `HeaderMap`.
fn normalise(raw: &str) -> Option<String> {
    let marker = raw.trim().to_lowercase();
    if marker.len() < MIN_LEN || marker.len() > MAX_LEN {
        return None;
    }
    let mut chars = marker.chars();
    let first = chars.next()?;
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return None;
    }
    if !marker
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return None;
    }
    Some(marker)
}

/// The client IP for a request, preferring the edge proxy's headers over the socket peer.
///
/// Multi-Directory binds `127.0.0.1:8089` behind nginx (`X-Real-IP` + `X-Forwarded-For`, see the
/// `zaarhub.com` server block), so `ConnectInfo` alone would record `127.0.0.1` for every visitor.
/// Order: first hop of `X-Forwarded-For` → `X-Real-IP` → the socket peer address.
pub fn client_ip(headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> Option<String> {
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = v.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
    }
    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    peer.map(|p| p.ip().to_string())
}

/// The `User-Agent` of the request, or `None` when the header is absent / not valid UTF-8.
pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(PROBE_HARNESS_HEADER, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn a_well_formed_marker_is_stored_trimmed_and_lowercased() {
        assert_eq!(
            normalise("  Verify-T_c8030af5  ").as_deref(),
            Some("verify-t_c8030af5")
        );
        assert_eq!(normalise("abc").as_deref(), Some("abc"));
        assert_eq!(normalise("a.b_c-d9").as_deref(), Some("a.b_c-d9"));
        assert_eq!(
            normalise(&"a".repeat(64)).as_deref(),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn everything_else_is_null() {
        assert_eq!(from_headers(&HeaderMap::new()), None);
        assert_eq!(normalise("ab"), None);
        assert_eq!(normalise(&"a".repeat(65)), None);
        assert_eq!(normalise("-abc"), None);
        assert_eq!(normalise(".abc"), None);
        assert_eq!(normalise("_abc"), None);
        assert_eq!(normalise("'\";DROP TABLE users;--"), None);
        assert_eq!(normalise("verify t_c8030af5"), None);
        assert_eq!(normalise("verify/../x"), None);
        assert_eq!(normalise("VERIFY"), Some("verify".to_string()));
        assert_eq!(normalise("café-1"), None);
        assert_eq!(normalise(""), None);
        assert_eq!(normalise("   "), None);
        assert_eq!(
            from_headers(&headers_with("Verify-T_c8030af5")).as_deref(),
            Some("verify-t_c8030af5")
        );
    }

    #[test]
    fn client_ip_prefers_the_edge_proxy_headers() {
        let peer = Some("127.0.0.1:5555".parse().unwrap());
        let mut h = HeaderMap::new();
        // no proxy headers -> socket peer
        assert_eq!(client_ip(&h, peer).as_deref(), Some("127.0.0.1"));
        assert_eq!(client_ip(&h, None), None);
        // X-Real-IP wins over the peer
        h.insert("x-real-ip", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(client_ip(&h, peer).as_deref(), Some("203.0.113.7"));
        // first hop of X-Forwarded-For wins over X-Real-IP
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.9, 10.0.0.1"),
        );
        assert_eq!(client_ip(&h, peer).as_deref(), Some("198.51.100.9"));
    }

    #[test]
    fn user_agent_is_read_verbatim() {
        assert_eq!(user_agent(&HeaderMap::new()), None);
        let mut h = HeaderMap::new();
        h.insert("user-agent", HeaderValue::from_static("curl/8.5.0"));
        assert_eq!(user_agent(&h).as_deref(), Some("curl/8.5.0"));
    }
}
