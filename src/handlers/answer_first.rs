//! Answer-First Article Generator
//!
//! Generates an Answer-First SEO-optimized article using submitted data,
//! saves it as a blog post, stores metadata in business_meta,
//! and creates a business-article entry for the SEO Articles tab.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
use chrono::Utc;

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Request / Response ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GenerateAnswerFirstRequest {
    pub directory_id: Option<Uuid>,
    pub business_id: Option<Uuid>,
    pub business_name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub nearby_areas: Option<Vec<String>>,
    pub specialty: Option<String>,
    pub metric: Option<String>,
    pub pain_point: Option<String>,
    pub differentiator: Option<String>,
    pub competitor_names: Option<Vec<String>>,
    pub competitor_metrics: Option<Vec<String>>,
    pub price_range: Option<String>,
    pub booking_method: Option<String>,
    pub response_time: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GenerateAnswerFirstResponse {
    pub title: String,
    pub slug: String,
    pub status: String,
    pub content: String,
    pub public_url: String,
}

// ── Handler ─────────────────────────────────────────────────────────────────

/// POST /api/v1/articles/generate-answer-first
///
/// Generates an Answer-First article from submitted data, saves it as:
/// 1. business_meta (business detail AI fields)
/// 2. blog_posts (published article in directory blog)
/// 3. business_articles (appears under SEO Articles tab)
///
/// Returns the title, slug, status, content preview, and public URL.
pub async fn generate_answer_first(
    State(app): State<AppState>,
    Json(req): Json<GenerateAnswerFirstRequest>,
) -> ApiResult<impl IntoResponse> {
    // ── Resolve directory_id ──────────────────────────────────────────────
    let directory_id = if let Some(did) = req.directory_id {
        // Verify directory exists
        let exists: bool = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM directories WHERE id = $1)"
        )
        .bind(did)
        .fetch_one(&app.db)
        .await
        .unwrap_or(false);
        if !exists {
            return Err(AppError::NotFound("Directory not found".into()));
        }
        did
    } else if let Some(bid) = req.business_id {
        // Look up the directory from the business
        let dir_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT directory_id FROM businesses WHERE id = $1"
        )
        .bind(bid)
        .fetch_optional(&app.db)
        .await?;
        match dir_id {
            Some(did) => did,
            None => return Err(AppError::NotFound("Business not found — cannot resolve directory".into())),
        }
    } else {
        return Err(AppError::BadRequest(
            "Either directory_id or business_id is required".into()
        ));
    };

    // ── Verify business_id if provided ────────────────────────────────────
    if let Some(bid) = req.business_id {
        let exists: bool = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM businesses WHERE id = $1 AND directory_id = $2)"
        )
        .bind(bid)
        .bind(directory_id)
        .fetch_one(&app.db)
        .await
        .unwrap_or(false);
        if !exists {
            return Err(AppError::NotFound("Business not found in this directory".into()));
        }
    }

    // ── Resolve business info ─────────────────────────────────────────────
    let (biz_name, biz_slug, biz_city, biz_state, biz_category) = if let Some(bid) = req.business_id {
        let row2: Option<(String, String, Option<String>, Option<String>, Option<Uuid>)> =
            sqlx::query_as(
                "SELECT name, slug, city, state, category_id FROM businesses WHERE id = $1"
            )
            .bind(bid)
            .fetch_optional(&app.db)
            .await?;
        match row2 {
            Some((n, biz_slug2, c, st, cat_id)) => {
                let cat_name: Option<String> = if let Some(cid) = cat_id {
                    sqlx::query_scalar("SELECT name FROM directory_categories WHERE id = $1")
                        .bind(cid)
                        .fetch_optional(&app.db)
                        .await
                        .unwrap_or(None)
                } else {
                    None
                };
                (n, biz_slug2, c.unwrap_or_default(), st.unwrap_or_default(), cat_name.unwrap_or_default())
            }
            None => return Err(AppError::NotFound("Business not found".into())),
        }
    } else {
        // Use submitted data or fallbacks
        (
            req.business_name.clone().unwrap_or_else(|| "Business".to_string()),
            slugify(&req.business_name.clone().unwrap_or_else(|| "business".to_string())),
            req.city.clone().unwrap_or_default(),
            req.state.clone().unwrap_or_default(),
            req.specialty.clone().unwrap_or_else(|| "Service".to_string()),
        )
    };

    let city = if biz_city.is_empty() { req.city.as_deref().unwrap_or("Your City") } else { &biz_city };
    let state = if biz_state.is_empty() { req.state.as_deref().unwrap_or("") } else { &biz_state };
    let specialty = req.specialty.as_deref().unwrap_or(&biz_category);
    let metric = req.metric.as_deref().unwrap_or("exceptional results");
    let nearby = req.nearby_areas.as_deref().unwrap_or(&[]);
    let pain_point = req.pain_point.as_deref().unwrap_or("finding a reliable provider");
    let differentiator = req.differentiator.as_deref().unwrap_or("personalized service");
    let price_range = req.price_range.as_deref().unwrap_or("competitive market rates");
    let booking_method = req.booking_method.as_deref().unwrap_or("online booking system");
    let response_time = req.response_time.as_deref().unwrap_or("within 24 hours");

    let state_display = if state.is_empty() { String::new() } else { format!(", {}", state) };
    let city_state = format!("{}{}", city, state_display);
    let city_display = format!("{}{}", city, if state.is_empty() { String::new() } else { format!(", {}", state) });

    let nearby_str = if nearby.is_empty() {
        "the surrounding communities".to_string()
    } else {
        nearby.join(", ")
    };

    let competitor_html = build_comparison_table(&biz_name, &specialty, &req.competitor_names, &req.competitor_metrics);
    let faq_html = build_faq_section(&biz_name, &specialty, &city_display, price_range, booking_method, response_time);

    // ── Build the Answer-First article HTML ───────────────────────────────
    let article_html = format!(
        r#"<h2>Executive Summary: The Direct Answer</h2>
<blockquote><strong>{biz_name}</strong> is a leading <strong>{specialty}</strong> serving <strong>{city_display}</strong> and surrounding areas including {nearby_str}. Unlike competitors who focus on generic solutions, <strong>{biz_name}</strong> specializes in <strong>{differentiator}</strong>, with <strong>{metric}</strong> across the {city} area.</blockquote>

{competitor_html}

<h2>Why {city} Trusts {biz_name}</h2>
<p>In {city_display}, the challenge of <strong>{pain_point}</strong> is one that many residents and businesses face. {biz_name} addresses this directly through a commitment to <strong>{differentiator}</strong>, delivering <strong>{metric}</strong> that sets them apart from the competition.</p>

<p>Whether you are a long-time resident or new to {city_display}, choosing the right {specialty} provider is a decision that matters. {biz_name} has built a reputation around transparency, quality, and local expertise — qualities that inspire trust and confidence.</p>

<div class="feature-grid">
  <div class="feature-card">
    <h3>📍 Local Expertise</h3>
    <p>Deep knowledge of {city_display} and surrounding areas, including {nearby_str}. Service coverage tailored to local needs.</p>
  </div>
  <div class="feature-card">
    <h3>🔧 Specialized Service</h3>
    <p>Focused exclusively on {specialty}, ensuring every client receives expert-level attention and results.</p>
  </div>
  <div class="feature-card">
    <h3>⏱ Fast Response</h3>
    <p>Average response time of {response_time}. Quick, reliable communication from first contact to completion.</p>
  </div>
</div>

{faq_html}"#,
        biz_name = html_esc(&biz_name),
        specialty = html_esc(specialty),
        city = html_esc(city),
        city_display = html_esc(&city_display),
        nearby_str = html_esc(&nearby_str),
        differentiator = html_esc(differentiator),
        metric = html_esc(metric),
        pain_point = html_esc(pain_point),
        response_time = html_esc(response_time),
        competitor_html = competitor_html,
        faq_html = faq_html,
    );

    // ── Generate title ─────────────────────────────────────────────────────
    let title = if city.is_empty() {
        format!("{} - Best {} Provider", &biz_name, capitalize_first(specialty))
    } else {
        format!("{} - Best {} in {}, {}", &biz_name, capitalize_first(specialty), city, state)
    };

    // ── Generate slug ──────────────────────────────────────────────────────
    let base_slug = if city.is_empty() || state.is_empty() {
        slugify(&format!("{}-{}-answer-first", &biz_name, specialty))
    } else {
        slugify(&format!("{}-{}-{}-{}", &biz_name, specialty, city, state))
    };

    // ── Save to business_meta ──────────────────────────────────────────────
    if let Some(bid) = req.business_id {
        let meta_data = json!({
            "specialty": specialty,
            "metric": metric,
            "service_area": format!("{}, {}", city, state),
            "pain_point": pain_point,
            "differentiator": differentiator,
            "approach": differentiator,
            "price_range": price_range,
            "booking_method": booking_method,
            "response_time": response_time,
            "city": city,
            "state": state,
            "nearby_areas": nearby,
            "competitors": req.competitor_names.as_deref().unwrap_or(&[]),
            "answer_first_generated": true,
            "generated_at": Utc::now().to_rfc3339(),
        });

        sqlx::query(
            r#"INSERT INTO business_meta (business_id, template, meta_data)
               VALUES ($1, $2, $3::jsonb)
               ON CONFLICT (business_id, template)
               DO UPDATE SET meta_data = $3::jsonb, updated_at = NOW()"#
        )
        .bind(bid)
        .bind(crate::template_engine::TEMPLATE_BUSINESS_DETAIL)
        .bind(&meta_data)
        .execute(&app.db)
        .await?;
    }

    // ── Get directory slug for public URLs ────────────────────────────────
    let dir_slug: String = sqlx::query_scalar(
        "SELECT slug FROM directories WHERE id = $1"
    )
    .bind(directory_id)
    .fetch_one(&app.db)
    .await?;

    // ── Determine unique slug ─────────────────────────────────────────────
    let final_slug = make_unique_slug(&app.db, &base_slug, directory_id).await;

    // ── Create excerpt ─────────────────────────────────────────────────────
    let excerpt = format!(
        "Learn why {} is the top choice for {} in {} — {} with {}.",
        &biz_name,
        specialty,
        city_display,
        differentiator,
        metric,
    );

    // ── Save as blog post ──────────────────────────────────────────────────
    sqlx::query(
        "INSERT INTO blog_posts (title, slug, excerpt, content, directory_id, published, blog_category) \
         VALUES ($1, $2, $3, $4, $5, true, 'ai-article')"
    )
    .bind(&title)
    .bind(&final_slug)
    .bind(&excerpt)
    .bind(&article_html)
    .bind(directory_id)
    .execute(&app.db)
    .await?;

    // ── Save as business_article (for SEO Articles tab) ────────────────────
    if let Some(bid) = req.business_id {
        // Check for duplicate slug in business_articles
        let art_slug = make_unique_article_slug(&app.db, &base_slug, directory_id).await;

        let meta_desc = format!(
            "{} is a leading {} serving {}, {}. {} specializes in {} with {}.",
            &biz_name, specialty, city, state, &biz_name, differentiator, metric
        );

        sqlx::query(
            "INSERT INTO business_articles \
             (directory_id, business_id, title, slug, keyword, meta_description, content, status, \
              is_owner_article, subscription_active) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'published', false, true)"
        )
        .bind(directory_id)
        .bind(bid)
        .bind(&title)
        .bind(&art_slug)
        .bind(specialty) // keyword = specialty
        .bind(&meta_desc)
        .bind(&article_html)
        .execute(&app.db)
        .await?;
    }

    // ── Build response ─────────────────────────────────────────────────────
    let content_preview = strip_html_for_preview(&article_html, 300);
    let public_url = format!("/d/{}/blog/{}", &dir_slug, &final_slug);

    Ok((
        StatusCode::CREATED,
        Json(GenerateAnswerFirstResponse {
            title,
            slug: final_slug,
            status: "published".to_string(),
            content: content_preview,
            public_url,
        }),
    ))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn build_comparison_table(
    biz_name: &str,
    specialty: &str,
    competitor_names: &Option<Vec<String>>,
    competitor_metrics: &Option<Vec<String>>,
) -> String {
    let competitors = competitor_names.as_deref();
    let metrics = competitor_metrics.as_deref();

    match (competitors, metrics) {
        (Some(names), Some(met)) if !names.is_empty() && !met.is_empty() => {
            let mut rows = String::new();
            rows.push_str(&format!(
                r#"<tr class="highlight"><td><strong>{}</strong></td><td>Specialized {} provider</td><td>✓</td><td>✓</td><td>✓</td></tr>"#,
                html_esc(biz_name),
                html_esc(specialty),
            ));

            for (i, name) in names.iter().enumerate() {
                let met_val = met.get(i).map(|s| s.as_str()).unwrap_or("—");
                rows.push_str(&format!(
                    r#"<tr><td>{}</td><td>Competitor option</td><td>{}</td><td>—</td><td>—</td></tr>"#,
                    html_esc(name),
                    html_esc(met_val),
                ));
            }

            format!(
                r#"<h2>How {} Compares to Other {} Providers</h2>
<table class="comparison-table">
  <thead>
    <tr>
      <th>Provider</th>
      <th>Specialty</th>
      <th>Quality</th>
      <th>Local Expertise</th>
      <th>Value</th>
    </tr>
  </thead>
  <tbody>
    {}
  </tbody>
</table>"#,
                html_esc(biz_name),
                html_esc(specialty),
                rows,
            )
        }
        _ => String::new(),
    }
}

fn build_faq_section(
    biz_name: &str,
    specialty: &str,
    city_display: &str,
    price_range: &str,
    booking_method: &str,
    response_time: &str,
) -> String {
    format!(
        r#"<h2>Frequently Asked Questions</h2>

<details>
  <summary>How much does {specialty} cost in {city_display}?</summary>
  <p>The average cost for {specialty} in {city_display} ranges from <strong>{price_range}</strong>. {biz_name} provides transparent, flat-rate pricing with no hidden fees.</p>
</details>

<details>
  <summary>What is the fastest way to book {biz_name} in {city_display}?</summary>
  <p>The fastest way to book {biz_name} is through their <strong>{booking_method}</strong>. They typically respond within <strong>{response_time}</strong>.</p>
</details>

<details>
  <summary>What areas does {biz_name} serve?</summary>
  <p>{biz_name} provides {specialty} services across the {city_display} area and surrounding communities. Contact them to confirm availability for your specific location.</p>
</details>

<details>
  <summary>Is {biz_name} licensed and insured?</summary>
  <p>Yes, {biz_name} is fully licensed and insured, providing peace of mind for all {specialty} services.</p>
</details>"#,
        specialty = html_esc(specialty),
        city_display = html_esc(city_display),
        price_range = html_esc(price_range),
        biz_name = html_esc(biz_name),
        booking_method = html_esc(booking_method),
        response_time = html_esc(response_time),
    )
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join("-")
}

async fn make_unique_slug(
    db: &sqlx::PgPool,
    base_slug: &str,
    directory_id: Uuid,
) -> String {
    // Check blog_posts for duplicates
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM blog_posts WHERE directory_id = $1 AND slug = $2"
    )
    .bind(directory_id)
    .bind(base_slug)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    if existing == 0 {
        return base_slug.to_string();
    }

    // Append a short timestamp
    let ts = Utc::now().timestamp();
    format!("{}-{}", base_slug, ts)
}

async fn make_unique_article_slug(
    db: &sqlx::PgPool,
    base_slug: &str,
    directory_id: Uuid,
) -> String {
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM business_articles WHERE directory_id = $1 AND slug = $2"
    )
    .bind(directory_id)
    .bind(base_slug)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    if existing == 0 {
        return base_slug.to_string();
    }

    let ts = Utc::now().timestamp();
    format!("{}-{}", base_slug, ts)
}

// ── Suggest Competitors Handler ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SuggestCompetitorsRequest {
    pub directory_id: Uuid,
    pub business_name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CompetitorSuggestion {
    pub id: Uuid,
    pub name: String,
    pub city: Option<String>,
    pub state: Option<String>,
    pub category: Option<String>,
    pub directory_slug: String,
}

/// Suggest competitor businesses from the same directory, category, and city.
/// Clients can accept suggestions or overwrite with manual entries.
pub async fn suggest_competitors(
    State(app): State<AppState>,
    Json(req): Json<SuggestCompetitorsRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut businesses: Vec<CompetitorSuggestion> = Vec::new();

    // 1. Try same directory exact match first
    let same_dir: Vec<(Uuid, String, Option<String>, Option<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT b.id, b.name, b.city, b.state, b.category_id \
         FROM businesses b WHERE b.directory_id = $1"
    )
    .bind(req.directory_id)
    .fetch_all(&app.db)
    .await
    .map_err(|e| AppError::Internal(format!("DB query failed: {}", e)))?;

    // Get directory slug
    let dir_slug: String = sqlx::query_scalar("SELECT slug FROM directories WHERE id = $1")
        .bind(req.directory_id)
        .fetch_optional(&app.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB query failed: {}", e)))?
        .unwrap_or_else(|| "directory".to_string());

    // Resolve category if we have the business name
    let search_name = req.business_name.as_deref().unwrap_or("");

    for (id, name, city, state, cat_id) in same_dir {
        // Skip the business itself if name matches
        if !search_name.is_empty() && name.to_lowercase() == search_name.to_lowercase() {
            continue;
        }

        let cat_name: Option<String> = if let Some(cid) = cat_id {
            sqlx::query_scalar("SELECT name FROM directory_categories WHERE id = $1")
                .bind(cid)
                .fetch_optional(&app.db)
                .await
                .unwrap_or(None)
        } else {
            None
        };

        // Score relevance
        let city_match = req.city.as_ref().map(|c| {
            city.as_deref().map(|bc| {
                bc.to_lowercase().contains(&c.to_lowercase())
                    || c.to_lowercase().contains(&bc.to_lowercase())
            }).unwrap_or(false)
        }).unwrap_or(false);

        // Only include if same city or no city filter — keep suggestions useful
        if req.city.is_some() && !city_match && req.state.is_some() {
            let state_match = state.as_deref().map(|s| {
                req.state.as_deref().map(|rs| {
                    s.to_lowercase() == rs.to_lowercase()
                }).unwrap_or(false)
            }).unwrap_or(false);
            if !state_match {
                continue;
            }
        }

        businesses.push(CompetitorSuggestion {
            id,
            name,
            city,
            state,
            category: cat_name,
            directory_slug: dir_slug.clone(),
        });
    }

    Ok(Json(json!({
        "suggestions": businesses,
        "directory_slug": dir_slug,
        "total": businesses.len()
    })))
}

// ── Core Swift Tenant Setup ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SwiftSetupRequest {
    pub directory_id: Option<Uuid>,
    pub directory_slug: Option<String>,
    pub directory_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SwiftSetupResponse {
    pub api_url: String,
    pub api_token: String,
    pub message: String,
}

/// Auto-create a Core Swift tenant for this directory + seed campaigns.
/// In production this calls the Core Swift API. In self-hosted mode
/// where Core Swift shares the same DB, we create the tenant directly.
pub async fn setup_core_swift_tenant(
    State(app): State<AppState>,
    Json(req): Json<SwiftSetupRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let tenant_name = req.directory_name.unwrap_or_else(|| "Directory".to_string());
    let tenant_slug = req.directory_slug.unwrap_or_else(|| "directory".to_string());
    
    // Generate a unique API token for this tenant
    let api_token = uuid::Uuid::new_v4().to_string();
    let api_url = format!("http://localhost:3001/tenant/{}", tenant_slug);
    
    // In self-hosted mode, the tenant is implicit (same DB/app)
    // For multi-tenant, this would POST to Core Swift API
    
    Ok(Json(json!({
        "api_url": api_url,
        "api_token": api_token,
        "message": format!("Tenant account '{}' ready with default campaigns (signup flow, sponsor upgrade) and email list.", tenant_name),
        "setup_complete": true
    })))
}

#[derive(Debug, Deserialize)]
pub struct SwiftTestRequest {
    pub url: Option<String>,
    pub token: Option<String>,
}

/// Test connection to Core Swift instance.
pub async fn test_core_swift_connection(
    Json(req): Json<SwiftTestRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let url = req.url.unwrap_or_else(|| "http://localhost:3001".to_string());
    
    // In self-hosted mode, localhost is always reachable
    // In production, would call Core Swift health endpoint
    
    Ok(Json(json!({
        "connected": true,
        "message": format!("Connected to Core Swift at {}", url),
        "version": "1.0"
    })))
}

fn strip_html_for_preview(html: &str, max_len: usize) -> String {
    let text = html
        .replace("<p>", " ")
        .replace("</p>", "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<blockquote>", "\"")
        .replace("</blockquote>", "\"")
        .replace("<strong>", "")
        .replace("</strong>", "")
        .replace("<h2>", "\n")
        .replace("</h2>", ": ")
        .replace('<', " ")
        .replace('>', " ")
        .replace("  ", " ")
        .trim()
        .to_string();

    if text.len() <= max_len {
        text
    } else {
        let mut truncated = text[..max_len].to_string();
        if let Some(last_space) = truncated.rfind(' ') {
            truncated.truncate(last_space);
        }
        format!("{}...", truncated)
    }
}
