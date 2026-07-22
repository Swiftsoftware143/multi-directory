//! Blog SEO handlers: Google News Sitemap, RSS Feed, and Related Articles.
//!
//! - `GET /public/directories/{slug}/news-sitemap.xml` — Google News sitemap (last 48h)
//! - `GET /public/directories/{slug}/blog/feed.xml` — RSS 2.0 feed for blog posts

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::AppState;
use crate::error::ApiResult;

// ── News Sitemap Helpers ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, sqlx::FromRow)]
struct NewsSitemapItem {
    title: String,
    slug: String,
    created_at: Option<DateTime<Utc>>,
    source_type: String, // "blog" or "article"
}

/// GET /public/directories/{slug}/news-sitemap.xml
///
/// Google News sitemap: includes blog_posts and business_articles
/// published within the last 48 hours.
pub async fn news_sitemap(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    // 1. Fetch directory
    let dir = sqlx::query!(
        r#"SELECT id, name, description, slug, url_value, custom_domain
           FROM directories WHERE slug = $1"#,
        slug
    )
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| crate::error::AppError::NotFound("Directory not found".into()))?;

    // 2. Determine domain
    let domain = if let Some(ref cd) = dir.custom_domain {
        cd.clone()
    } else {
        let uv = dir.url_value.as_deref().unwrap_or(&dir.slug);
        format!("{}.{}", uv, s.config.base_domain)
    };

    // 3. Fetch items published within the last 48 hours from both tables
    //    Google News sitemap requires content within the last 48 hours.
    let items: Vec<NewsSitemapItem> = sqlx::query_as::<_, NewsSitemapItem>(
        r#"
        SELECT
            bp.title,
            bp.slug,
            COALESCE(bp.scheduled_at, bp.created_at) AS created_at,
            'blog' AS source_type
        FROM blog_posts bp
        WHERE bp.directory_id = $1
          AND bp.status = 'published'
          AND (bp.published = true OR bp.published IS NULL)
          AND COALESCE(bp.scheduled_at, bp.created_at) >= NOW() - INTERVAL '48 hours'
        UNION ALL
        SELECT
            ba.title,
            ba.slug,
            ba.created_at,
            'article' AS source_type
        FROM business_articles ba
        WHERE ba.directory_id = $1
          AND ba.status = 'published'
          AND ba.created_at >= NOW() - INTERVAL '48 hours'
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .bind(dir.id)
    .fetch_all(&s.db)
    .await?;

    // 4. Build the Google News sitemap XML
    let mut xml = String::with_capacity(4096);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"\n");
    xml.push_str("        xmlns:news=\"http://www.google.com/schemas/sitemap-news/0.9\">\n");

    for item in &items {
        let link = format!(
            "https://{domain}/api/v1/d/{slug}/blog/{post_slug}",
            domain = domain,
            slug = dir.slug,
            post_slug = item.slug,
        );

        let pub_date = item
            .created_at
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

        xml.push_str("  <url>\n");
        xml.push_str(&format!("    <loc>{}</loc>\n", esc_xml(&link)));
        xml.push_str("    <news:news>\n");
        xml.push_str("      <news:publication>\n");
        xml.push_str(&format!("        <news:name>{}</news:name>\n", esc_xml(&dir.name)));
        xml.push_str("        <news:language>en</news:language>\n");
        xml.push_str("      </news:publication>\n");
        xml.push_str(&format!("      <news:publication_date>{}</news:publication_date>\n", pub_date));
        xml.push_str(&format!("      <news:title>{}</news:title>\n", esc_xml(&item.title)));
        xml.push_str("    </news:news>\n");
        xml.push_str("  </url>\n");
    }

    xml.push_str("</urlset>\n");

    Ok(([("content-type", "application/xml; charset=utf-8")], xml))
}

// ── RSS Feed for Blogs ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, sqlx::FromRow)]
struct RssFeedItem {
    title: String,
    slug: String,
    excerpt: Option<String>,
    content: Option<String>,
    blog_category: Option<String>,
    tags: Option<Vec<String>>,
    created_at: Option<DateTime<Utc>>,
    scheduled_at: Option<chrono::NaiveDateTime>,
}

/// GET /public/directories/{slug}/blog/feed.xml
///
/// Standard RSS 2.0 feed for the directory's blog posts (not business articles).
/// Includes the last 50 published blog posts.
pub async fn blog_rss_feed(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    // 1. Fetch directory
    let dir = sqlx::query!(
        r#"SELECT id, name, description, slug, url_value, custom_domain
           FROM directories WHERE slug = $1"#,
        slug
    )
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| crate::error::AppError::NotFound("Directory not found".into()))?;

    // 2. Determine domain
    let domain = if let Some(ref cd) = dir.custom_domain {
        cd.clone()
    } else {
        let uv = dir.url_value.as_deref().unwrap_or(&dir.slug);
        format!("{}.{}", uv, s.config.base_domain)
    };

    // 3. Fetch last 50 published blog posts
    let items: Vec<RssFeedItem> = sqlx::query_as::<_, RssFeedItem>(
        r#"
        SELECT
            bp.title,
            bp.slug,
            bp.excerpt,
            bp.content,
            bp.blog_category,
            bp.tags,
            bp.created_at,
            bp.scheduled_at
        FROM blog_posts bp
        WHERE bp.directory_id = $1
          AND bp.status = 'published'
          AND (bp.published = true OR bp.published IS NULL)
          AND (bp.scheduled_at IS NULL OR bp.scheduled_at <= NOW())
        ORDER BY COALESCE(bp.scheduled_at::timestamptz, bp.created_at) DESC
        LIMIT 50
        "#,
    )
    .bind(dir.id)
    .fetch_all(&s.db)
    .await?;

    // 4. Build RSS 2.0 XML
    let now_rfc2822 = Utc::now().format("%a, %d %b %Y %H:%M:%S %z").to_string();
    let dir_description = dir.description.as_deref().unwrap_or("").to_string();
    let channel_description = if dir_description.is_empty() {
        format!("Latest blog posts from {}", esc_xml(&dir.name))
    } else {
        esc_xml(&dir_description)
    };

    let feed_url = format!(
        "https://{domain}/public/directories/{slug}/blog/feed.xml",
        domain = domain,
        slug = dir.slug,
    );

    let mut xml = String::with_capacity(8192);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n");
    xml.push_str("<channel>\n");

    // Channel metadata
    xml.push_str(&format!(
        "  <title>{} Blog</title>\n",
        esc_xml(&dir.name)
    ));
    xml.push_str(&format!(
        "  <link>https://{domain}/api/v1/d/{slug}/blog</link>\n",
        domain = domain,
        slug = dir.slug,
    ));
    xml.push_str(&format!("  <description>{}</description>\n", channel_description));
    xml.push_str("  <language>en-us</language>\n");
    xml.push_str(&format!("  <lastBuildDate>{}</lastBuildDate>\n", now_rfc2822));
    xml.push_str(&format!(
        "  <atom:link href=\"{}\" rel=\"self\" type=\"application/rss+xml\"/>\n",
        esc_xml(&feed_url)
    ));

    // Items
    for item in &items {
        let link = format!(
            "https://{domain}/api/v1/d/{slug}/blog/{post_slug}",
            domain = domain,
            slug = dir.slug,
            post_slug = item.slug,
        );

        // Description: use excerpt, fallback to truncated content
        let description: String = item
            .excerpt
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                item.content.as_ref().map(|c| {
                    let stripped = strip_html(c);
                    if stripped.len() > 200 {
                        let truncated: String = stripped.chars().take(197).collect();
                        format!("{}...", truncated)
                    } else {
                        stripped
                    }
                })
            })
            .unwrap_or_default();

        // Use scheduled_at if available, else created_at
        let pub_date = item
            .scheduled_at
            .map(|d| {
                // Convert NaiveDateTime to DateTime<Utc> for formatting
                let dt: DateTime<Utc> = DateTime::from_naive_utc_and_offset(d, Utc);
                dt.format("%a, %d %b %Y %H:%M:%S %z").to_string()
            })
            .or_else(|| {
                item.created_at
                    .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S %z").to_string())
            })
            .unwrap_or_else(|| now_rfc2822.clone());

        // Categories: combine blog_category + tags
        let mut categories: Vec<&str> = Vec::new();
        if let Some(ref cat) = item.blog_category {
            if !cat.is_empty() {
                categories.push(cat);
            }
        }
        if let Some(ref tags) = item.tags {
            for tag in tags {
                if !tag.is_empty() && !categories.contains(&tag.as_str()) {
                    categories.push(tag);
                }
            }
        }

        xml.push_str("  <item>\n");
        xml.push_str(&format!("    <title>{}</title>\n", esc_xml(&item.title)));
        xml.push_str(&format!("    <link>{}</link>\n", esc_xml(&link)));
        xml.push_str(&format!("    <guid>{}</guid>\n", esc_xml(&link)));
        xml.push_str(&format!("    <pubDate>{}</pubDate>\n", pub_date));
        xml.push_str(&format!(
            "    <description><![CDATA[{}]]></description>\n",
            description
        ));
        for cat in &categories {
            xml.push_str(&format!("    <category>{}</category>\n", esc_xml(cat)));
        }
        xml.push_str("  </item>\n");
    }

    xml.push_str("</channel>\n");
    xml.push_str("</rss>\n");

    Ok(([("content-type", "application/rss+xml; charset=utf-8")], xml))
}

// ── Helper Functions ─────────────────────────────────────────────────────────

/// Simple HTML tag stripper for RSS descriptions.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
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

/// XML-escape a string for safe inclusion in XML output.
fn esc_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
