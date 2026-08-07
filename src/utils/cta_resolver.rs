//! CTA type resolver — Maps predefined CTA types to URLs and display labels.
//! Used for standardized call-to-action buttons on business listings.

use serde_json::Value;

/// 13 valid CTA types that can be assigned to a business listing.
pub const VALID_CTA_TYPES: &[&str] = &[
    "book_now",
    "book_call",
    "claim_deal",
    "get_quote",
    "learn_more",
    "visit_website",
    "call_now",
    "send_message",
    "view_menu",
    "view_catalog",
    "join_rewards",
    "rsvp",
    "none",
];

/// Resolve a CTA type into a URL suitable for an href attribute.
///
/// * `cta_type` — one of the VALID_CTA_TYPES; unknown values map to "#".
/// * `business` — the business JSON object (must contain id, city/slug if available, website, phone).
/// * `meta_data` — the business_meta.meta_data JSONB, which may contain overrides like `book_url`.
/// * `base_path` — directory base path (e.g. "/atlanta"); used for booking links.
pub fn resolve_cta_url(
    cta_type: &str,
    business: &Value,
    meta_data: &Value,
) -> String {
    let business_id = business.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let city_slug = business
        .get("city")
        .and_then(|v| v.as_str())
        .map(|c| slugify(c))
        .unwrap_or_default();
    let website = business
        .get("website")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let phone = business
        .get("phone")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match cta_type {
        "book_now" => format!("/book/{}/{}", city_slug, business_id),
        "book_call" => {
            if let Some(book_url) = meta_data.get("book_url").and_then(|v| v.as_str()) {
                if !book_url.is_empty() {
                    return book_url.to_string();
                }
            }
            format!("/book/{}/{}", city_slug, business_id)
        }
        "visit_website" => {
            if website.is_empty() {
                "#".to_string()
            } else {
                website.to_string()
            }
        }
        "call_now" => {
            if phone.is_empty() {
                "#".to_string()
            } else {
                format!("tel:{}", phone)
            }
        }
        "claim_deal" => "#deals".to_string(),
        "get_quote" => "#contact".to_string(),
        "learn_more" => "#about".to_string(),
        "send_message" => "#contact".to_string(),
        "view_menu" => "#services".to_string(),
        "view_catalog" => "#catalog".to_string(),
        "join_rewards" => "#loyalty".to_string(),
        "rsvp" => "#events".to_string(),
        "none" | _ => "#".to_string(),
    }
}

/// Map a CTA type to a human-readable display label (icon + text).
/// Returns (label, icon_html) — suitable for template rendering.
pub fn cta_label(cta_type: &str) -> (&'static str, &'static str) {
    match cta_type {
        "book_now" => ("Book Now", "📅"),
        "book_call" => ("Book a Call", "📞"),
        "claim_deal" => ("Claim Deal", "🎯"),
        "get_quote" => ("Get Quote", "💰"),
        "learn_more" => ("Learn More", "ℹ️"),
        "visit_website" => ("Visit Website", "🌐"),
        "call_now" => ("Call Now", "📱"),
        "send_message" => ("Send Message", "💬"),
        "view_menu" => ("View Menu", "📋"),
        "view_catalog" => ("View Catalog", "📖"),
        "join_rewards" => ("Join Rewards", "⭐"),
        "rsvp" => ("RSVP", "📩"),
        _ => ("", ""),
    }
}

/// Verify a cta_type string is one of the 13 valid values.
/// Returns true for all valid types including "none".
pub fn is_valid_cta_type(cta_type: &str) -> bool {
    VALID_CTA_TYPES.contains(&cta_type)
}

/// Simple slugify: lowercase, replace non-alphanumeric with hyphens, collapse runs.
fn slugify(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_was_dash = false;
    for ch in lower.chars() {
        if ch.is_alphanumeric() || ch == '-' {
            out.push(if ch == '-' { '-' } else { ch });
            last_was_dash = ch == '-';
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed
    }
}
