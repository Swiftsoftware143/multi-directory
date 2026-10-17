//! ONE canonical slug rule for the whole backend.
//!
//! Every door that accepts a slug — and every path that derives one — normalises through here, so
//! three things always hold the SAME bytes for one slug:
//!
//!   1. what md stores (`directories.slug` / `networks.slug`),
//!   2. what md hands to a sibling app (CoreSwift's `account_slug`, and the workspace e-mail
//!      `md-<slug>@local.coreswift` derived from it),
//!   3. what md renders into a URL (`https://zaarhub.com/<slug>`).
//!
//! Before this module a caller-supplied slug was stored *verbatim*. A padded slug therefore landed
//! as three different strings in three places: md stored `"  palm-bay "` (and served the city at
//! `/api/v1/directories/%20%20palm-bay`), CoreSwift stored the trimmed `"palm-bay"`, and the city
//! page rendered `https://zaarhub.com/%20%20palm-bay` — a URL that can never reach the city whose
//! slug it claims to be. No persisted value may be one a URL can never match.
//!
//! Rule: lowercase; ASCII alphanumerics kept; every other character (including whitespace, `.`,
//! `_`, `&`, and any non-ASCII letter) becomes a single `-`; collapsed runs; leading/trailing `-`
//! trimmed. `"  Palm Bay & Sons!  "` -> `"palm-bay-sons"`.

/// Canonical form of a slug. Idempotent: `slugify(slugify(s)) == slugify(s)`.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true; // suppress a leading dash
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// A caller-supplied slug, canonicalised. `None` when nothing survives normalisation — i.e. the
/// field was absent, blank, or whitespace-only. Callers MUST read that as NOT SUPPLIED (fall back
/// to a derived slug on create, keep the existing slug on update) and never as an empty slug.
/// CoreSwift applies the mirror of this rule to the same field, which is what keeps the two apps
/// holding one string.
pub fn supplied(raw: Option<&str>) -> Option<String> {
    raw.map(slugify).filter(|s| !s.is_empty())
}

/// Canonical form, clamped to `max` bytes-worth of characters. For slugs DERIVED from a long free
/// text field (a directory name), where an absurdly long name must not become an absurdly long
/// URL component. A trailing dash left by the cut is trimmed, so the result never ends in `-`.
pub fn slugify_max(s: &str, max: usize) -> String {
    let mut out: String = slugify(s).chars().take(max).collect();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_slug_is_trimmed() {
        assert_eq!(slugify("  palm-bay "), "palm-bay");
        assert_eq!(slugify("\t\npalm-bay\r\n"), "palm-bay");
        assert_eq!(slugify(" palm bay "), "palm-bay");
    }

    #[test]
    fn wellformed_slug_is_byte_identical() {
        // The rule must be a no-op on anything already canonical — 10 directories and 3 networks
        // live in production hold canonical slugs and must not shift.
        for s in ["palm-bay", "zaarhub", "st-lucie-county", "a1-towing-2"] {
            assert_eq!(slugify(s), s);
        }
    }

    #[test]
    fn punctuation_and_case_collapse() {
        assert_eq!(slugify("Palm Bay & Sons!"), "palm-bay-sons");
        assert_eq!(slugify("St. Lucie  County"), "st-lucie-county");
        assert_eq!(slugify("--leading-and-trailing--"), "leading-and-trailing");
        assert_eq!(slugify("already--double"), "already-double");
    }

    #[test]
    fn slugify_is_idempotent() {
        for s in ["  Palm Bay & Sons!  ", "palm-bay", "  ", "###", "A_B"] {
            assert_eq!(slugify(&slugify(s)), slugify(s));
        }
    }

    #[test]
    fn blank_is_not_supplied() {
        assert_eq!(supplied(None), None);
        assert_eq!(supplied(Some("")), None);
        assert_eq!(supplied(Some("   ")), None);
        assert_eq!(supplied(Some("  ")), None);
        assert_eq!(supplied(Some("###")), None);
        assert_eq!(supplied(Some(" Palm Bay ")), Some("palm-bay".to_string()));
    }

    #[test]
    fn derived_slug_is_clamped_without_trailing_dash() {
        let long = "Palm Bay ".repeat(30);
        let s = slugify_max(&long, 20);
        assert_eq!(s.chars().count(), 20);
        assert!(!s.ends_with('-'));
        assert!(s.starts_with("palm-bay-"));
        // A cut landing on a dash must not leave one behind: "ab-cdefghij" cut to 3 = "ab-".
        assert_eq!(slugify_max("ab cdefghij", 3), "ab");
        assert_eq!(slugify_max("ab cdefghij", 4), "ab-c");
    }
}
