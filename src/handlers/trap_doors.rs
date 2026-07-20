//! Handlers: Trap Door Hyper-Niche Pages
//!
//! Adds time/day dimensions to programmatic_pages for generating hyper-specific
//! SEO landing pages targeting long-tail queries that bigger competitors miss.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::AppState;
use crate::error::{AppError, ApiResult};

// ── Models ──

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TrapDoorTemplate {
    pub id: Uuid,
    pub directory_id: Uuid,
    pub name: String,
    pub pattern: String,
    pub placeholders: Option<serde_json::Value>,
    pub is_active: Option<bool>,
    pub last_generated_at: Option<DateTime<Utc>>,
    pub page_count: Option<i32>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateTrapDoorsReq {
    pub template_id: Uuid,
    pub service_ids: Vec<Uuid>,
    pub cities: Vec<String>,
    pub day_tags: Vec<String>,
    pub time_tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrackPageEventReq {
    pub event: String,
}

#[derive(Debug, Serialize)]
pub struct GenerateResult {
    pub created: usize,
    pub skipped: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct PreviewResult {
    pub estimated_total: usize,
    pub unique_slugs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AvailableFactorsResult {
    pub cities: Vec<String>,
    pub day_tags: Vec<String>,
    pub time_tags: Vec<String>,
}

// ── Helpers ──

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != ' ', "-")
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

fn available_day_tags() -> Vec<String> {
    vec![
        "monday".into(),
        "tuesday".into(),
        "wednesday".into(),
        "thursday".into(),
        "friday".into(),
        "saturday".into(),
        "sunday".into(),
    ]
}

fn available_time_tags() -> Vec<String> {
    vec![
        "morning".into(),
        "afternoon".into(),
        "evening".into(),
        "past-9pm".into(),
    ]
}

// ── Endpoints ──

/// POST /directories/:id/trap-doors/generate
///
/// Cross-product: services × cities × days × times → programmatic_pages rows.
/// Skips existing; returns counts.
pub async fn generate_trap_doors(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Json(req): Json<GenerateTrapDoorsReq>,
) -> ApiResult<impl IntoResponse> {
    // Validate template belongs to this directory
    let tmpl = sqlx::query_as::<_, TrapDoorTemplate>(
        "SELECT * FROM trap_door_templates WHERE id=$1 AND directory_id=$2"
    )
    .bind(req.template_id)
    .bind(dir_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Template not found in this directory".into()))?;

    // Fetch directory name + services
    let dir_name: String = sqlx::query_scalar(
        "SELECT name FROM directories WHERE id=$1"
    )
    .bind(dir_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Directory".into()))?;

    let services = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directory_services WHERE directory_id=$1 AND id=ANY($2) AND is_active=true"
    )
    .bind(dir_id)
    .bind(&req.service_ids)
    .fetch_all(&s.db)
    .await?;

    let user_day_tags: Vec<String> = req.day_tags.iter().map(|d| d.to_lowercase()).collect();
    let user_time_tags: Vec<String> = req.time_tags.iter().map(|t| t.to_lowercase()).collect();

    let mut created = 0usize;
    let mut skipped = 0usize;

    for (svc_id, svc_name, svc_slug) in &services {
        for city in &req.cities {
            let city_slug = slugify(city);
            for day in &user_day_tags {
                for time in &user_time_tags {
                    let slug = tmpl.pattern
                        .replace("{service}", svc_slug)
                        .replace("{city}", &city_slug)
                        .replace("{day}", day)
                        .replace("{time}", time)
                        .trim_matches('/')
                        .to_string();

                    // Check for duplicate
                    let exists: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM programmatic_pages WHERE directory_id=$1 AND slug=$2"
                    )
                    .bind(dir_id)
                    .bind(&slug)
                    .fetch_one(&s.db)
                    .await
                    .unwrap_or(0);

                    if exists > 0 {
                        skipped += 1;
                        continue;
                    }

                    let title = format!("{} in {} Open {} {}", svc_name, city, time, day);
                    let meta_title = format!("{} in {} Open {} {} | {}",
                        svc_name, city, time, day, dir_name);
                    let meta_description = format!(
                        "Find {} providers in {} open {} on {}. Browse top-rated {} services near you.",
                        svc_name, city, time, day, svc_name
                    );
                    let h1 = format!("{} in {} — Open {} on {}", svc_name, city, time, day);

                    sqlx::query(
                        "INSERT INTO programmatic_pages \
                         (directory_id,service_id,slug,title,meta_title,meta_description,h1,\
                          day_tags,time_tags,status) \
                         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'draft')"
                    )
                    .bind(dir_id)
                    .bind(svc_id)
                    .bind(&slug)
                    .bind(&title)
                    .bind(&meta_title)
                    .bind(&meta_description)
                    .bind(&h1)
                    .bind(&[day.clone()])
                    .bind(&[time.clone()])
                    .execute(&s.db)
                    .await?;

                    created += 1;
                }
            }
        }
    }

    // Update template page_count and last_generated_at
    let total_for_tmpl: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM programmatic_pages WHERE directory_id=$1"
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    sqlx::query(
        "UPDATE trap_door_templates SET page_count=$1, last_generated_at=NOW(), updated_at=NOW() WHERE id=$2"
    )
    .bind(total_for_tmpl as i32)
    .bind(req.template_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "created": created,
        "skipped": skipped,
        "total": services.len() * req.cities.len() * user_day_tags.len() * user_time_tags.len()
    })))
}

/// POST /directories/:id/trap-doors/preview
///
/// Same cross-product logic, but only counts and samples 10 slugs without writing.
pub async fn preview_trap_doors(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Json(req): Json<GenerateTrapDoorsReq>,
) -> ApiResult<impl IntoResponse> {
    // Validate template belongs to this directory
    let tmpl = sqlx::query_as::<_, TrapDoorTemplate>(
        "SELECT * FROM trap_door_templates WHERE id=$1 AND directory_id=$2"
    )
    .bind(req.template_id)
    .bind(dir_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Template not found in this directory".into()))?;

    let services = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directory_services WHERE directory_id=$1 AND id=ANY($2) AND is_active=true"
    )
    .bind(dir_id)
    .bind(&req.service_ids)
    .fetch_all(&s.db)
    .await?;

    let user_day_tags: Vec<String> = req.day_tags.iter().map(|d| d.to_lowercase()).collect();
    let user_time_tags: Vec<String> = req.time_tags.iter().map(|t| t.to_lowercase()).collect();

    let estimated_total = services.len() * req.cities.len() * user_day_tags.len() * user_time_tags.len();

    // Build a few sample slugs
    let mut sample_slugs: Vec<String> = Vec::new();
    'outer: for (_, svc_name, svc_slug) in &services {
        for city in &req.cities {
            let city_slug = slugify(city);
            for day in &user_day_tags {
                for time in &user_time_tags {
                    if sample_slugs.len() >= 10 {
                        break 'outer;
                    }
                    let slug = tmpl.pattern
                        .replace("{service}", svc_slug)
                        .replace("{city}", &city_slug)
                        .replace("{day}", day)
                        .replace("{time}", time)
                        .trim_matches('/')
                        .to_string();
                    sample_slugs.push(slug);
                }
            }
        }
    }

    Ok(Json(json!({
        "estimated_total": estimated_total,
        "unique_slugs": sample_slugs
    })))
}

/// GET /directories/:id/trap-doors/available-factors
///
/// Returns distinct cities from the businesses table for this directory,
/// plus available day/time tag options.
pub async fn available_factors(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    // Fetch distinct cities from businesses table
    let cities: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT city FROM businesses WHERE directory_id=$1 AND city IS NOT NULL AND city != '' ORDER BY city"
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(AvailableFactorsResult {
        cities,
        day_tags: available_day_tags(),
        time_tags: available_time_tags(),
    }))
}

/// POST /programmatic-pages/:page_id/track
///
/// Increments impression, click, or conversion counter.
pub async fn track_page_event(
    State(s): State<AppState>,
    Path(page_id): Path<Uuid>,
    Json(req): Json<TrackPageEventReq>,
) -> ApiResult<impl IntoResponse> {
    let event = req.event.to_lowercase();

    match event.as_str() {
        "impression" => {
            sqlx::query("UPDATE programmatic_pages SET impressions = impressions + 1 WHERE id=$1")
                .bind(page_id)
                .execute(&s.db)
                .await?;
        }
        "click" => {
            sqlx::query("UPDATE programmatic_pages SET clicks = clicks + 1 WHERE id=$1")
                .bind(page_id)
                .execute(&s.db)
                .await?;
        }
        "conversion" => {
            sqlx::query("UPDATE programmatic_pages SET conversions = conversions + 1 WHERE id=$1")
                .bind(page_id)
                .execute(&s.db)
                .await?;
        }
        _ => return Err(AppError::Validation(format!(
            "Unknown event type '{}'. Valid: impression, click, conversion", event
        ))),
    }

    Ok(Json(json!({"tracked": event, "page_id": page_id})))
}

/// GET /directories/:id/trap-doors/pages
///
/// Enhanced list of programmatic pages for this directory, supporting
/// day_tag / time_tag / status filters.
pub async fn list_trap_door_pages(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    use sqlx::Row;

    let mut conditions = vec!["pp.directory_id=$1".to_string()];
    let mut idx = 2;

    if let Some(ref status) = params.get("status") {
        conditions.push(format!("pp.status=${}", idx));
        idx += 1;
    }
    if let Some(ref day) = params.get("day_tag") {
        conditions.push(format!("${} = ANY(pp.day_tags)", idx));
        idx += 1;
    }
    if let Some(ref time) = params.get("time_tag") {
        conditions.push(format!("${} = ANY(pp.time_tags)", idx));
        idx += 1;
    }
    if let Some(ref search) = params.get("search") {
        conditions.push(format!("(pp.title ILIKE ${} OR pp.slug ILIKE ${})", idx, idx + 1));
        idx += 2;
    }

    let where_clause = conditions.join(" AND ");

    let sql = format!(
        "SELECT pp.*, ds.name as service_name \
         FROM programmatic_pages pp \
         LEFT JOIN directory_services ds ON ds.id=pp.service_id \
         WHERE {} \
         ORDER BY pp.updated_at DESC",
        where_clause
    );

    let status_filter = params.get("status").cloned();
    let day_filter = params.get("day_tag").cloned();
    let time_filter = params.get("time_tag").cloned();
    let search_filter = params.get("search").cloned();

    let mut query = sqlx::query(&sql).bind(dir_id);

    if let Some(ref status) = status_filter {
        query = query.bind(status.clone());
    }
    if let Some(ref day) = day_filter {
        query = query.bind(day.clone());
    }
    if let Some(ref time) = time_filter {
        query = query.bind(time.clone());
    }
    if let Some(ref search) = search_filter {
        let s = search.clone();
        query = query.bind(s.clone()).bind(s);
    }

    let rows = query.fetch_all(&s.db).await?;

    let mut results: Vec<serde_json::Value> = Vec::new();
    for row in &rows {
        results.push(json!({
            "id": row.try_get::<Uuid,_>("id").unwrap_or_default(),
            "directory_id": row.try_get::<Uuid,_>("directory_id").unwrap_or_default(),
            "service_id": row.try_get::<Option<Uuid>,_>("service_id").ok().flatten(),
            "slug": row.try_get::<String,_>("slug").unwrap_or_default(),
            "title": row.try_get::<Option<String>,_>("title").ok().flatten(),
            "meta_title": row.try_get::<Option<String>,_>("meta_title").ok().flatten(),
            "meta_description": row.try_get::<Option<String>,_>("meta_description").ok().flatten(),
            "h1": row.try_get::<Option<String>,_>("h1").ok().flatten(),
            "content": row.try_get::<Option<String>,_>("content").ok().flatten(),
            "template_name": row.try_get::<Option<String>,_>("template_name").ok().flatten(),
            "status": row.try_get::<Option<String>,_>("status").ok().flatten(),
            "day_tags": row.try_get::<Option<Vec<String>>,_>("day_tags").ok().flatten(),
            "time_tags": row.try_get::<Option<Vec<String>>,_>("time_tags").ok().flatten(),
            "hour_slot": row.try_get::<Option<String>,_>("hour_slot").ok().flatten(),
            "impressions": row.try_get::<Option<i32>,_>("impressions").ok().flatten().unwrap_or(0),
            "clicks": row.try_get::<Option<i32>,_>("clicks").ok().flatten().unwrap_or(0),
            "conversions": row.try_get::<Option<i32>,_>("conversions").ok().flatten().unwrap_or(0),
            "created_at": row.try_get::<Option<DateTime<Utc>>,_>("created_at").ok().flatten(),
            "updated_at": row.try_get::<Option<DateTime<Utc>>,_>("updated_at").ok().flatten(),
            "service_name": row.try_get::<Option<String>,_>("service_name").ok().flatten(),
        }));
    }

    Ok(Json(results))
}

/// GET /directories/:id/trap-doors/analytics
///
/// Aggregated analytics for trap door pages in this directory.
pub async fn trap_door_analytics(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT \
           COUNT(*) as total_pages, \
           COALESCE(SUM(impressions),0) as total_impressions, \
           COALESCE(SUM(clicks),0) as total_clicks, \
           COALESCE(SUM(conversions),0) as total_conversions \
         FROM programmatic_pages WHERE directory_id=$1"
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let mut total_pages = 0i64;
    let mut total_impressions = 0i64;
    let mut total_clicks = 0i64;
    let mut total_conversions = 0i64;

    if let Some(row) = rows.first() {
        total_pages = row.try_get::<i64, _>("total_pages").unwrap_or(0);
        total_impressions = row.try_get::<i64, _>("total_impressions").unwrap_or(0);
        total_clicks = row.try_get::<i64, _>("total_clicks").unwrap_or(0);
        total_conversions = row.try_get::<i64, _>("total_conversions").unwrap_or(0);
    }

    let conversion_rate = if total_impressions > 0 {
        (total_conversions as f64 / total_impressions as f64 * 100.0 * 100.0).round() / 100.0
    } else {
        0.0
    };

    // Top pages by impressions
    let top_pages = sqlx::query(
        "SELECT id, slug, title, impressions, clicks, conversions \
         FROM programmatic_pages \
         WHERE directory_id=$1 \
         ORDER BY impressions DESC \
         LIMIT 20"
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let mut top_results: Vec<serde_json::Value> = Vec::new();
    for row in &top_pages {
        top_results.push(json!({
            "id": row.try_get::<Uuid,_>("id").unwrap_or_default(),
            "slug": row.try_get::<String,_>("slug").unwrap_or_default(),
            "title": row.try_get::<Option<String>,_>("title").ok().flatten(),
            "impressions": row.try_get::<Option<i32>,_>("impressions").ok().flatten().unwrap_or(0),
            "clicks": row.try_get::<Option<i32>,_>("clicks").ok().flatten().unwrap_or(0),
            "conversions": row.try_get::<Option<i32>,_>("conversions").ok().flatten().unwrap_or(0),
        }));
    }

    // Day-of-week breakdown from day_tags
    let day_breakdown: Vec<serde_json::Value> = {
        let day_counts = sqlx::query(
            "SELECT unnest(day_tags) as day, COUNT(*) as cnt \
             FROM programmatic_pages \
             WHERE directory_id=$1 AND day_tags IS NOT NULL AND array_length(day_tags,1) > 0 \
             GROUP BY day ORDER BY cnt DESC"
        )
        .bind(dir_id)
        .fetch_all(&s.db)
        .await?;

        day_counts.iter().map(|row| {
            json!({
                "day": row.try_get::<String, _>("day").unwrap_or_default(),
                "count": row.try_get::<i64, _>("cnt").unwrap_or(0),
            })
        }).collect()
    };

    Ok(Json(json!({
        "summary": {
            "total_pages": total_pages,
            "total_impressions": total_impressions,
            "total_clicks": total_clicks,
            "total_conversions": total_conversions,
            "conversion_rate": conversion_rate,
        },
        "top_pages": top_results,
        "day_breakdown": day_breakdown,
    })))
}

/// Generate FAQPage schema JSON-LD tailored to this trap door page.
fn generate_faq_schema(
    service_name: &str,
    city: &str,
    day_tags: &[String],
    time_tags: &[String],
    dir_name: &str,
    _dir_slug: &str,
) -> String {
    let is_plumbing = service_name.to_lowercase().contains("plumber")
        || service_name.to_lowercase().contains("plumbing");
    let is_weekend = day_tags.iter().any(|d| {
        let d = d.to_lowercase();
        d == "saturday" || d == "sunday"
    });

    let city_display = if city.is_empty() { "your area" } else { city };
    let service_lower = service_name.to_lowercase();
    let day_str = if day_tags.is_empty() { String::new() } else { day_tags.join(", ") };
    let time_str = if time_tags.is_empty() { String::new() } else { time_tags.join(", ") };
    let has_time = !time_str.is_empty();
    let has_day = !day_str.is_empty();

    let mut questions: Vec<serde_json::Value> = Vec::new();

    fn qa(name: &str, text: &str) -> serde_json::Value {
        json!({
            "@type": "Question",
            "name": name,
            "acceptedAnswer": {
                "@type": "Answer",
                "text": text
            }
        })
    }

    // 1. Plumbing-specific pricing question
    if is_plumbing && has_time {
        questions.push(qa(
            &format!("Do {} charge extra for {} calls in {}?", service_name, time_str, city_display),
            &format!(
                "Many {} providers do charge premium rates for {} service calls, especially for emergency work. It is always best to confirm pricing upfront. Check our directory for {} providers in {} that clearly list their {} rates.",
                service_lower, time_str, service_lower, city_display, time_str
            )
        ));
    }

    // 2. Weekend availability
    if is_weekend || has_day {
        let day_context = if is_weekend { "on weekends" } else { &format!("on {}", day_str) };
        questions.push(qa(
            &format!("Are {} available {} in {}?", service_name, day_context, city_display),
            &format!(
                "Yes, many {} providers in {} offer services {}. Availability varies by business, so we recommend checking each listing in our directory for specific hours and booking options.",
                service_lower, city_display, day_context
            )
        ));
    }

    // 3. Finding the best service
    questions.push(qa(
        &format!("How do I find the best {} in {}?", service_name, city_display),
        &format!(
            "To find the best {} in {}, start by browsing our directory. Compare providers based on reviews, years of experience, pricing transparency, and range of services offered. Pay attention to customer feedback patterns and look for businesses with consistent positive ratings. Check our {} directory for a curated list of {} providers serving {}.",
            service_lower, city_display, dir_name, service_lower, city_display
        )
    ));

    // 4. Hiring tips
    questions.push(qa(
        &format!("What should I look for when hiring {} in {}?", service_name, city_display),
        &format!(
            "When hiring {} in {}, look for proper licensing and insurance, transparent pricing, positive customer reviews, relevant experience, and clear communication. A reputable provider should offer written estimates and be willing to answer your questions. Our directory helps you compare multiple {} providers in {} side by side.",
            service_lower, city_display, service_lower, city_display
        )
    ));

    // 5. Time-specific availability
    if has_time {
        questions.push(qa(
            &format!("Are {} open {} in {}?", service_name, time_str, city_display),
            &format!(
                "Many {} providers in {} offer {} hours to accommodate different schedules. Hours can vary, so check each business listing in our directory for their specific {} availability. Some providers may require advance booking for {} appointments.",
                service_lower, city_display, time_str, time_str, time_str
            )
        ));
    }

    // 6. Booking ahead
    questions.push(qa(
        &format!("Do I need to book ahead for {} in {}?", service_name, city_display),
        &format!(
            "It depends on the provider and the time of year. Some {} in {} accept walk-ins, while others require appointments booked days or weeks in advance. For {} appointments, booking ahead is generally recommended. Browse our directory to find {} providers in {} and check their booking policies directly.",
            service_lower, city_display, time_name(&time_str), service_lower, city_display
        )
    ));

    let schema = json!({
        "@context": "https://schema.org",
        "@type": "FAQPage",
        "mainEntity": questions
    });

    format!(
        r#"<script type="application/ld+json">
{}
</script>"#,
        serde_json::to_string_pretty(&schema).unwrap_or_default()
    )
}

fn time_name(tag: &str) -> &str {
    match tag.to_lowercase().as_str() {
        "morning" => "morning",
        "afternoon" => "afternoon",
        "evening" => "evening",
        "past-9pm" | "past 9 pm" => "late-night",
        _ => tag,
    }
}

/// Generate visible FAQ accordion HTML based on the same data used for the FAQ schema.
fn generate_faq_accordion(
    service_name: &str,
    city: &str,
    day_tags: &[String],
    time_tags: &[String],
    dir_name: &str,
    _dir_slug: &str,
) -> String {
    let is_plumbing = service_name.to_lowercase().contains("plumber")
        || service_name.to_lowercase().contains("plumbing");
    let is_weekend = day_tags.iter().any(|d| {
        let d = d.to_lowercase();
        d == "saturday" || d == "sunday"
    });

    let city_display = if city.is_empty() { "your area" } else { city };
    let service_lower = service_name.to_lowercase();
    let day_str = if day_tags.is_empty() { String::new() } else { day_tags.join(", ") };
    let time_str = if time_tags.is_empty() { String::new() } else { time_tags.join(", ") };
    let has_time = !time_str.is_empty();
    let has_day = !day_str.is_empty();
    let esc_svc_dir = htmlesc(service_name);
    let esc_svc_low = htmlesc(&service_lower);
    let esc_city = htmlesc(city_display);
    let esc_dir = htmlesc(dir_name);
    let esc_time = htmlesc(&time_str);
    let esc_day = htmlesc(&day_str);

    let mut items: Vec<String> = Vec::new();

    let q_find_best = format!("How do I find the best {} in {}?", esc_svc_dir, esc_city);
    let a_find_best = format!("Start by browsing our directory of {} providers in {}. You can compare reviews, check business hours, and view contact information for each listing in our network.", esc_svc_low, esc_city);
    let q_hiring = format!("What should I look for when hiring {} in {}?", esc_svc_low, esc_city);
    let a_hiring = format!("Look for verified reviews, proper licensing, transparent pricing, and availability during your preferred hours. The {} directory makes it easy to compare {} providers side by side.", esc_dir, esc_svc_low);
    let during_time_str = if has_time { format!(" during {}", esc_time) } else { String::new() };
    let on_day_str = if has_day { format!(" on {}", esc_day) } else { String::new() };
    let a_open_hours = format!("Yes! Our directory lists {} providers in {} available{}{}. Browse our listings to find businesses with hours that match your schedule.", esc_svc_low, esc_city, during_time_str, on_day_str);
    let q_book_ahead = format!("Do I need to book ahead for {} in {}?", esc_svc_low, esc_city);
    let a_book_ahead = format!("It depends on the provider and time of day. For {} services in {} during {} hours, booking ahead is recommended. Search our directory to find providers with online booking or call-ahead options.", esc_svc_low, esc_city, if has_time { &esc_time } else { "busy" });

    items.push(format!(
        r##"<div class="faq-item">
    <button class="faq-q" onclick="toggleFaq(1)">{} <span class="faq-arrow">&#9660;</span></button>
    <div class="faq-a" id="faq-1"><p>{}</p></div>
</div>"##,
        q_find_best, a_find_best
    ));

    items.push(format!(
        r##"<div class="faq-item">
    <button class="faq-q" onclick="toggleFaq(2)">{} <span class="faq-arrow">&#9660;</span></button>
    <div class="faq-a" id="faq-2"><p>{}</p></div>
</div>"##,
        q_hiring, a_hiring
    ));

    if has_time || has_day {
        let q_open_hours = format!("Are {} open{} in {}?", esc_svc_low, during_time_str, esc_city);
        items.push(format!(
            r##"<div class="faq-item">
    <button class="faq-q" onclick="toggleFaq(3)">{} <span class="faq-arrow">&#9660;</span></button>
    <div class="faq-a" id="faq-3"><p>{}</p></div>
</div>"##,
            q_open_hours, a_open_hours
        ));
    }

    if is_weekend {
        let q_weekend = format!("Are {} available on weekends in {}?", esc_svc_low, esc_city);
        let a_weekend = format!("Yes, many {} providers in {} offer weekend services. Check the {} directory for businesses with Saturday and Sunday hours near you.", esc_svc_low, esc_city, esc_dir);
        let idx = items.len() + 1;
        items.push(format!(
            r##"<div class="faq-item">
    <button class="faq-q" onclick="toggleFaq({})">{} <span class="faq-arrow">&#9660;</span></button>
    <div class="faq-a" id="faq-{}"><p>{}</p></div>
</div>"##,
            idx, q_weekend, idx, a_weekend
        ));
    }

    if is_plumbing {
        let q_plumbing_extra = format!("Do {} charge extra for {} calls in {}?", esc_svc_low, if has_time { &esc_time } else { "emergency" }, esc_city);
        let a_plumbing_extra = format!("Some {} providers in {} may charge premium rates for {}{} calls. We recommend checking business listings in the {} directory for upfront pricing information before booking.", esc_svc_low, esc_city, if has_time { &esc_time } else { "after-hours" }, on_day_str, esc_dir);
        let idx = items.len() + 1;
        items.push(format!(
            r##"<div class="faq-item">
    <button class="faq-q" onclick="toggleFaq({})">{} <span class="faq-arrow">&#9660;</span></button>
    <div class="faq-a" id="faq-{}"><p>{}</p></div>
</div>"##,
            idx, q_plumbing_extra, idx, a_plumbing_extra
        ));
    }

    let idx = items.len() + 1;
    items.push(format!(
        r##"<div class="faq-item">
    <button class="faq-q" onclick="toggleFaq({})">{} <span class="faq-arrow">&#9660;</span></button>
    <div class="faq-a" id="faq-{}"><p>{}</p></div>
</div>"##,
        idx, q_book_ahead, idx, a_book_ahead
    ));

    let accordion_style = r##"<style>
.faq-section{margin-top:32px;padding-top:24px;border-top:2px solid #e2e8f0}
.faq-section h3{font-size:1.25rem;margin-bottom:16px;color:#0f172a}
.faq-item{border:1px solid #e2e8f0;border-radius:8px;margin-bottom:8px;overflow:hidden}
.faq-q{width:100%;padding:14px 18px;background:#f8fafc;border:none;text-align:left;font-size:.95rem;font-weight:600;cursor:pointer;display:flex;justify-content:space-between;align-items:center;transition:background .2s}
.faq-q:hover{background:#f1f5f9}
.faq-arrow{font-size:.7rem;transition:transform .2s}
.faq-a{max-height:0;overflow:hidden;transition:max-height .3s ease;background:#fff}
.faq-a p{padding:0 18px 14px;margin:0;font-size:.9rem;color:#475569;line-height:1.6}
.faq-item.active .faq-a{max-height:300px}
.faq-item.active .faq-arrow{transform:rotate(180deg)}
</style>"##;

    let accordion_js = r##"<script>
function toggleFaq(id){var el=document.getElementById('faq-'+id);if(!el)return;var item=el.closest('.faq-item');if(item)item.classList.toggle('active');}
</script>"##;

    format!(
        r##"<div class="faq-section"><h3>Frequently Asked Questions</h3>
{}
{}
{}</div>"##,
        accordion_style, accordion_js, items.join("\n")
    )
}

pub async fn serve_trap_door_page(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let page = sqlx::query(
        "SELECT pp.*, d.name as directory_name, d.slug as directory_slug \
         FROM programmatic_pages pp \
         JOIN directories d ON d.id = pp.directory_id \
         WHERE pp.slug = $1 \
         LIMIT 1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Page not found".into()))?;

    use sqlx::Row;
    let page_id: Uuid = page.try_get("id").unwrap_or_default();
    let dir_id: Uuid = page.try_get("directory_id").unwrap_or_default();
    let dir_name: String = page.try_get::<String,_>("directory_name").unwrap_or_default();
    let dir_slug: String = page.try_get::<String,_>("directory_slug").unwrap_or_default();
    let meta_title: String = page.try_get::<Option<String>, _>("meta_title").ok().flatten()
        .unwrap_or_else(|| slug.clone());
    let meta_description: String = page.try_get::<Option<String>, _>("meta_description").ok().flatten()
        .unwrap_or_default();
    let h1: String = page.try_get::<Option<String>, _>("h1").ok().flatten().unwrap_or_default();
    let content: String = page.try_get::<Option<String>, _>("content").ok().flatten().unwrap_or_default();
    let title: String = page.try_get::<Option<String>, _>("title").ok().flatten().unwrap_or_default();
    let day_tags: Vec<String> = page.try_get::<Option<Vec<String>>, _>("day_tags").ok().flatten().unwrap_or_default();
    let time_tags: Vec<String> = page.try_get::<Option<Vec<String>>, _>("time_tags").ok().flatten().unwrap_or_default();

    // Track impression asynchronously (fire-and-forget)
    let db = s.db.clone();
    let page_id_track = page_id;
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE programmatic_pages SET impressions = impressions + 1 WHERE id=$1")
            .bind(page_id_track)
            .execute(&db)
            .await;
    });

    let day_str = if day_tags.is_empty() { String::new() } else {
        format!(" on <strong>{}</strong>", day_tags.join(", "))
    };
    let time_str = if time_tags.is_empty() { String::new() } else {
        format!(" during <strong>{}</strong>", time_tags.join(", "))
    };

    // Extract service name and city from title for FAQ generation
    let service_name = &title;
    let city = "your area";

    let cta_html = format!(
        r##"<div style="margin-top:32px;padding:24px;background:linear-gradient(135deg,#0d9488,#14b8a6);border-radius:12px;text-align:center;color:#fff">
            <h3 style="margin-bottom:12px">Access Full {dn} Directory</h3>
            <p style="margin-bottom:16px">Search all {svc} providers in the {dn} area{dts} or claim your business listing.</p>
            <a href="/{dls}" style="display:inline-block;padding:12px 28px;background:#fff;color:#0d9488;border-radius:8px;font-weight:700;text-decoration:none">Browse Directory</a>
            <a href="/claim-business" style="display:inline-block;margin-left:12px;padding:12px 28px;background:rgba(255,255,255,.2);color:#fff;border-radius:8px;font-weight:700;text-decoration:none">Claim Listing &rarr;</a>
        </div>"##,
        dn = htmlesc(&dir_name),
        svc = htmlesc(&title),
        dls = htmlesc(&dir_slug),
        dts = format!("{}{}", day_str, time_str),
    );

    let content_block = if content.is_empty() {
        format!("<p>Find top-rated {} services in the {} area{}{}. Browse our directory to compare providers, read reviews, and find the right business for your needs.</p>",
            htmlesc(&title), htmlesc(&dir_name), time_str, day_str)
    } else {
        content.clone()
    };

    // Generate FAQ schema and accordion
    let faq_schema = generate_faq_schema(
        service_name,
        city,
        &day_tags,
        &time_tags,
        &dir_name,
        &dir_slug,
    );
    let faq_accordion = generate_faq_accordion(
        service_name,
        city,
        &day_tags,
        &time_tags,
        &dir_name,
        &dir_slug,
    );

    // Determine domain for OG image URL
    let og_page_id_str = &page_id.to_string();
    let base_domain = &s.config.base_domain;
    let og_image_url = format!("https://{}/public/og/trapdoor/{}", base_domain, og_page_id_str);

    let mt = htmlesc(&meta_title);
    let md = htmlesc(&meta_description);
    let sl = htmlesc(&slug);
    let hn = htmlesc(&dir_name);
    let year_str = Utc::now().format("%Y").to_string();
    let og_img = htmlesc(&og_image_url);

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{mt}</title>
    <meta name="description" content="{md}">
    <meta property="og:title" content="{mt}">
    <meta property="og:description" content="{md}">
    <meta property="og:image" content="{og_img}">
    <meta property="og:type" content="article">
    <meta property="twitter:card" content="summary_large_image">
    <meta property="twitter:title" content="{mt}">
    <meta property="twitter:description" content="{md}">
    <meta name="robots" content="index,follow">
    <link rel="canonical" href="/p/{sl}">
    {faq_schema}
    <style>
        *{{margin:0;padding:0;box-sizing:border-box}}
        body{{font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,sans-serif;background:#f8fafc;color:#1e293b;line-height:1.6}}
        .container{{max-width:800px;margin:0 auto;padding:40px 24px}}
        h1{{font-size:2rem;margin-bottom:16px;color:#0d9488}}
        .meta{{font-size:.85rem;color:#64748b;margin-bottom:24px}}
        .content{{font-size:1rem;line-height:1.8}}
        .content p{{margin-bottom:16px}}
        .content ul,.content ol{{margin:0 0 16px 24px}}
        footer{{margin-top:48px;padding-top:24px;border-top:1px solid #e2e8f0;font-size:.8rem;color:#64748b;text-align:center}}
    </style>
</head>
<body>
    <div class="container">
        <h1>{h1}</h1>
        <div class="content">{content_block}</div>
        {faq_accordion}
        {cta_html}
        <footer>
            <p>&copy; {year_str} {hn} Directory. All rights reserved.</p>
        </footer>
    </div>
</body>
</html>"##,
        mt = mt,
        md = md,
        sl = sl,
        h1 = htmlesc(&h1),
        content_block = content_block,
        faq_schema = faq_schema,
        faq_accordion = faq_accordion,
        cta_html = cta_html,
        year_str = year_str,
        hn = hn,
        og_img = og_img,
    );

    Ok((StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")], html))
}

pub async fn scheduled_generate_trap_doors(
    State(s): State<AppState>,
    Path(dir_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {

    // Fetch services, cities, day tags, time tags
    let services = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directory_services WHERE directory_id=$1 AND is_active=true"
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let cities: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT city FROM businesses WHERE directory_id=$1 AND city IS NOT NULL AND city != ''"
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let day_tags = vec![
        "monday".to_string(),
        "tuesday".to_string(),
        "wednesday".to_string(),
        "thursday".to_string(),
        "friday".to_string(),
        "saturday".to_string(),
        "sunday".to_string(),
    ];

    let time_tags = vec![
        "morning".to_string(),
        "afternoon".to_string(),
        "evening".to_string(),
        "past-9pm".to_string(),
    ];

    if services.is_empty() || cities.is_empty() {
        return Ok(Json(json!({
            "created": 0,
            "skipped": 0,
            "total": 0,
            "message": "No services or cities available to generate pages"
        })));
    }

    let mut created: usize = 0;
    let mut skipped: usize = 0;
    let mut total: usize = 0;

    for (service_id, service_name, service_slug) in &services {
        for city in &cities {
            for day in &day_tags {
                for time in &time_tags {
                    total += 1;

                    let slug = format!(
                        "{}-in-{}-open-{}-on-{}",
                        service_slug.replace(" ", "-").replace("&", "and").to_lowercase(),
                        city.to_lowercase().replace(" ", "-"),
                        time,
                        day
                    );

                    let title = format!(
                        "{} in {} Open {} on {}",
                        capitalize_first(service_name),
                        city,
                        time_label(time),
                        capitalize_first(day)
                    );

                    let meta_description = format!(
                        "Looking for {} in {} open {} on {}? Find late-night, early morning, and {} service providers in {}.",
                        service_name.to_lowercase(),
                        city,
                        time_label(time).to_lowercase(),
                        day,
                        time_label(time).to_lowercase(),
                        city
                    );

                    let h1 = title.clone();

                    let content = format!(
                        r##"<p>Browse our directory of {} providers in {} open {} on {}. Find detailed business listings, reviews, and contact information for each provider serving the {} area during {} hours.</p>"##,
                        service_name.to_lowercase(),
                        city,
                        time_label(time).to_lowercase(),
                        day,
                        city,
                        time_label(time).to_lowercase()
                    );

                    // Check for existing slug
                    let existing = sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM programmatic_pages WHERE directory_id=$1 AND slug=$2"
                    )
                    .bind(dir_id)
                    .bind(&slug)
                    .fetch_one(&s.db)
                    .await?;

                    if existing > 0 {
                        skipped += 1;
                        continue;
                    }

                    let result = sqlx::query(
                        r##"INSERT INTO programmatic_pages
                        (directory_id, service_id, slug, title, meta_title, meta_description, h1, content, template_name, status, day_tags, time_tags)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'published', $10, $11)"##
                    )
                    .bind(dir_id)
                    .bind(service_id)
                    .bind(&slug)
                    .bind(&title)
                    .bind(&title)
                    .bind(&meta_description)
                    .bind(&h1)
                    .bind(&content)
                    .bind("scheduled-trap-door")
                    .bind(&vec![day.clone()])
                    .bind(&vec![time.clone()])
                    .execute(&s.db)
                    .await;

                    match result {
                        Ok(_) => created += 1,
                        Err(_) => skipped += 1,
                    }
                }
            }
        }
    }

    // Update last_generated_at on any template that uses this directory
    // (or create a reference for the auto-generated run)
    let _ = sqlx::query(
        "UPDATE trap_door_templates SET last_generated_at=NOW(), page_count=$1 WHERE directory_id=$2"
    )
    .bind(created as i32)
    .bind(dir_id)
    .execute(&s.db)
    .await;

    Ok(Json(json!({
        "created": created,
        "skipped": skipped,
        "total": total,
        "message": format!("Generated {} hyper-niche pages in directory", created)
    })))
}

fn time_label(tag: &str) -> &str {
    match tag {
        "morning" => "Morning",
        "afternoon" => "Afternoon",
        "evening" => "Evening",
        "past-9pm" => "Past 9 PM",
        _ => tag,
    }
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Simple HTML escaping helper
fn htmlesc(s: &str) -> String {
    s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&#39;")
}
