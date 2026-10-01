//! ONE source of truth for the brand — palette + typography.
//!
//! Two surfaces consume these tokens and therefore cannot drift again:
//!   * the **network homepage** (`frontend/index.html`), into which
//!     [`BrandTheme::css_block`] is injected at serve time (see `routes.rs`);
//!   * the **server-rendered directory / city pages** (`handlers::subfolder`),
//!     which render the same `:root` block into their own `<style>`.
//!
//! Nothing here is decoration: the values are *configuration*. Resolution order
//! is
//!   1. the compiled ZaarHub defaults in [`BrandTheme::zaarhub`] (the shape of
//!      the homepage — system font stack, `#f8fafc` / `#4a5568`),
//!   2. overlayed by the admin-editable row for the surface being rendered:
//!      * a network's home **and every one of its cities** resolve the *same*
//!        network theme (`network_branding`, else `zaarhub_site_config` — the
//!        "Branding" tab of `/zaarhub-admin.html`);
//!      * a **standalone** directory (no `network_id`) resolves its own
//!        `directory_branding` row.
//!
//! A sold directory therefore re-skins itself from the admin with no code edit.

use sqlx::{PgPool, Row};
use uuid::Uuid;

/// The homepage's body font stack. Kept byte-identical to the declared value
/// in `frontend/index.html` so both surfaces report the same computed
/// `font-family`.
pub const FONT_SYSTEM_STACK: &str =
    "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";

/// A fully resolved set of brand tokens for one surface.
#[derive(Debug, Clone, PartialEq)]
pub struct BrandTheme {
    pub font_body: String,
    pub font_heading: String,
    pub bg: String,
    pub card: String,
    pub text: String,
    pub text_light: String,
    pub text_muted: String,
    pub border: String,
    pub border_light: String,
    pub primary: String,
    pub primary_hover: String,
    pub primary_light: String,
    pub secondary: String,
    pub dark: String,
    pub bg_teal: String,
    pub link: String,
    pub radius: String,
    pub radius_lg: String,
    pub radius_xl: String,
    pub shadow_sm: String,
    pub shadow: String,
    pub shadow_lg: String,
}

/// Optional per-surface overrides read from the DB. `None` / blank means
/// "keep the inherited value".
#[derive(Debug, Default, Clone)]
pub struct BrandOverrides {
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    pub heading_color: Option<String>,
    pub link_color: Option<String>,
    pub heading_font: Option<String>,
    pub body_font: Option<String>,
}

fn clean(v: &Option<String>) -> Option<String> {
    v.as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl BrandOverrides {
    /// Read the override fields from a sqlx row that carries the
    /// `network_branding` / `directory_branding` column names.
    pub fn from_row(row: &sqlx::postgres::PgRow) -> Self {
        let get = |col: &str| row.try_get::<Option<String>, _>(col).ok().flatten();
        Self {
            primary_color: get("primary_color"),
            secondary_color: get("secondary_color"),
            background_color: get("background_color"),
            text_color: get("text_color"),
            heading_color: get("heading_color"),
            link_color: get("link_color"),
            heading_font: get("heading_font"),
            body_font: get("body_font"),
        }
    }
}

impl BrandTheme {
    /// The ZaarHub network defaults — the exact values the homepage ships.
    pub fn zaarhub() -> Self {
        Self {
            font_body: FONT_SYSTEM_STACK.to_string(),
            font_heading: FONT_SYSTEM_STACK.to_string(),
            bg: "#f8fafc".into(),
            card: "#fff".into(),
            text: "#4a5568".into(),
            text_light: "#718096".into(),
            text_muted: "#a0aec0".into(),
            border: "#e2e8f0".into(),
            border_light: "#edf2f7".into(),
            primary: "#f27f2f".into(),
            primary_hover: "#e06e1a".into(),
            primary_light: "#fef3e7".into(),
            secondary: "#116dc0".into(),
            dark: "#2b3255".into(),
            bg_teal: "#f0fdfa".into(),
            link: "#116dc0".into(),
            radius: "12px".into(),
            radius_lg: "16px".into(),
            radius_xl: "24px".into(),
            shadow_sm: "0 1px 2px rgba(0,0,0,.05)".into(),
            shadow: "0 4px 12px rgba(0,0,0,.08)".into(),
            shadow_lg: "0 12px 32px rgba(0,0,0,.12)".into(),
        }
    }

    /// Overlay admin-configured values on top of the inherited theme.
    pub fn with_overrides(mut self, o: &BrandOverrides) -> Self {
        if let Some(v) = clean(&o.primary_color) {
            self.primary = v.clone();
            self.primary_hover = darken(&v, 12);
            self.primary_light = lighten(&v, 92);
            self.link = v;
        }
        if let Some(v) = clean(&o.secondary_color) {
            self.secondary = v;
        }
        if let Some(v) = clean(&o.background_color) {
            self.bg = v;
        }
        if let Some(v) = clean(&o.text_color) {
            self.text = v.clone();
            self.text_light = mix(&v, "#ffffff", 25);
        }
        if let Some(v) = clean(&o.link_color) {
            self.link = v;
        }
        if let Some(v) = clean(&o.heading_font) {
            self.font_heading = v;
        }
        if let Some(v) = clean(&o.body_font) {
            self.font_body = v;
        }
        // `heading_color` only affects headings, which the page CSS paints with
        // `--dark`; honour it when the admin sets it.
        if let Some(v) = clean(&o.heading_color) {
            self.dark = v;
        }
        self
    }

    /// The `:root { … }` block both surfaces render. Names match the homepage
    /// so an injected block is a drop-in for the static `:root` there.
    pub fn css_block(&self) -> String {
        format!(
            ":root{{--font:{font};--font-heading:{fh};--bg:{bg};--card:{card};--text:{text};\
--text-light:{tl};--text-muted:{tm};--border:{border};--border-light:{bl};\
--primary:{p};--primary-hover:{ph};--primary-light:{pl};--secondary:{s};\
--dark:{dark};--bg-teal:{teal};--link:{link};--radius:{r};--radius-lg:{rl};\
--radius-xl:{rx};--shadow-sm:{shs};--shadow:{sh};--shadow-lg:{shl}}}",
            font = self.font_body,
            fh = self.font_heading,
            bg = self.bg,
            card = self.card,
            text = self.text,
            tl = self.text_light,
            tm = self.text_muted,
            border = self.border,
            bl = self.border_light,
            p = self.primary,
            ph = self.primary_hover,
            pl = self.primary_light,
            s = self.secondary,
            dark = self.dark,
            teal = self.bg_teal,
            link = self.link,
            r = self.radius,
            rl = self.radius_lg,
            rx = self.radius_xl,
            shs = self.shadow_sm,
            sh = self.shadow,
            shl = self.shadow_lg,
        )
    }

    /// Inject this theme into a full HTML document, before `</head>` so it wins
    /// over any static `:root` the document already carries. Returns the input
    /// unchanged when it has no `</head>`.
    pub fn inject(&self, html: &str) -> String {
        let tag = format!("<style id=\"brand-theme\">{}</style>", self.css_block());
        match html.rfind("</head>") {
            Some(pos) => {
                let mut out = String::with_capacity(html.len() + tag.len());
                out.push_str(&html[..pos]);
                out.push_str(&tag);
                out.push_str(&html[pos..]);
                out
            }
            None => html.to_string(),
        }
    }
}

/// `#rrggbb` → `(r,g,b)`, `None` for anything unparseable (never panics).
fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().trim_start_matches('#');
    let h = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.to_string()
    };
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(&h, 16).ok()?;
    Some((
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    ))
}

fn hex(r: u32, g: u32, b: u32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        r.clamp(0, 255),
        g.clamp(0, 255),
        b.clamp(0, 255)
    )
}

/// Percentage (0-100) of the colour mixed toward white.
fn lighten(c: &str, pct: u32) -> String {
    match rgb(c) {
        Some((r, g, b)) => hex(
            r as u32 + (255 - r as u32) * pct / 100,
            g as u32 + (255 - g as u32) * pct / 100,
            b as u32 + (255 - b as u32) * pct / 100,
        ),
        None => c.to_string(),
    }
}

/// Percentage (0-100) taken off each channel (a cheap "hover" shade).
fn darken(c: &str, pct: u32) -> String {
    match rgb(c) {
        Some((r, g, b)) => hex(
            r as u32 * (100 - pct) / 100,
            g as u32 * (100 - pct) / 100,
            b as u32 * (100 - pct) / 100,
        ),
        None => c.to_string(),
    }
}

/// Mix `a` toward `b` by `pct` percent.
fn mix(a: &str, b: &str, pct: u32) -> String {
    match (rgb(a), rgb(b)) {
        (Some((r1, g1, b1)), Some((r2, g2, b2))) => {
            let f = |x: u8, y: u8| (x as u32 * (100 - pct) + y as u32 * pct) / 100;
            hex(f(r1, r2), f(g1, g2), f(b1, b2))
        }
        _ => a.to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Resolution from the database (admin-editable, never hardcoded)
// ─────────────────────────────────────────────────────────────────────────────

/// The ZaarHub network slug — the platform's own community network. Preferred
/// (not hardwired) when resolving the homepage theme.
const ZAARHUB_NETWORK_SLUG: &str = "zaarhub";

/// Theme for a network's home page **and every city page under it**.
pub async fn theme_for_network(pool: &PgPool, network_id: Option<Uuid>) -> BrandTheme {
    let theme = BrandTheme::zaarhub();

    if let Some(nid) = network_id {
        let row = sqlx::query("SELECT * FROM network_branding WHERE network_id = $1 LIMIT 1")
            .bind(nid)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
        if let Some(row) = row {
            return theme.with_overrides(&BrandOverrides::from_row(&row));
        }
    }

    // No network_branding row yet: fall back to the network's admin-editable
    // site config (the "Branding" tab in /zaarhub-admin.html).
    let cfg = sqlx::query("SELECT primary_color, secondary_color FROM zaarhub_site_config LIMIT 1")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    match cfg {
        Some(row) => theme.with_overrides(&BrandOverrides {
            primary_color: row.try_get("primary_color").ok().flatten(),
            secondary_color: row.try_get("secondary_color").ok().flatten(),
            ..Default::default()
        }),
        None => theme,
    }
}

/// Theme for a directory. A directory that belongs to a network inherits the
/// network brand (home + cities stay consistent); a standalone directory
/// (no `network_id`) uses its own `directory_branding` row.
pub async fn theme_for_directory(
    pool: &PgPool,
    directory_id: Uuid,
    network_id: Option<Uuid>,
) -> BrandTheme {
    if network_id.is_some() {
        return theme_for_network(pool, network_id).await;
    }
    let row = sqlx::query("SELECT * FROM directory_branding WHERE directory_id = $1 LIMIT 1")
        .bind(directory_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    match row {
        Some(row) => BrandTheme::zaarhub().with_overrides(&BrandOverrides::from_row(&row)),
        None => BrandTheme::zaarhub(),
    }
}

/// Theme for the network homepage (`/`). Resolved from the `networks` table so
/// a second network can re-skin its own root without a code change; the
/// platform's ZaarHub network is simply the preferred row.
pub async fn theme_for_home(pool: &PgPool) -> BrandTheme {
    let network_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM networks WHERE status = 'active' \
         ORDER BY (slug = $1) DESC, created_at LIMIT 1",
    )
    .bind(ZAARHUB_NETWORK_SLUG)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    theme_for_network(pool, network_id).await
}

/// The admin-editable brand **name** a directory's pages carry — the suffix of
/// every server-rendered page title (and the footer / breadcrumb root /
/// `og:site_name`). Resolution order is
///   1. the owning network's name (`networks.name`) — a network's home *and*
///      every city under it share one brand,
///   2. the directory's own name (`directories.name`) when it stands alone,
///   3. the platform default (`zaarhub_site_config.site_name`).
///
/// The request host is deliberately **never** consulted: a page title must name
/// the *brand* — which a buyer renames from the admin — not the hostname the
/// request happened to arrive on (which renders as `127.0.0.1` locally and the
/// bare domain in production).
pub async fn brand_name_for_directory(
    pool: &PgPool,
    directory_name: &str,
    network_id: Option<Uuid>,
) -> String {
    if let Some(nid) = network_id {
        let name: Option<String> =
            sqlx::query_scalar("SELECT name FROM networks WHERE id = $1 LIMIT 1")
                .bind(nid)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        if let Some(n) = name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            return n;
        }
    }

    let dir = directory_name.trim();
    if !dir.is_empty() {
        return dir.to_string();
    }

    default_brand_name(pool).await
}

/// The platform's default brand name — the admin-editable
/// `zaarhub_site_config.site_name` (never a compiled-in literal), used only
/// when both the network and the directory carry no name.
pub async fn default_brand_name(pool: &PgPool) -> String {
    let name: Option<String> = sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(trim(site_name), ''), '') FROM zaarhub_site_config LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    name.filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_homepage() {
        let t = BrandTheme::zaarhub();
        assert_eq!(t.bg, "#f8fafc");
        assert_eq!(t.text, "#4a5568");
        assert_eq!(t.primary, "#f27f2f");
        assert_eq!(t.font_body, FONT_SYSTEM_STACK);
        assert!(!t.font_body.contains("Inter"));
    }

    #[test]
    fn overrides_apply_and_derive() {
        let t = BrandTheme::zaarhub().with_overrides(&BrandOverrides {
            primary_color: Some("#2563eb".into()),
            body_font: Some("Georgia, serif".into()),
            ..Default::default()
        });
        assert_eq!(t.primary, "#2563eb");
        assert_eq!(t.link, "#2563eb");
        assert_eq!(t.font_body, "Georgia, serif");
        // blank values never wipe an inherited token
        let t2 = BrandTheme::zaarhub().with_overrides(&BrandOverrides {
            primary_color: Some("   ".into()),
            ..Default::default()
        });
        assert_eq!(t2.primary, "#f27f2f");
    }

    #[test]
    fn injection_lands_inside_head() {
        let html = "<html><head><title>x</title></head><body></body></html>";
        let out = BrandTheme::zaarhub().inject(html);
        assert!(out.contains("<style id=\"brand-theme\">"));
        assert!(out.find("brand-theme").unwrap() < out.find("</head>").unwrap());
        assert_eq!(
            BrandTheme::zaarhub().inject("<html></html>"),
            "<html></html>"
        );
    }
}
