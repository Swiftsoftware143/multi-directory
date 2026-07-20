//! Dynamic OG Image Generation (SVG-based social cards).
//!
//! Returns `image/svg+xml` for social media crawlers so directory, blog, business,
//! and trapdoor pages automatically get rich OG images without server-side rasterization.

use axum::{
    extract::{Path, State},
    http::header,
    response::IntoResponse,
};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

/// GET /public/og/{page_type}/{page_id}
///
/// Generates a 1200x630 SVG social card for the given entity.
pub async fn dynamic_og_image(
    State(s): State<AppState>,
    Path((page_type, page_id_str)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let (title, description, dir_name) = match page_type.as_str() {
        "directory" => {
            let id = Uuid::parse_str(&page_id_str)
                .map_err(|_| AppError::NotFound("Invalid directory ID".into()))?;
            let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT name, description, city FROM directories WHERE id = $1"
            )
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound("Directory not found".into()))?;

            let dir_name = row.0.clone();
            let desc = row.1.unwrap_or_else(|| format!("Browse {} - find local businesses and services.", &row.0));
            (row.0, desc, dir_name)
        }
        "blog" => {
            let id = Uuid::parse_str(&page_id_str)
                .map_err(|_| AppError::NotFound("Invalid blog post ID".into()))?;
            let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT bp.title, bp.excerpt, d.name \
                 FROM blog_posts bp \
                 JOIN directories d ON d.id = bp.directory_id \
                 WHERE bp.id = $1"
            )
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound("Blog post not found".into()))?;

            let dir_name = row.2.unwrap_or_default();
            let desc = row.1.unwrap_or_default();
            (row.0, desc, dir_name)
        }
        "business" => {
            let id = Uuid::parse_str(&page_id_str)
                .map_err(|_| AppError::NotFound("Invalid business ID".into()))?;
            let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT b.name, b.description, d.name \
                 FROM businesses b \
                 JOIN directories d ON d.id = b.directory_id \
                 WHERE b.id = $1"
            )
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound("Business not found".into()))?;

            let dir_name = row.2.unwrap_or_default();
            let desc = row.1.unwrap_or_else(|| format!("Find {} - browse services, reviews, and contact info.", &row.0));
            (row.0, desc, dir_name)
        }
        "trapdoor" | "programmatic" => {
            let id = Uuid::parse_str(&page_id_str)
                .map_err(|_| AppError::NotFound("Invalid page ID".into()))?;
            let row = sqlx::query_as::<
                _,
                (Option<String>, Option<String>, Option<String>, Option<String>),
            >(
                "SELECT pp.title, pp.meta_title, pp.meta_description, d.name \
                 FROM programmatic_pages pp \
                 JOIN directories d ON d.id = pp.directory_id \
                 WHERE pp.id = $1"
            )
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound("Trapdoor page not found".into()))?;

            let title = row.0.or(row.1.clone()).unwrap_or_else(|| "Page".to_string());
            let description = row.2.unwrap_or_default();
            let dir_name = row.3.unwrap_or_default();
            (title, description, dir_name)
        }
        _ => return Err(AppError::NotFound(format!("Unknown page type: {}", page_type))),
    };

    let svg = generate_og_svg(&title, &description, &dir_name);

    let headers = [
        (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
        (header::CACHE_CONTROL, "public, max-age=3600"),
        (axum::http::HeaderName::from_static("x-og-generated"), "true"),
    ];

    Ok((headers, svg))
}

// SVG Generation
// ---------------

fn generate_og_svg(title: &str, description: &str, dir_name: &str) -> String {
    let title = truncate(title, 80);
    let description = truncate(description, 120);
    let dir_name: String = if dir_name.is_empty() {
        "SwiftDirectory".to_string()
    } else {
        truncate(dir_name, 40)
    };

    // font family for SVG text — use system-ui which is available on all modern platforms
    let ff = "-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, Oxygen, Ubuntu, Cantarell, Helvetica Neue, sans-serif";
    let t1 = esc_xml(&title_line(&title, 0, 35));
    let t2 = title_line2_xml(&title, 35);
    let d1 = esc_xml(&description_line(&description, 0, 60));
    let d2 = description_line2_xml(&description, 60);
    let dl = esc_xml(&dir_name);

    // Build SVG piece-by-piece to avoid Rust macro issues with hyphens in raw strings.
    let mut s = String::with_capacity(1400);
    s.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630" width="1200" height="630">"#);
    s.push_str(r#"<defs><linearGradient id="og-bg" x1="0%" y1="0%" x2="100%" y2="100%">"#);
    s.push_str(r#"<stop offset="0%" style="stop-color:#0d9488;stop-opacity:1"/>"#);
    s.push_str(r#"<stop offset="100%" style="stop-color:#2563eb;stop-opacity:1"/>"#);
    s.push_str(r#"</linearGradient>"#);
    s.push_str(r#"<linearGradient id="og-accent" x1="0%" y1="0%" x2="100%" y2="0%">"#);
    s.push_str(r#"<stop offset="0%" style="stop-color:#14b8a6;stop-opacity:1"/>"#);
    s.push_str(r#"<stop offset="100%" style="stop-color:#3b82f6;stop-opacity:1"/>"#);
    s.push_str(r#"</linearGradient></defs>"#);

    s.push_str(r#"<rect width="1200" height="630" fill="url(#og-bg)" rx="0"/>"#);
    s.push_str(r#"<rect x="0" y="0" width="1200" height="6" fill="url(#og-accent)"/>"#);
    s.push_str(r#"<circle cx="1100" cy="530" r="280" fill="rgba(255,255,255,0.04)"/>"#);
    s.push_str(r#"<circle cx="1050" cy="500" r="180" fill="rgba(255,255,255,0.06)"/>"#);

    // Title block
    s.push_str(&format!("<text x=\"60\" y=\"210\" font-family=\"{ff}\" font-size=\"52\" font-weight=\"800\" fill=\"#ffffff\">"));
    s.push_str(&format!("<tspan x=\"60\" dy=\"0\">{t1}</tspan>"));
    s.push_str(&t2);
    s.push_str("</text>");

    // Description block
    s.push_str(&format!("<text x=\"60\" y=\"340\" font-family=\"{ff}\" font-size=\"26\" font-weight=\"400\" fill=\"rgba(255,255,255,0.9)\">"));
    s.push_str(&d1);
    s.push_str(&d2);
    s.push_str("</text>");

    // Directory name
    s.push_str(&format!("<text x=\"60\" y=\"560\" font-family=\"{ff}\" font-size=\"22\" font-weight=\"600\" fill=\"rgba(255,255,255,0.85)\">{dl}</text>"));

    // Branding
    s.push_str(&format!("<text x=\"1060\" y=\"580\" font-family=\"{ff}\" font-size=\"20\" font-weight=\"700\" fill=\"rgba(255,255,255,0.5)\" text-anchor=\"end\">SwiftDirectory</text>"));

    s.push_str("</svg>");
    s
}

/// Truncate text with ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('\u{2026}');
        t
    } else {
        s.to_string()
    }
}

fn title_line(s: &str, start: usize, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if start >= chars.len() {
        return String::new();
    }
    let end = (start + max).min(chars.len());
    chars[start..end].iter().collect()
}

fn title_line2_xml(s: &str, offset: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if offset >= chars.len() {
        return String::new();
    }
    let rest: String = chars[offset..].iter().collect();
    if rest.is_empty() {
        return String::new();
    }
    format!("<tspan x=\"60\" dy=\"60\">{}</tspan>", esc_xml(&rest))
}

fn description_line(s: &str, start: usize, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if start >= chars.len() {
        return String::new();
    }
    let end = (start + max).min(chars.len());
    chars[start..end].iter().collect()
}

fn description_line2_xml(s: &str, offset: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if offset >= chars.len() {
        return String::new();
    }
    let rest: String = chars[offset..].iter().collect();
    if rest.is_empty() {
        return String::new();
    }
    format!("<tspan x=\"60\" dy=\"40\">{}</tspan>", esc_xml(&rest))
}

/// Minimal XML-escaping for SVG text content.
fn esc_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}
