//! URL cleaner — Strips tracking parameters and extracts display-friendly domains.
//! Used to present clean business URLs instead of Google Places UTM-laden links.

/// Known tracking/UTM parameters that should be stripped from URLs.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
    "utm_id", "utm_name", "utm_cid",
    "utm-source", "utm-medium", "utm-campaign",  // hyphen variants
    "fbclid", "gclid", "gclsrc", "dclid", "msclkid", "twclid",
    "y_source", "yclid",
    "igshid", "ref", "ref_src", "ref_url",
    "source", "mc_cid", "mc_eid",
    "_ga", "_gl",
    "campaign_id", "adgroup_id", "ad_id",
    "wickedid", "wbraid", "gbraid",
    "hsCtaTracking",
];

/// Strip known tracking/UTM parameters from a URL.
/// Returns the cleaned URL. If the URL is malformed, returns it unchanged.
pub fn strip_tracking_params(url: &str) -> String {
    if url.is_empty() {
        return url.to_string();
    }

    // Parse the URL
    let (base, query_str) = match url.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (url, None),
    };

    let query_str = match query_str {
        Some(q) if !q.is_empty() => q,
        _ => return base.to_string(),
    };

    // Split into individual params and filter out tracking params
    let clean_params: Vec<&str> = query_str
        .split('&')
        .filter(|param| {
            let key = param.split_once('=').map(|(k, _)| k).unwrap_or(param);
            let key_lower = key.to_lowercase();
            !TRACKING_PARAMS.iter().any(|tp| key_lower == *tp)
        })
        .collect();

    if clean_params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, clean_params.join("&"))
    }
}

/// Extract a display-friendly domain from a URL for use as link text.
/// E.g. "https://www.aspendental.com/providers/Duy-Truong/1134614878/"
///   → "aspendental.com"
/// E.g. "https://frankgayservices.com/service-area/orlando/"
///   → "frankgayservices.com"
pub fn display_domain(url: &str) -> String {
    if url.is_empty() {
        return url.to_string();
    }

    // Strip protocol
    let without_protocol = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Get just the hostname (before first / or ?)
    let host = without_protocol
        .split('/')
        .next()
        .and_then(|h| h.split('?').next())
        .unwrap_or(without_protocol);

    // Strip www. prefix
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// Full clean: strip tracking params AND return display domain.
/// Returns (clean_url, display_label).
pub fn clean_url_pair(url: &str) -> (String, String) {
    let clean = strip_tracking_params(url);
    let display = display_domain(&clean);
    (clean, display)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_utm_params() {
        let dirty = "https://www.aspendental.com/providers/Duy-Truong/1134614878/?utm_source=googleplaces&utm_medium=lociqgoogleplaces&utm_campaign=&utm_content=listing";
        let clean = strip_tracking_params(dirty);
        assert_eq!(clean, "https://www.aspendental.com/providers/Duy-Truong/1134614878/");
    }

    #[test]
    fn test_strip_fbclid() {
        let dirty = "https://example.com/page?fbclid=abc123&good=keep";
        let clean = strip_tracking_params(dirty);
        assert_eq!(clean, "https://example.com/page?good=keep");
    }

    #[test]
    fn test_no_params() {
        assert_eq!(strip_tracking_params("https://example.com"), "https://example.com");
    }

    #[test]
    fn test_only_tracking_params() {
        let dirty = "https://example.com?utm_source=google&fbclid=123";
        let clean = strip_tracking_params(dirty);
        assert_eq!(clean, "https://example.com");
    }

    #[test]
    fn test_y_source() {
        let dirty = "https://heyrowan.com/pages/boca?utm_source=yext&y_source=1_MTA2ODY4MDA3NS03MTUtbG9jYXRpb24ud2Vic2l0ZQ%3D%3D";
        let clean = strip_tracking_params(dirty);
        assert_eq!(clean, "https://heyrowan.com/pages/boca");
    }

    #[test]
    fn test_display_domain() {
        assert_eq!(display_domain("https://www.aspendental.com/providers/Duy-Truong/"), "aspendental.com");
        assert_eq!(display_domain("http://frankgayservices.com/service-area/orlando/"), "frankgayservices.com");
        assert_eq!(display_domain("https://example.com"), "example.com");
        assert_eq!(display_domain("https://www.example.com"), "example.com");
    }

    #[test]
    fn test_clean_url_pair() {
        let (clean, display) = clean_url_pair("https://www.example.com/page?utm_source=g&ref=abc&good=1");
        assert_eq!(clean, "https://www.example.com/page?good=1");
        assert_eq!(display, "example.com");
    }
}
