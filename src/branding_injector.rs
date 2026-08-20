//! White-label branding injection.
//!
//! Pure, side-effect-free helper that takes a raw HTML document plus an
//! optional `DirectoryBranding` and returns the HTML with the directory's
//! white-label styling injected into the `<head>`:
//!   * favicon `<link rel="icon">`
//!   * theme-color meta
//!   * CSS `:root` variables for colors + fonts (override the default theme)
//!   * logo swap (replaces the default logo `<img>`/mark with the branded logo)
//!
//! A `None` branding yields the input unchanged (default theme applies).

use crate::models::DirectoryBranding;
use sqlx::PgPool;
use uuid::Uuid;

/// Regex-free HTML head injection. We insert right after `<head>` (first match)
/// or fall back to prepending at the very start of the document if no head tag
/// is found. Never panics on malformed input.

const HEAD_STYLE_TEMPLATE: &str = r#"<style id="branding-theme">
:root {
  --md-primary: $PRIMARY;
  --md-secondary: $SECONDARY;
  --md-accent: $ACCENT;
  --md-bg: $BACKGROUND;
  --md-text: $TEXT;
  --md-heading: $HEADING;
  --md-link: $LINK;
  --md-btn-bg: $BTN_BG;
  --md-btn-text: $BTN_TEXT;
  --md-font-heading: $HEADING_FONT;
  --md-font-body: $BODY_FONT;
}
</style>"#;

const DEFAULT_COLOR: &str = "#0d9488";
const DEFAULT_BG: &str = "#ffffff";
const DEFAULT_TEXT: &str = "#1f2937";

fn clean_css(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Build the branded `<head>` fragment for a directory (or empty string if no branding).
pub fn branding_head(b: &DirectoryBranding) -> String {
    let primary = clean_css(&b.primary_color).unwrap_or_else(|| DEFAULT_COLOR.to_string());
    let secondary = clean_css(&b.secondary_color).unwrap_or_else(|| DEFAULT_COLOR.to_string());
    let accent = clean_css(&b.accent_color).unwrap_or_else(|| DEFAULT_COLOR.to_string());
    let background = clean_css(&b.background_color).unwrap_or_else(|| DEFAULT_BG.to_string());
    let text = clean_css(&b.text_color).unwrap_or_else(|| DEFAULT_TEXT.to_string());
    let heading = clean_css(&b.heading_color).unwrap_or_else(|| DEFAULT_TEXT.to_string());
    let link = clean_css(&b.link_color).unwrap_or_else(|| DEFAULT_COLOR.to_string());
    let btn_bg = clean_css(&b.button_background).unwrap_or_else(|| DEFAULT_COLOR.to_string());
    let btn_text = clean_css(&b.button_text).unwrap_or_else(|| "#ffffff".to_string());
    let hfont = clean_css(&b.heading_font).unwrap_or_default();
    let bfont = clean_css(&b.body_font).unwrap_or_default();

    let css = HEAD_STYLE_TEMPLATE
        .replace("$PRIMARY", &primary)
        .replace("$SECONDARY", &secondary)
        .replace("$ACCENT", &accent)
        .replace("$BACKGROUND", &background)
        .replace("$TEXT", &text)
        .replace("$HEADING", &heading)
        .replace("$LINK", &link)
        .replace("$BTN_BG", &btn_bg)
        .replace("$BTN_TEXT", &btn_text)
        .replace("$HEADING_FONT", &hfont)
        .replace("$BODY_FONT", &bfont);

    let mut out = String::new();

    if let Some(favicon) = clean_css(&b.favicon_url) {
        out.push_str(&format!(
            r#"<link rel="icon" type="image/x-icon" href="{}">"#,
            escape_attr(&favicon)
        ));
        out.push_str(&format!(
            r#"<link rel="shortcut icon" href="{}">"#,
            escape_attr(&favicon)
        ));
    }

    if let Some(color) = clean_css(&b.primary_color) {
        out.push_str(&format!(
            r#"<meta name="theme-color" content="{}">"#,
            escape_attr(&color)
        ));
    }

    out.push_str(&css);

    if let Some(logo) = clean_css(&b.logo_url) {
        // Inject a CSS rule that hides the default logo image and swaps in the
        // branded logo. The frontend already uses `.logo` / `#logo` classes on
        // most pages; this rule covers the common selectors without JS.
        out.push_str(&format!(
            r#"<style id="branding-logo">
.logo img, #logo img, .brand-logo img {{ content: none; }}
.logo, #logo {{
  background-image: url('{}');
  background-repeat: no-repeat;
  background-size: contain;
  background-position: left center;
}}
.logo img, #logo img {{ display: none !important; }}
</style>"#,
            escape_css_url(&logo)
        ));
    }

    out
}

/// Inject branding fragment into the HTML document head. Returns the modified
/// HTML, or the original unchanged if no `<head>` is present.
pub fn inject_branding(raw: &str, branding: Option<&DirectoryBranding>) -> String {
    let Some(b) = branding else {
        return raw.to_string();
    };

    let fragment = branding_head(b);
    if fragment.is_empty() {
        return raw.to_string();
    }

    if let Some(pos) = raw.to_lowercase().find("<head") {
        let head_end = raw[pos..].find('>').map(|e| pos + e + 1);
        if let Some(insert_at) = head_end {
            let mut out = String::with_capacity(raw.len() + fragment.len());
            out.push_str(&raw[..insert_at]);
            out.push_str(&fragment);
            out.push_str(&raw[insert_at..]);
            return out;
        }
    }

    // No head tag found — prepend.
    let mut out = String::with_capacity(raw.len() + fragment.len());
    out.push_str(&fragment);
    out.push_str(raw);
    out
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_css_url(s: &str) -> String {
    s.replace("\\", "\\\\").replace('"', "\\22").replace('\'', "\\27")
}

/// Fetch a directory's branding row by directory id, if one exists.
pub async fn fetch_branding(pool: &PgPool, directory_id: Uuid) -> Option<DirectoryBranding> {
    sqlx::query_as::<_, DirectoryBranding>(
        "SELECT * FROM directory_branding WHERE directory_id = $1 LIMIT 1",
    )
    .bind(directory_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// Extract the directory slug from a SPA path like `/d/{slug}` or `/d/{slug}/...`.
pub fn slug_from_dir_path(path: &str) -> Option<String> {
    let trimmed = path.trim_start_matches('/');
    let mut parts = trimmed.split('/');
    if parts.next()? != "d" {
        return None;
    }
    parts.next().map(|s| s.to_string())
}

/// Fetch branding by directory slug (resolved through the directories table).
pub async fn fetch_branding_by_slug(pool: &PgPool, slug: &str) -> Option<DirectoryBranding> {
    let row: Option<(Uuid,)> = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE slug = $1 LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    match row {
        Some((id,)) => fetch_branding(pool, id).await,
        None => None,
    }
}
