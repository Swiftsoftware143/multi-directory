//! Public blog page rendering — serves HTML blog listing and individual posts
//! using the directory's template.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::models::directory::Directory;
use crate::models::directory::{BlogPost, AuthorProfile};
use crate::template_engine;
use crate::tracking_script;

/// GET /api/v1/directories/:slug/blog — public blog listing page
pub async fn render_blog_list(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let posts = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at,
                focus_keyword, meta_title, meta_description, canonical_url, robots_meta,
                featured_image_url, featured_image_alt, schema_type, author_id,
                service_id, location_id, scheduled_at, template_id, template_data,
                is_master, master_post_id, blog_category, tags,
                feature_image, feature_video, media_json
         FROM blog_posts
         WHERE directory_id = $1 AND (published = true OR (scheduled_at IS NOT NULL AND scheduled_at <= NOW()))
         ORDER BY COALESCE(scheduled_at, created_at) DESC
         LIMIT 50"
    )
    .bind(directory.id)
    .fetch_all(&s.db)
    .await?;

    let posts_html: Vec<String> = posts.iter().map(|post| {
        let title = &post.title;
        let ps = post.slug.as_deref().unwrap_or("post");
        let excerpt = post.excerpt.as_deref().unwrap_or("");
        let date = post.scheduled_at
            .map(|d| d.format("%B %d, %Y").to_string())
            .unwrap_or_else(|| post.created_at.map(|d| d.format("%B %d, %Y").to_string()).unwrap_or_default());
        format!(
            "<article class=\"blog-card\"><a href=\"/api/v1/d/{slug}/blog/{ps}\"><h2>{title}</h2></a><p class=\"post-meta\">{date}</p><p>{excerpt}</p></article>",
            slug = slug,
            ps = ps,
            title = esc(title),
            date = esc(&date),
            excerpt = esc(excerpt),
        )
    }).collect();

    let template_id = directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let ctx = template_engine::build_template_context(
        &dir_val,
        &serde_json::Value::Null,
        &serde_json::Value::Null,
        None,
        None,
    );

    let rss_link = format!(
        "<p class=\"blog-feed-link\"><a href=\"/public/directories/{slug}/blog/feed.xml\" class=\"rss-link\">Subscribe to RSS Feed</a></p>",
        slug = slug,
    );

    let blog_section = format!(
        "<div class=\"blog-list\"><h1>Blog - {name}</h1>{rss}{posts}</div>",
        name = esc(&directory.name),
        rss = rss_link,
        posts = posts_html.join("\n"),
    );

    let full_html = engine.render_blog_page(template_id, &ctx, &blog_section)
        .map_err(|e| AppError::Internal(e))?;

    let mut output = crate::tracking_script::inject_tracking_script(&full_html);
    // Apply custom head/body/footer injections
    let dir = &directory;
    if let Some(ref hi) = dir.head_injection {
        if !hi.trim().is_empty() {
            output = output.replace("</head>", &format!("\n{}\n</head>", crate::template_engine::sanitize_html(hi)));
        }
    }
    if let Some(ref bi) = dir.body_injection {
        if !bi.trim().is_empty() {
            output = output.replace("<body", &format!("\n{}\n<body", crate::template_engine::sanitize_html(bi)));
        }
    }
    if let Some(ref fi) = dir.footer_injection {
        if !fi.trim().is_empty() {
            output = output.replace("</body>", &format!("\n{}\n</body>", crate::template_engine::sanitize_html(fi)));
        }
    }
    // Inject survey widget if onboarding_survey is enabled
    if let Some(ref fc) = dir.feature_config {
        if fc.get("onboarding_survey").and_then(|v| v.as_bool()).unwrap_or(false) {
            let survey_tag = "<script src=\"/survey-widget.js\"></script>";
            output = output.replace("</head>", &format!("\n{}\n</head>", survey_tag));
        }
    }
    Ok(axum::response::Html(output))
}

/// GET /api/v1/directories/:slug/blog/:post_slug — public single blog post
pub async fn render_blog_post(
    State(s): State<AppState>,
    Path((slug, post_slug)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let directory = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let post = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at,
                focus_keyword, meta_title, meta_description, canonical_url, robots_meta,
                featured_image_url, featured_image_alt, schema_type, author_id,
                service_id, location_id, scheduled_at, template_id, template_data,
                is_master, master_post_id, blog_category, tags,
                feature_image, feature_video, media_json
         FROM blog_posts
         WHERE directory_id = $1 AND slug = $2 AND (published = true OR (scheduled_at IS NOT NULL AND scheduled_at <= NOW()))"
    )
    .bind(directory.id)
    .bind(&post_slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Blog post not found".into()))?;

    let mut byline = String::from("Admin");
    if let Some(aid) = post.author_id {
        if let Ok(author) = sqlx::query_as::<_, AuthorProfile>(
            "SELECT id, directory_id, user_id, name, slug, bio, avatar_url, twitter_url, linkedin_url, website_url, role, is_active, created_at, updated_at FROM author_profiles WHERE id = $1"
        )
        .bind(aid)
        .fetch_optional(&s.db)
        .await
        {
            if let Some(a) = author {
                byline = a.name;
            }
        }
    }

    // ── Tags & Category sidebar ──
    let mut sidebar_html = String::new();

    // Category link
    if let Some(ref cat) = post.blog_category {
        if !cat.is_empty() {
            sidebar_html.push_str(&format!(
                "<div class=\"blog-sidebar-section\"><h4>Category</h4><p><a href=\"/api/v1/d/{slug}/blog?category={cat_url}\">{cat_name}</a></p></div>",
                slug = slug,
                cat_url = esc(&url_encode(cat)),
                cat_name = esc(cat),
            ));
        }
    }

    // Tags
    if let Some(ref tags) = post.tags {
        if !tags.is_empty() {
            let tag_links: Vec<String> = tags.iter().filter(|t| !t.is_empty()).map(|t| {
                format!(
                    "<a href=\"/api/v1/d/{slug}/blog?tag={tag_url}\" class=\"tag-link\">{tag_name}</a>",
                    slug = slug,
                    tag_url = esc(&url_encode(t)),
                    tag_name = esc(t),
                )
            }).collect();
            if !tag_links.is_empty() {
                sidebar_html.push_str(&format!(
                    "<div class=\"blog-sidebar-section\"><h4>Tags</h4><div class=\"tag-list\">{}</div></div>",
                    tag_links.join(" ")
                ));
            }
        }
    }

    // Share this article
    let share_url = format!(
        "https://{domain}/api/v1/d/{slug}/blog/{post_slug}",
        domain = if let Some(ref cd) = directory.custom_domain {
            cd.clone()
        } else {
            let uv = directory.url_value.as_deref().unwrap_or(&directory.slug);
            format!("{}.{}", uv, s.config.base_domain)
        },
        slug = slug,
        post_slug = post.slug.as_deref().unwrap_or("post"),
    );
    sidebar_html.push_str(&format!(
        "<div class=\"blog-sidebar-section\"><h4>Share</h4>\n
         <p><button onclick=\"navigator.clipboard.writeText('{url}').then(()=>alert('Link copied!')).catch(()=>prompt('Copy this link:', '{url}'))\" class=\"share-btn\">Copy Link</button></p>\n\
         </div>",
        url = esc(&share_url),
    ));

    // ── Related Articles ──
    // Fetch latest 4 published blog posts from same directory
    // Priority: same tags/category first, then same blog_category, then most recent
    let mut related_html = String::new();

    let related_posts: Vec<BlogPost> = sqlx::query_as::<_, BlogPost>(
        "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at,
                focus_keyword, meta_title, meta_description, canonical_url, robots_meta,
                featured_image_url, featured_image_alt, schema_type, author_id,
                service_id, location_id, scheduled_at, template_id, template_data,
                is_master, master_post_id, blog_category, tags,
                feature_image, feature_video, media_json
         FROM blog_posts
         WHERE directory_id = $1
           AND slug != $2
           AND (published = true OR (scheduled_at IS NOT NULL AND scheduled_at <= NOW()))
         ORDER BY
           CASE WHEN $3::text[] && COALESCE(tags, '{}'::text[]) THEN 0 ELSE 1 END,
           CASE WHEN $4::text = COALESCE(blog_category, '') THEN 0 ELSE 1 END,
           COALESCE(scheduled_at, created_at) DESC
         LIMIT 4"
    )
    .bind(directory.id)
    .bind(&post_slug)
    .bind(post.tags.as_ref().unwrap_or(&vec![]))
    .bind(post.blog_category.as_deref().unwrap_or(""))
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    if !related_posts.is_empty() {
        let mut cards = Vec::new();
        for rp in &related_posts {
            let rp_slug = rp.slug.as_deref().unwrap_or("post");
            let rp_date = rp.scheduled_at
                .map(|d| d.format("%b %d, %Y").to_string())
                .unwrap_or_else(|| rp.created_at.map(|d| d.format("%b %d, %Y").to_string()).unwrap_or_default());
            let rp_excerpt = rp.excerpt.as_deref()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>();

            let img_html = if let Some(ref img) = rp.featured_image_url {
                if !img.is_empty() {
                    format!(
                        "<img src=\"{}\" alt=\"{}\" loading=\"lazy\">",
                        esc(img),
                        esc(rp.featured_image_alt.as_deref().unwrap_or(""))
                    )
                } else if let Some(ref fi) = rp.feature_image {
                    format!(
                        "<img src=\"{}\" alt=\"{}\" loading=\"lazy\">",
                        esc(fi),
                        esc(&rp.title)
                    )
                } else {
                    String::new()
                }
            } else if let Some(ref fi) = rp.feature_image {
                format!(
                    "<img src=\"{}\" alt=\"{}\" loading=\"lazy\">",
                    esc(fi),
                    esc(&rp.title)
                )
            } else {
                String::new()
            };

            cards.push(format!(
                "<a href=\"/api/v1/d/{slug}/blog/{rp_slug}\" class=\"related-card\">\n\
                 {img}\n\
                 <h4>{title}</h4>\n\
                 <p>{excerpt}</p>\n\
                 <span class=\"date\">{date}</span>\n\
                 </a>",
                slug = slug,
                rp_slug = rp_slug,
                img = img_html,
                title = esc(&rp.title),
                excerpt = esc(&rp_excerpt),
                date = esc(&rp_date),
            ));
        }

        related_html.push_str(&format!(
            "<div class=\"related-articles\">\n\
n             <h3>Related Articles</h3>\n\
             <div class=\"related-grid\">\n\
             {}\n\
             </div>\n\
             </div>",
            cards.join("\n")
        ));
    }

    // ── Network (Sister Directory) Articles ──
    // Query connected_directory_ids from directories.template_config JSONB
    let mut network_html = String::new();
    if let Some(ref tc) = directory.template_config {
        if let Some(connected_ids) = tc.get("connected_directory_ids").and_then(|v| v.as_array()) {
            let mut network_items: Vec<String> = Vec::new();

            for conn_val in connected_ids {
                let conn_id_str = conn_val.as_str();
                let conn_id = conn_id_str.and_then(|s| Uuid::parse_str(s).ok());

                if let Some(cid) = conn_id {
                    // Get directory name and slug
                    if let Ok(Some(conn_dir)) = sqlx::query!(
                        r#"SELECT id, name, slug FROM directories WHERE id = $1"#,
                        cid
                    )
                    .fetch_optional(&s.db)
                    .await
                    {
                        // Get most recent blog post in same blog_category (1 per connected directory)
                        if let Ok(conn_posts) = sqlx::query_as::<_, BlogPost>(
                            "SELECT id, title, slug, excerpt, content, directory_id, published, created_at, updated_at,
                                    focus_keyword, meta_title, meta_description, canonical_url, robots_meta,
                                    featured_image_url, featured_image_alt, schema_type, author_id,
                                    service_id, location_id, scheduled_at, template_id, template_data,
                                    is_master, master_post_id, blog_category, tags,
                                    feature_image, feature_video, media_json
                             FROM blog_posts
                             WHERE directory_id = $1
                               AND (published = true OR (scheduled_at IS NOT NULL AND scheduled_at <= NOW()))
                               AND blog_category = $2
                               AND slug != $3
                             ORDER BY COALESCE(scheduled_at, created_at) DESC
                             LIMIT 1"
                        )
                        .bind(cid)
                        .bind(post.blog_category.as_deref().unwrap_or(""))
                        .bind(&post_slug)
                        .fetch_all(&s.db)
                        .await
                        {
                            if let Some(cp) = conn_posts.first() {
                                let cp_slug = cp.slug.as_deref().unwrap_or("post");
                                let dir_slug = &conn_dir.slug;
                                network_items.push(format!(
                                    "<li><a href=\"/api/v1/d/{dir_slug}/blog/{cp_slug}\">{title}</a> <span class=\"network-source\">from {dir_name}</span></li>",
                                    dir_slug = dir_slug,
                                    cp_slug = cp_slug,
                                    title = esc(&cp.title),
                                    dir_name = esc(&conn_dir.name),
                                ));
                            }
                        }
                    }
                }
            }

            if !network_items.is_empty() {
                network_html.push_str(&format!(
                    "<div class=\"network-articles\">\n\
                     <h4>Also in our Network</h4>\n\
                     <ul>\n\
                     {}\n\
                     </ul>\n\
                     </div>",
                    network_items.join("\n")
                ));
            }
        }
    }

    // ── Assemble the article HTML ──
    let template_id = directory.template.as_deref().unwrap_or(template_engine::TEMPLATE_LOCAL_BUSINESS);
    let engine = s.template_engine.lock().unwrap();

    let dir_val = serde_json::to_value(&directory).unwrap_or_default();
    let ctx = template_engine::build_template_context(
        &dir_val,
        &serde_json::Value::Null,
        &serde_json::Value::Null,
        None,
        None,
    );

    let content = strip_blockquote(&post.content);

    let date_str = post.scheduled_at
        .map(|d| d.format("%B %d, %Y").to_string())
        .unwrap_or_else(|| post.created_at.map(|d| d.format("%B %d, %Y").to_string()).unwrap_or_default());

    let back_link = format!(
        "<p><a href=\"/api/v1/d/{slug}/blog\">&larr; Back to {name} Blog</a> | <a href=\"/api/v1/directories/{slug}/render\">Back to {name}</a></p>",
        slug = slug,
        name = esc(&directory.name)
    );

    let article_html = format!(
        "<article class=\"blog-post\">\n\
         <h1>{title}</h1>\n\
         <p class=\"post-meta\">{date} by {byline}</p>\n\
         <div class=\"blog-content\">\n\
         {content}\n\
         </div>\n\
         <div class=\"blog-sidebar\">\n\
         {sidebar}\n\
         </div>\n\
         {related}\n\
         {network}\n\
         {back}\n\
         </article>",
        title = esc(&post.title),
        date = esc(&date_str),
        byline = esc(&byline),
        content = content,
        sidebar = sidebar_html,
        related = related_html,
        network = network_html,
        back = back_link,
    );

    let full_html = engine.render_blog_page(template_id, &ctx, &article_html)
        .map_err(|e| AppError::Internal(e))?;

    let mut output = crate::tracking_script::inject_tracking_script(&full_html);
    // Apply custom head/body/footer injections
    if let Some(ref hi) = directory.head_injection {
        if !hi.trim().is_empty() {
            output = output.replace("</head>", &format!("\n{}\n</head>", crate::template_engine::sanitize_html(hi)));
        }
    }
    if let Some(ref bi) = directory.body_injection {
        if !bi.trim().is_empty() {
            output = output.replace("<body", &format!("\n{}\n<body", crate::template_engine::sanitize_html(bi)));
        }
    }
    if let Some(ref fi) = directory.footer_injection {
        if !fi.trim().is_empty() {
            output = output.replace("</body>", &format!("\n{}\n</body>", crate::template_engine::sanitize_html(fi)));
        }
    }
    // Inject survey widget if onboarding_survey is enabled in feature_config
    if let Some(ref fc) = directory.feature_config {
        if fc.get("onboarding_survey").and_then(|v| v.as_bool()).unwrap_or(false) {
            let survey_tag = "<script src=\"/survey-widget.js\"></script>";
            output = output.replace("</head>", &format!("\n{}\n</head>", survey_tag));
        }
    }
    Ok(axum::response::Html(output))
}

/// URL-encode a simple string for use in query params (replaces spaces and special chars).
fn url_encode(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('#', "%23")
        .replace('&', "%26")
        .replace('?', "%3F")
        .replace('=', "%3D")
        .replace('\"', "%22")
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('{', "%7B")
        .replace('}', "%7D")
        .replace('|', "%7C")
        .replace('\\', "%5C")
        .replace('^', "%5E")
        .replace('~', "%7E")
        .replace('`', "%60")
        .replace('%', "%25")
}

fn strip_blockquote(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.starts_with("<blockquote>") && trimmed.ends_with("</blockquote>") {
        let inner = &trimmed[12..trimmed.len()-13];
        inner.trim().to_string()
    } else {
        content.to_string()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
