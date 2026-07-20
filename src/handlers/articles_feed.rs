//! Public RSS/XML articles feed for search engine discovery.
//! Serves the last 50 published blog posts and business articles
//! for a directory, combined and ordered by publication date.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::AppState;
use crate::error::ApiResult;

/// Combined feed item from a UNION of blog_posts and business_articles.
#[derive(Debug, Deserialize, sqlx::FromRow)]
struct FeedItem {
    title: String,
    slug: String,
    excerpt_or_description: Option<String>,
    content: Option<String>,
    created_at: Option<DateTime<Utc>>,
    #[allow(dead_code)]
    source_type: String,
    author_or_business: Option<String>,
    first_tag_or_keyword: Option<String>,
}

/// GET /public/directories/{slug}/articles.xml
pub async fn articles_xml_feed(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    // 1. Fetch the directory to get name, description, and domain info
    let dir = sqlx::query!(
        r#"SELECT id, name, description, slug, url_value, custom_domain
           FROM directories WHERE slug = $1"#,
        slug
    )
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| crate::error::AppError::NotFound("Directory not found".into()))?;

    // 2. Determine the domain for links
    let domain = if let Some(ref cd) = dir.custom_domain {
        cd.clone()
    } else {
        let uv = dir.url_value.as_deref().unwrap_or(&dir.slug);
        format!("{}.{}", uv, s.config.base_domain)
    };

    // 3. Fetch the 50 latest published items from both tables
    let items: Vec<FeedItem> = sqlx::query_as::<_, FeedItem>(
        r#"
        SELECT
            bp.title,
            bp.slug,
            bp.excerpt AS excerpt_or_description,
            bp.content,
            COALESCE(bp.scheduled_at, bp.created_at) AS created_at,
            'blog' AS source_type,
            bp.author_name AS author_or_business,
            (SELECT unnest from unnest(bp.tags) LIMIT 1) AS first_tag_or_keyword
        FROM blog_posts bp
        WHERE bp.directory_id = $1
          AND bp.status = 'published'
          AND (bp.published = true OR bp.published IS NULL)
        UNION ALL
        SELECT
            ba.title,
            ba.slug,
            ba.meta_description AS excerpt_or_description,
            ba.content,
            ba.created_at,
            'article' AS source_type,
            NULL AS author_or_business,
            ba.keyword AS first_tag_or_keyword
        FROM business_articles ba
        WHERE ba.directory_id = $1
          AND ba.status = 'published'
        ORDER BY created_at DESC
        LIMIT 50
        "#,
    )
    .bind(dir.id)
    .fetch_all(&s.db)
    .await?;

    // 4. Build the RSS XML string
    let now_rfc2822 = Utc::now().format("%a, %d %b %Y %H:%M:%S %z").to_string();

    let mut xml = String::with_capacity(4096);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n");
    xml.push_str("<channel>\n");

    // Channel metadata
    xml.push_str(&format!(
        "  <title>{name} - Articles</title>\n",
        name = esc_xml(&dir.name)
    ));
    xml.push_str(&format!(
        "  <link>https://{domain}/{slug}/blog</link>\n",
        domain = domain,
        slug = dir.slug,
    ));
    xml.push_str(&format!(
        "  <description>{desc}</description>\n",
        desc = esc_xml(dir.description.as_deref().unwrap_or(""))
    ));
    xml.push_str("  <language>en-us</language>\n");
    xml.push_str(&format!(
        "  <atom:link href=\"https://{domain}/{slug}/articles.xml\" rel=\"self\" type=\"application/rss+xml\"/>\n",
        domain = domain,
        slug = dir.slug,
    ));
    xml.push_str(&format!("  <lastBuildDate>{}</lastBuildDate>\n", now_rfc2822));

    // Items
    for item in &items {
        let link = format!(
            "https://{domain}/{slug}/blog/{post_slug}",
            domain = domain,
            slug = dir.slug,
            post_slug = item.slug,
        );

        // Description: use excerpt/meta_description, fall back to truncated content
        let description: String = item
            .excerpt_or_description
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                item.content.as_ref().map(|c| {
                    // Truncate to ~500 chars for RSS description
                    let stripped = strip_html(c);
                    if stripped.len() > 500 {
                        let truncated: String = stripped.chars().take(497).collect();
                        format!("{}...", truncated)
                    } else {
                        stripped
                    }
                })
            })
            .unwrap_or_default();

        // Author name or empty
        let author = item.author_or_business.as_deref().unwrap_or("");
        let category = item.first_tag_or_keyword.as_deref().unwrap_or("");

        // Datetime: prefer created_at; if none, use current time
        let pub_date = item
            .created_at
            .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S %z").to_string())
            .unwrap_or_else(|| now_rfc2822.clone());

        xml.push_str("  <item>\n");
        xml.push_str(&format!("    <title>{}</title>\n", esc_xml(&item.title)));
        xml.push_str(&format!("    <link>{}</link>\n", esc_xml(&link)));
        xml.push_str(&format!("    <guid>{}</guid>\n", esc_xml(&link)));
        xml.push_str(&format!("    <pubDate>{}</pubDate>\n", pub_date));
        xml.push_str(&format!("    <description>{}</description>\n", esc_xml(&description)));
        if !author.is_empty() {
            xml.push_str(&format!("    <author>{}</author>\n", esc_xml(author)));
        }
        if !category.is_empty() {
            xml.push_str(&format!("    <category>{}</category>\n", esc_xml(category)));
        }
        xml.push_str("  </item>\n");
    }

    xml.push_str("</channel>\n");
    xml.push_str("</rss>\n");

    Ok(([("content-type", "application/rss+xml; charset=utf-8")], xml))
}

/// Simple HTML tag stripper for RSS descriptions.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                // Collapse multiple whitespace into single space
                if ch.is_whitespace() {
                    if !out.ends_with(' ') {
                        out.push(' ');
                    }
                } else {
                    out.push(ch);
                }
            }
            _ => {}
        }
    }
    out.trim().to_string()
}

/// XML-escape a string for safe inclusion in RSS output.
fn esc_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
