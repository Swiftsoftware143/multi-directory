//! Shared merge-field engine (kanban B119).
//!
//! ONE vocabulary and ONE resolver used by every surface where content is authored, so a
//! single template can serve many cities. The engine accepts both `{name}` (page copy) and
//! `{{name}}` (email templates) — they are the same token, so authors do not have to learn
//! two syntaxes.
//!
//! The rule that matters (kanban t_ba93aea4): a merge field that cannot be resolved must
//! NEVER reach a visitor as raw braces. `render()` therefore keeps non-token braces (CSS,
//! JS) verbatim but DROPS any `{name}`/`{{name}}` it cannot fill, and reports it in
//! `Rendered::missing` so the admin can see it at save/preview time.

use serde::Serialize;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// The shared merge-field vocabulary. Additive changes only; names are never renamed so
/// already-authored content keeps working.
pub const VOCABULARY: &[(&str, &str)] = &[
    ("city", "City name"),
    ("city_name", "City name (same as {city})"),
    ("directory_name", "Directory / site name"),
    (
        "network_name",
        "Network name (blank for a standalone directory)",
    ),
    ("state", "State, e.g. FL"),
    ("primary_color", "Brand primary colour"),
    ("accent_color", "Brand accent colour"),
    ("background_color", "Brand background colour"),
    ("text_color", "Brand text colour"),
    ("business_name", "Business name"),
    ("owner_name", "Business owner's name"),
    ("dashboard_url", "Business dashboard URL"),
    ("contact_email", "Support / contact email"),
    ("contact_phone", "Contact phone"),
    ("site_url", "Directory site URL"),
    ("current_year", "Current year"),
    (
        "category",
        "Category / service category (on a category page)",
    ),
    ("service", "Service name (on a service page)"),
];

/// True when `name` is a field the platform can fill.
pub fn is_known(name: &str) -> bool {
    VOCABULARY.iter().any(|(k, _)| *k == name)
}

/// Field names only, for pickers and error text.
pub fn names() -> Vec<&'static str> {
    VOCABULARY.iter().map(|(k, _)| *k).collect()
}

/// A resolved set of merge-field values for one directory/city (or the network).
#[derive(Debug, Clone, Default)]
pub struct MergeContext {
    values: HashMap<String, String>,
}

impl MergeContext {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.values.insert(key.to_string(), value.into());
    }

    /// Set only when the value is present and non-empty (a blank never overwrites a default).
    pub fn set_opt(&mut self, key: &str, value: Option<String>) {
        if let Some(v) = value {
            let v = v.trim();
            if !v.is_empty() {
                self.values.insert(key.to_string(), v.to_string());
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    /// Platform/network scope: the ZaarHub site config, used for network-scoped pages
    /// (legal pages) and as the inheritance default for city directories.
    pub async fn for_network(db: &PgPool) -> Self {
        let mut ctx = MergeContext::new();
        ctx.set("site_url", "https://zaarhub.com");
        ctx.set("current_year", chrono::Utc::now().format("%Y").to_string());
        ctx.set("directory_name", "ZaarHub");
        ctx.set("network_name", "ZaarHub");

        if let Ok(Some(row)) = sqlx::query(
            "SELECT site_name, primary_color, secondary_color, contact_email, contact_phone \
             FROM zaarhub_site_config LIMIT 1",
        )
        .fetch_optional(db)
        .await
        {
            ctx.set_opt("directory_name", row.try_get("site_name").ok());
            ctx.set_opt("network_name", row.try_get("site_name").ok());
            ctx.set_opt(
                "primary_color",
                row.try_get::<Option<String>, _>("primary_color")
                    .ok()
                    .flatten(),
            );
            ctx.set_opt(
                "accent_color",
                row.try_get::<Option<String>, _>("secondary_color")
                    .ok()
                    .flatten(),
            );
            ctx.set_opt(
                "contact_email",
                row.try_get::<Option<String>, _>("contact_email")
                    .ok()
                    .flatten(),
            );
            ctx.set_opt(
                "contact_phone",
                row.try_get::<Option<String>, _>("contact_phone")
                    .ok()
                    .flatten(),
            );
        }
        ctx
    }

    /// City/directory scope: this directory's own identity, inheriting anything it does not
    /// set from the network context.
    pub async fn for_directory(db: &PgPool, directory_id: Uuid) -> Self {
        let mut ctx = MergeContext::for_network(db).await;

        let row = sqlx::query(
            "SELECT d.name, d.city, d.state, d.color_scheme, d.url_value, d.custom_domain, n.name AS network_name \
             FROM directories d LEFT JOIN networks n ON n.id = d.network_id WHERE d.id = $1",
        )
        .bind(directory_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();

        if let Some(r) = row {
            ctx.set_opt("directory_name", r.try_get("name").ok());
            if let Ok(Some(city)) = r.try_get::<Option<String>, _>("city") {
                ctx.set("city", city.clone());
                ctx.set("city_name", city);
            }
            ctx.set_opt("state", r.try_get("state").ok());
            ctx.set_opt(
                "network_name",
                r.try_get::<Option<String>, _>("network_name")
                    .ok()
                    .flatten(),
            );

            if let Ok(Some(scheme)) = r.try_get::<Option<serde_json::Value>, _>("color_scheme") {
                for (field, json_key) in [
                    ("primary_color", "primary"),
                    ("accent_color", "accent"),
                    ("background_color", "background"),
                    ("text_color", "text"),
                ] {
                    if let Some(v) = scheme.get(json_key).and_then(|v| v.as_str()) {
                        ctx.set(field, v);
                    }
                }
            }

            let url = r
                .try_get::<Option<String>, _>("custom_domain")
                .ok()
                .flatten()
                .or_else(|| r.try_get::<Option<String>, _>("url_value").ok().flatten());
            if let Some(u) = url {
                let u = u.trim();
                if !u.is_empty() {
                    let site = if u.starts_with("http") {
                        u.to_string()
                    } else {
                        format!("https://{}", u.trim_end_matches('/'))
                    };
                    ctx.set("site_url", site);
                }
            }
        }
        ctx
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Extract the merge-field names used in `text`, first-seen order, deduped.
/// Accepts `{name}` and `{{name}}`; ignores non-token braces such as CSS/JS blocks.
pub fn used_fields(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while let Some(open) = text[i..].find('{') {
        let start = i + open;
        let (inner_start, close_pat) = if text[start..].starts_with("{{") {
            (start + 2, "}}")
        } else {
            (start + 1, "}")
        };
        match text[inner_start..].find(close_pat) {
            Some(close) => {
                let name = text[inner_start..inner_start + close].trim();
                let next = inner_start + close + close_pat.len();
                if valid_name(name) && !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
                i = next;
            }
            None => break,
        }
    }
    out
}

/// Field names used by `fields` that the platform cannot fill. Empty == safe to save.
pub fn unknown_fields(fields: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in fields {
        for name in used_fields(f) {
            if !is_known(&name) && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// The result of rendering a chunk of authored text.
#[derive(Debug, Clone, Serialize)]
pub struct Rendered {
    /// Text with every resolvable field substituted; unresolvable tokens dropped (never raw braces).
    pub text: String,
    /// Field names the text referenced.
    pub used: Vec<String>,
    /// Referenced field names with no value (known-but-empty or unknown) — surfaced in preview.
    pub missing: Vec<String>,
}

/// Render `text` with `ctx`. Non-token braces are preserved; real tokens are always consumed
/// (so a visitor never receives `{{directory_name}}` literally).
pub fn render(text: &str, ctx: &MergeContext) -> Rendered {
    let used = used_fields(text);
    let mut missing: Vec<String> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while let Some(open) = text[i..].find('{') {
        let start = i + open;
        out.push_str(&text[i..start]);
        let (inner_start, close_pat) = if text[start..].starts_with("{{") {
            (start + 2, "}}")
        } else {
            (start + 1, "}")
        };
        match text[inner_start..].find(close_pat) {
            Some(close) => {
                let name = text[inner_start..inner_start + close].trim();
                let next = inner_start + close + close_pat.len();
                if valid_name(name) {
                    match ctx.get(name) {
                        Some(v) if !v.is_empty() => out.push_str(v),
                        _ => {
                            if !missing.iter().any(|m| m == name) {
                                missing.push(name.to_string());
                            }
                        }
                    }
                } else {
                    // Not a merge token (CSS/JS/format braces) — keep it exactly.
                    out.push_str(&text[start..next]);
                }
                i = next;
            }
            None => {
                out.push_str(&text[start..]);
                i = text.len();
            }
        }
    }
    out.push_str(&text[i..]);
    Rendered {
        text: out,
        used,
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> MergeContext {
        let mut c = MergeContext::new();
        c.set("city", "Palm Bay");
        c.set("directory_name", "Palm Bay Directory");
        c.set("current_year", "2026");
        c
    }

    #[test]
    fn resolves_single_and_double_braces() {
        let r = render("Welcome to {city} in {{current_year}}", &ctx());
        assert_eq!(r.text, "Welcome to Palm Bay in 2026");
        assert!(r.missing.is_empty());
        assert_eq!(r.used, vec!["city".to_string(), "current_year".to_string()]);
    }

    #[test]
    fn never_ships_unknown_or_empty_braces() {
        let r = render("Hi {nope} from {network_name}", &ctx());
        assert!(!r.text.contains('{'), "raw braces leaked: {}", r.text);
        assert!(r.missing.contains(&"nope".to_string()));
        assert!(r.missing.contains(&"network_name".to_string()));
    }

    #[test]
    fn preserves_non_token_braces() {
        let r = render("body{margin:0}var x={a:1}", &ctx());
        assert_eq!(r.text, "body{margin:0}var x={a:1}");
    }

    #[test]
    fn unknown_fields_flags_only_tokens() {
        assert_eq!(
            unknown_fields(&["{city} {bogus}"]),
            vec!["bogus".to_string()]
        );
        assert!(unknown_fields(&["color:#fff;{}"]).is_empty());
    }

    #[test]
    fn vocabulary_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for (k, _) in VOCABULARY {
            assert!(seen.insert(*k), "duplicate merge field {k}");
        }
    }
}
