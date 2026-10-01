//! ZaarHub community directory frontend API endpoints
//! New endpoints for the community-driven directory experience.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct CityHub {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub business_count: i64,
    pub featured_image: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivityItem {
    pub id: String,
    pub activity_type: String, // "review", "deal_added", "business_claimed", "event"
    pub message: String,
    pub business_name: Option<String>,
    pub business_slug: Option<String>,
    pub directory_slug: Option<String>,
    pub directory_name: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectoryHomepageData {
    pub directory: DirectorySummary,
    pub stats: DirectoryStats,
    pub featured_businesses: Vec<BusinessCard>,
    pub recent_reviews: Vec<ReviewCard>,
    pub active_deals: Vec<DealCard>,
    pub upcoming_events: Vec<EventCard>,
    pub categories: Vec<CategoryPill>,
    /// Slugs of the dining categories the synthetic "Places to Eat" pill resolves to, so the
    /// frontend filterCity('dining') can match the full browsable dining set per city.
    pub dining_slugs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spotlights: Option<Vec<Value>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectorySummary {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub city: Option<String>,
    pub business_count: i64,
    pub image_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectoryStats {
    pub total_businesses: i64,
    pub total_reviews: i64,
    pub total_deals: i64,
    pub total_events: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BusinessCard {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub category_slug: Option<String>,
    pub rating: Option<f64>,
    pub review_count: Option<i32>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub image_url: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub is_claimed: bool,
    pub has_deal: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewCard {
    pub id: Uuid,
    pub business_name: String,
    pub business_slug: String,
    pub reviewer_name: Option<String>,
    pub rating: i32,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DealCard {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub deal_price: Option<String>,
    pub original_price: Option<String>,
    pub discount_percent: Option<i32>,
    pub image_url: Option<String>,
    pub business_name: String,
    pub business_slug: String,
    pub directory_slug: String,
    pub end_date: Option<DateTime<Utc>>,
    pub featured: Option<bool>,
    pub business_category: Option<String>,
    pub business_category_slug: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventCard {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub event_date: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub image_url: Option<String>,
    pub business_name: Option<String>,
    pub business_slug: Option<String>,
    pub directory_slug: Option<String>,
    pub rsvp_count: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CategoryPill {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub business_count: i64,
    pub icon: Option<String>,
    pub group_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DirectoryListQuery {
    pub status: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/v1/zaarhub/cities — list active cities with counts
pub async fn list_cities(State(s): State<AppState>) -> ApiResult<Json<Vec<CityHub>>> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>, i64, String)>(
        r#"SELECT d.id, d.name, d.slug, d.description,
                  COALESCE((SELECT COUNT(*) FROM businesses b WHERE b.directory_id = d.id AND b.is_active = true), 0) as business_count,
                  COALESCE(d.status, 'draft') as status
           FROM directories d
           WHERE d.status = 'active' OR d.status IS NULL
           ORDER BY business_count DESC"#
    )
    .fetch_all(&s.db)
    .await?;

    let cities: Vec<CityHub> = rows
        .into_iter()
        .map(|(id, name, slug, desc, count, status)| CityHub {
            id,
            name,
            slug,
            description: desc,
            business_count: count,
            featured_image: None,
            status,
        })
        .collect();

    Ok(Json(cities))
}

/// GET /api/v1/zaarhub/activity — recent platform-wide activity
pub async fn get_activity(State(s): State<AppState>) -> ApiResult<Json<Vec<ActivityItem>>> {
    // Recent reviews (only from directories visible on network)
    let recent_reviews: Vec<(
        Uuid,
        String,
        Option<String>,
        i32,
        Option<String>,
        Option<DateTime<Utc>>,
        String,
        String,
    )> = sqlx::query_as(
        r#"SELECT r.id, b.name, r.reviewer_name, r.rating, r.content, r.created_at,
                  b.slug, d.slug as dir_slug
           FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           JOIN directories d ON d.id = b.directory_id
           WHERE r.status = 'approved'
             AND (d.zaarhub_config->>'network_visible')::boolean = true
             AND (d.zaarhub_config->>'show_reviews')::boolean = true
           ORDER BY r.created_at DESC
           LIMIT 10"#,
    )
    .fetch_all(&s.db)
    .await?;

    let mut items: Vec<ActivityItem> = recent_reviews
        .into_iter()
        .map(
            |(id, biz_name, reviewer, rating, _comment, ts, biz_slug, dir_slug)| {
                let reviewer_name = reviewer.unwrap_or_else(|| "Someone".to_string());
                let stars = "★".repeat(rating as usize);
                ActivityItem {
                    id: format!("review-{}", id),
                    activity_type: "review".to_string(),
                    message: format!(
                        "{} left a {}-star review for {}",
                        reviewer_name, rating, biz_name
                    ),
                    business_name: Some(biz_name),
                    business_slug: Some(biz_slug),
                    directory_slug: Some(dir_slug),
                    directory_name: None,
                    timestamp: ts.unwrap_or_else(Utc::now),
                }
            },
        )
        .collect();

    // Recent deals added (from directories with show_deals enabled)
    let recent_deals: Vec<(Uuid, String, String, String, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT de.id, de.title, b.slug, d.slug as dir_slug, de.created_at
           FROM deals de
           JOIN businesses b ON b.id = de.business_id
           JOIN directories d ON d.id = de.directory_id
           WHERE de.status = 'active'
             AND (d.zaarhub_config->>'network_visible')::boolean = true
             AND (d.zaarhub_config->>'show_deals')::boolean = true
           ORDER BY de.created_at DESC
           LIMIT 5"#,
    )
    .fetch_all(&s.db)
    .await?;

    for (id, title, biz_slug, dir_slug, ts) in recent_deals {
        items.push(ActivityItem {
            id: format!("deal-{}", id),
            activity_type: "deal_added".to_string(),
            message: format!("New deal: {} 🎉", title),
            business_name: None,
            business_slug: Some(biz_slug),
            directory_slug: Some(dir_slug),
            directory_name: None,
            timestamp: ts,
        });
    }

    // Recently claimed businesses (from visible directories)
    let recent_claimed: Vec<(Uuid, String, String, String, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT cb.id, b.name, b.slug, d.slug as dir_slug, cb.created_at
           FROM claimed_businesses cb
           JOIN businesses b ON b.id = cb.business_id
           JOIN directories d ON d.id = b.directory_id
           WHERE (d.zaarhub_config->>'network_visible')::boolean = true
           ORDER BY cb.created_at DESC
           LIMIT 5"#,
    )
    .fetch_all(&s.db)
    .await?;

    for (id, biz_name, biz_slug, dir_slug, ts) in recent_claimed {
        items.push(ActivityItem {
            id: format!("claimed-{}", id),
            activity_type: "business_claimed".to_string(),
            message: format!("{} is now a verified business", biz_name),
            business_name: Some(biz_name),
            business_slug: Some(biz_slug),
            directory_slug: Some(dir_slug),
            directory_name: None,
            timestamp: ts,
        });
    }

    // Sort by recency
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    items.truncate(20);

    Ok(Json(items))
}

/// GET /api/v1/zaarhub/homepage — full homepage data for the ZaarHub network homepage
/// If a network slug is provided, scopes data to that network
pub async fn get_homepage(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    // Featured/active cities with business counts
    let cities = sqlx::query_as::<_, (Uuid, String, String, Option<String>, i64, Option<serde_json::Value>)>(
        r#"SELECT d.id, d.name, d.slug, d.description,
                  COALESCE((SELECT COUNT(*) FROM businesses b WHERE b.directory_id = d.id AND b.is_active = true), 0) as business_count,
                  d.zaarhub_config
           FROM directories d
           WHERE (d.status = 'active' OR d.status IS NULL)
             AND (d.zaarhub_config->>'network_visible')::boolean = true
           ORDER BY business_count DESC
           LIMIT 20"#
    )
    .fetch_all(&s.db)
    .await?;

    let city_list: Vec<Value> = cities
        .into_iter()
        .map(|(id, name, slug, desc, count, zh_config)| {
            let featured_url = zh_config.and_then(|c| {
                c.get("featured_image_url")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            });
            json!({
                "id": id,
                "name": name,
                "slug": slug,
                "description": desc,
                "business_count": count,
                "featured_image": featured_url,
            })
        })
        .collect();

    // Featured deals across the network (respects zaarhub_config.show_deals per directory)
    let deals = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<String>, String, String, String, Option<DateTime<Utc>>, Option<bool>, Option<String>, Option<String>)>(
        r#"SELECT de.id, de.title, de.description, de.deal_price, de.original_price, de.discount_percent,
                  de.image_url, b.name as biz_name, b.slug as biz_slug, d.slug as dir_slug,
                  de.end_date, de.zaarhub_featured,
                  dc.name AS business_category, dc.slug AS business_category_slug
           FROM deals de
           JOIN businesses b ON b.id = de.business_id
           JOIN directories d ON d.id = de.directory_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE de.status = 'active'
             AND de.zaarhub_featured = true
             AND (d.zaarhub_config->>'show_deals')::boolean = true
             AND (d.zaarhub_config->>'network_visible')::boolean = true
           ORDER BY de.created_at DESC
           LIMIT 8"#
    )
    .fetch_all(&s.db)
    .await?;

    let deal_list: Vec<DealCard> = deals
        .into_iter()
        .map(
            |(
                id,
                title,
                desc,
                deal_price,
                orig_price,
                discount,
                img,
                biz_name,
                biz_slug,
                dir_slug,
                end_date,
                featured,
                biz_cat,
                biz_cat_slug,
            )| {
                DealCard {
                    id,
                    title,
                    description: desc,
                    deal_price,
                    original_price: orig_price,
                    discount_percent: discount,
                    image_url: img,
                    business_name: biz_name,
                    business_slug: biz_slug,
                    directory_slug: dir_slug,
                    end_date,
                    featured,
                    business_category: biz_cat,
                    business_category_slug: biz_cat_slug,
                }
            },
        )
        .collect();

    // Upcoming events across the network
    let events = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            Option<i64>,
        ),
    >(
        r#"SELECT e.id, e.title, e.description, e.event_date, e.location,
                  e.image_url, b.slug as biz_slug, d.slug as dir_slug,
                  (SELECT COUNT(*) FROM event_rsvps r WHERE r.event_id = e.id) as rsvp_count
           FROM community_events e
           LEFT JOIN businesses b ON b.id = e.business_id
           JOIN directories d ON d.id = e.directory_id
           WHERE (e.event_date >= NOW() - INTERVAL '1 day')
             AND e.zaarhub_featured = true
             AND (d.zaarhub_config->>'show_events')::boolean = true
             AND (d.zaarhub_config->>'network_visible')::boolean = true
           ORDER BY e.event_date ASC
           LIMIT 6"#,
    )
    .fetch_all(&s.db)
    .await?;

    let event_list: Vec<EventCard> = events
        .into_iter()
        .map(
            |(id, title, desc, event_date, location, img, biz_slug, dir_slug, rsvp_count)| {
                EventCard {
                    id,
                    title,
                    description: desc,
                    event_date,
                    location,
                    image_url: img,
                    business_name: None,
                    business_slug: biz_slug,
                    directory_slug: Some(dir_slug),
                    rsvp_count,
                }
            },
        )
        .collect();

    // Network-wide stats
    let total_businesses: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM businesses WHERE is_active = true")
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);
    let total_reviews: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reviews WHERE status = 'approved'")
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);
    let total_cities: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM directories WHERE (status = 'active' OR status IS NULL) AND (zaarhub_config->>'network_visible')::boolean = true"
    )
        .fetch_one(&s.db).await.unwrap_or(0);

    // Recent activity feed (top 8)
    let activity = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            i32,
            Option<String>,
            Option<DateTime<Utc>>,
            String,
            String,
        ),
    >(
        r#"SELECT r.id, b.name, r.reviewer_name, r.rating, r.content, r.created_at,
                  b.slug, d.slug as dir_slug
           FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           JOIN directories d ON d.id = b.directory_id
           WHERE r.status = 'approved'
           ORDER BY r.created_at DESC
           LIMIT 8"#,
    )
    .fetch_all(&s.db)
    .await?;

    let activity_feed: Vec<Value> = activity.into_iter().map(|(id, biz_name, reviewer, rating, _comment, ts, biz_slug, dir_slug)| {
        json!({
            "id": format!("review-{}", id),
            "type": "review",
            "message": format!("{} rated {} {}★", reviewer.unwrap_or_else(|| "Someone".to_string()), biz_name, rating),
            "business_slug": biz_slug,
            "directory_slug": dir_slug,
            "timestamp": ts,
        })
    }).collect();

    // Categories for filter pills
    let categories = sqlx::query_as::<_, (Uuid, String, String, Option<i64>, Option<String>, Option<String>)>(
        r#"SELECT c.id, c.name, c.slug,
                  (SELECT COUNT(*) FROM businesses b WHERE b.category_id = c.id AND b.is_active = true) as biz_count,
                  c.icon, c.group_name
           FROM directory_categories c
           ORDER BY
             CASE c.slug
               WHEN 'fine-dining' THEN 1 WHEN 'fitness-studio' THEN 2 WHEN 'day-spa' THEN 3
               WHEN 'dentist' THEN 4 WHEN 'real-estate-agent' THEN 5 WHEN 'hair-salon' THEN 6
               WHEN 'auto-repair' THEN 7 WHEN 'plumber' THEN 8
               ELSE 100
             END ASC,
             biz_count DESC"#
    )
    .fetch_all(&s.db)
    .await?;

    let category_pills: Vec<Value> = categories
        .into_iter()
        .map(|(id, name, slug, count, icon, group_name)| {
            json!({
                "id": id, "name": name, "slug": slug, "business_count": count.unwrap_or(0),
                "icon": icon, "group_name": group_name,
            })
        })
        .collect();

    // ??? Phase 4: Spotlight/sponsored listings across active directories
    let spotlights = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<f64>,
            Option<i32>,
            Option<String>,
            String,
            Option<String>,
        ),
    >(
        r#"SELECT sl.id, b.name, b.slug, b.description,
                  dc.name as category,
                  b.rating, b.review_count,
                  sl.badge_text, sl.slot_position::text, d.slug as dir_slug
           FROM sponsored_listings sl
           JOIN businesses b ON b.id = sl.business_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           JOIN directories d ON d.id = sl.directory_id
           WHERE sl.is_active = true
             AND sl.start_date <= CURRENT_DATE
             AND sl.end_date >= CURRENT_DATE
           ORDER BY sl.slot_position ASC
           LIMIT 12"#,
    )
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    let spotlight_list: Vec<Value> = spotlights
        .into_iter()
        .map(
            |(id, name, slug, desc, cat, rating, rv_count, badge, pos, dir_slug)| {
                json!({
                    "id": id,
                    "name": name,
                    "slug": slug,
                    "description": desc,
                    "category": cat,
                    "rating": rating,
                    "review_count": rv_count,
                    "badge_text": badge,
                    "directory_slug": dir_slug,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "cities": city_list,
        "featured_deals": deal_list,
        "upcoming_events": event_list,
        "recent_activity": activity_feed,
        "category_pills": category_pills,
        "spotlights": spotlight_list,
        "stats": {
            "total_businesses": total_businesses,
            "total_reviews": total_reviews,
            "total_cities": total_cities,
        }
    })))
}

/// GET /api/v1/zaarhub/cities/:slug — full directory/city page data
pub async fn get_city_page(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<DirectoryHomepageData>> {
    // Look up directory
    let dir = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>)>(
        "SELECT id, name, slug, description, city FROM directories WHERE slug = $1",
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("City '{}' not found", slug)))?;

    let (dir_id, dir_name, dir_slug, dir_desc, dir_city) = dir;

    // Business count
    let biz_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses WHERE directory_id = $1 AND is_active = true",
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Total reviews in this directory
    let total_reviews: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           WHERE b.directory_id = $1 AND r.status = 'approved'"#,
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Active deals
    let total_deals: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM deals de
           JOIN businesses b ON b.id = de.business_id
           WHERE b.directory_id = $1 AND de.status = 'active'"#,
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Upcoming events
    let total_events: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM community_events e
           JOIN directories d ON d.id = e.directory_id
           WHERE d.id = $1 AND (e.event_date >= NOW() OR e.event_date IS NULL)"#,
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Featured businesses (with rating + category)
    let businesses = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<f64>,
            Option<i32>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<f64>,
            Option<f64>,
            Option<Uuid>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"SELECT b.id, b.name, b.slug, b.description, b.rating, b.review_count,
                  b.phone, b.website, b.address, b.city, b.latitude, b.longitude, b.category_id,
                  dc.name as category_name, dc.slug as category_slug
           FROM businesses b
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE b.directory_id = $1 AND b.is_active = true
           ORDER BY b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST
           LIMIT 500"#,
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let featured: Vec<BusinessCard> = businesses
        .into_iter()
        .map(
            |(
                id,
                name,
                slug,
                desc,
                rating,
                review_count,
                phone,
                website,
                address,
                city,
                lat,
                lng,
                cat_id,
                cat_name,
                cat_slug,
            )| {
                BusinessCard {
                    id,
                    name,
                    slug,
                    description: desc,
                    category: cat_name,
                    category_slug: cat_slug,
                    rating,
                    review_count,
                    phone,
                    website,
                    address,
                    city,
                    image_url: None,
                    latitude: lat,
                    longitude: lng,
                    is_claimed: false,
                    has_deal: false,
                }
            },
        )
        .collect();

    // Recent reviews
    let reviews = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            i32,
            Option<String>,
            Option<DateTime<Utc>>,
        ),
    >(
        r#"SELECT r.id, b.name, b.slug, r.reviewer_name, r.rating, r.content, r.created_at
           FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           WHERE b.directory_id = $1 AND r.status = 'approved'
           ORDER BY r.created_at DESC
           LIMIT 10"#,
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let review_cards: Vec<ReviewCard> = reviews
        .into_iter()
        .map(
            |(id, biz_name, biz_slug, reviewer, rating, comment, ts)| ReviewCard {
                id,
                business_name: biz_name,
                business_slug: biz_slug,
                reviewer_name: reviewer,
                rating,
                comment,
                created_at: ts.unwrap_or_else(Utc::now),
            },
        )
        .collect();

    // Active deals in this city
    let deals = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<String>, String, String, Option<DateTime<Utc>>, Option<bool>, Option<String>, Option<String>)>(
        r#"SELECT de.id, de.title, de.description, de.deal_price, de.original_price, de.discount_percent,
                  de.image_url, b.name as biz_name, b.slug as biz_slug, de.end_date, de.featured,
                  dc.name AS business_category, dc.slug AS business_category_slug
           FROM deals de
           JOIN businesses b ON b.id = de.business_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE b.directory_id = $1 AND de.status = 'active'
           ORDER BY de.created_at DESC
           LIMIT 8"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let deal_cards: Vec<DealCard> = deals
        .into_iter()
        .map(
            |(
                id,
                title,
                desc,
                deal_price,
                orig_price,
                discount,
                img,
                biz_name,
                biz_slug,
                end_date,
                featured,
                biz_cat,
                biz_cat_slug,
            )| {
                DealCard {
                    id,
                    title,
                    description: desc,
                    deal_price,
                    original_price: orig_price,
                    discount_percent: discount,
                    image_url: img,
                    business_name: biz_name,
                    business_slug: biz_slug,
                    directory_slug: dir_slug.clone(),
                    end_date,
                    featured,
                    business_category: biz_cat,
                    business_category_slug: biz_cat_slug,
                }
            },
        )
        .collect();

    // Upcoming events
    let events = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        r#"SELECT e.id, e.title, e.description, e.event_date, e.location,
                  e.image_url, b.slug as biz_slug,
                  (SELECT COUNT(*) FROM event_rsvps r WHERE r.event_id = e.id) as rsvp_count
           FROM community_events e
           LEFT JOIN businesses b ON b.id = e.business_id
           WHERE e.directory_id = $1 AND (e.event_date >= NOW() - INTERVAL '1 day')
           ORDER BY e.event_date ASC
           LIMIT 6"#,
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let event_cards: Vec<EventCard> = events
        .into_iter()
        .map(
            |(id, title, desc, event_date, location, img, biz_slug, rsvp_count)| EventCard {
                id,
                title,
                description: desc,
                event_date,
                location,
                image_url: img,
                business_name: None,
                business_slug: biz_slug,
                directory_slug: Some(dir_slug.clone()),
                rsvp_count,
            },
        )
        .collect();

    // Categories for filtering
    // Always include the pinned featured 7 (non-dining) even if a city has 0 businesses in
    // them, then append any remaining categories that have active businesses in this city.
    //
    // Dining note: the 'fine-dining' parent (id 8ad7286b) holds cuisine children
    // (american, italian, chinese, indian, japanese, french, mediterranean, mexican,
    // steakhouse) and the 'Food & Drink' parent (id 3f44c007) holds pizza, seafood,
    // coffee-shop, catering, breakfast, food-trucks, bars-breweries. The real, browsable
    // restaurant businesses are stored on these LEAF children (pizza=121, coffee-shop=96,
    // catering=63, seafood=29 today; the Fine Dining cuisines are unpopulated). We pull all
    // children of both families so the synthetic "Places to Eat" pill (built below) always
    // resolves to the per-city dining set on every city — count-independent, all cities.
    let categories = sqlx::query_as::<_, (Uuid, String, String, Option<i64>, Option<String>, Option<String>)>(
        r#"SELECT c.id, c.name, c.slug,
                  (SELECT COUNT(*) FROM businesses b WHERE b.category_id = c.id AND b.directory_id = $1 AND b.is_active = true) as biz_count,
                  c.icon, c.group_name
           FROM directory_categories c
           WHERE c.slug IN ('fitness-studio','day-spa','dentist','real-estate-agent','hair-salon','auto-repair','plumber')
              OR c.parent_id = '8ad7286b-8be1-4224-b1d6-04dec038ac81'   -- Fine Dining children
              OR c.parent_id = '3f44c007-771d-42a9-940d-227f46171cbf'   -- Food & Drink children
              OR EXISTS (SELECT 1 FROM businesses b WHERE b.category_id = c.id AND b.directory_id = $1 AND b.is_active = true)
           ORDER BY
             CASE c.slug
               WHEN 'fitness-studio' THEN 0 WHEN 'day-spa' THEN 1 WHEN 'dentist' THEN 2
               WHEN 'real-estate-agent' THEN 3 WHEN 'hair-salon' THEN 4 WHEN 'auto-repair' THEN 5
               WHEN 'plumber' THEN 6
               ELSE 100
             END ASC,
             c.group_name, c.name"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let mut category_pills: Vec<CategoryPill> = categories
        .into_iter()
        .map(|(id, name, slug, count, icon, group_name)| CategoryPill {
            id,
            name,
            slug,
            business_count: count.unwrap_or(0),
            icon,
            group_name,
        })
        .collect();

    // Dining set: the categories the "Places to Eat" pill should match. We aggregate
    // every leaf dining category from BOTH the Fine Dining group (cuisines) AND the
    // Food & Drink group (pizza, coffee-shop, catering, seafood, breakfast, etc.), but
    // exclude the two aggregate parent rows themselves. This ensures the pill resolves to
    // real browsable businesses per city (the Fine Dining cuisines were empty in practice;
    // the real dining businesses live in the Food & Drink leaf categories). Slug list is
    // exposed to the frontend so filterCity resolves the parent pill to the full dining set.
    let dining_slugs: Vec<String> = category_pills
        .iter()
        .filter(|c| {
            let g = c.group_name.as_deref().unwrap_or("");
            (g == "Fine Dining" || g == "Food & Drink")
                && c.slug != "fine-dining"
                && c.slug != "food-drink"
        })
        .map(|c| c.slug.clone())
        .collect();

    // Aggregate business count across all dining leaf categories for this city.
    let dining_count: i64 = if dining_slugs.is_empty() {
        0
    } else {
        let cnt: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM businesses b
               JOIN directory_categories c ON c.id = b.category_id
               WHERE b.directory_id = $1 AND b.is_active = true
                 AND c.slug = ANY($2)"#,
        )
        .bind(dir_id)
        .bind(&dining_slugs)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);
        cnt
    };

    // Remove raw dining parent rows (fine-dining / food-drink) that the pills query may have
    // returned so they don't render as dead pills; the synthetic "Places to Eat" pill below
    // replaces them.
    category_pills
        .retain(|c| c.slug != "fine-dining" && c.slug != "food-drink" && c.slug != "dining");

    // Insert the synthetic "Places to Eat" pill as the #1 pinned pill. It aggregates every
    // dining leaf category in this city (Fine Dining cuisines + Food & Drink children) so
    // users can find places to eat (every type) in one click on any city page.
    category_pills.insert(
        0,
        CategoryPill {
            id: Uuid::nil(),
            name: "Places to Eat".to_string(),
            slug: "dining".to_string(),
            business_count: dining_count,
            icon: Some("🍽️".to_string()),
            group_name: Some("Dining".to_string()),
        },
    );

    // ??? Phase 4: Spotlights for this directory
    let spotlights = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<f64>,
            Option<i32>,
            Option<String>,
            i32,
            Option<bool>,
        ),
    >(
        r#"SELECT sl.id, b.name, b.slug, b.description,
                  dc.name as category,
                  b.rating, b.review_count,
                  sl.badge_text, sl.slot_position, sl.featured
           FROM sponsored_listings sl
           JOIN businesses b ON b.id = sl.business_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE sl.directory_id = $1
             AND sl.is_active = true
             AND sl.start_date <= CURRENT_DATE
             AND sl.end_date >= CURRENT_DATE
           ORDER BY sl.slot_position ASC, sl.featured DESC"#,
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    let spotlight_list: Vec<Value> = spotlights
        .into_iter()
        .map(
            |(id, name, slug, desc, cat, rating, rv_count, badge, pos, featured)| {
                json!({
                    "id": id,
                    "name": name,
                    "slug": slug,
                    "description": desc,
                    "category": cat,
                    "rating": rating,
                    "review_count": rv_count,
                    "badge_text": badge,
                    "slot_position": pos,
                    "featured": featured.unwrap_or(false),
                })
            },
        )
        .collect();

    Ok(Json(DirectoryHomepageData {
        directory: DirectorySummary {
            id: dir_id,
            name: dir_name,
            slug: dir_slug,
            description: dir_desc,
            city: dir_city,
            business_count: biz_count,
            image_url: None,
        },
        stats: DirectoryStats {
            total_businesses: biz_count,
            total_reviews,
            total_deals,
            total_events,
        },
        featured_businesses: featured,
        recent_reviews: review_cards,
        active_deals: deal_cards,
        upcoming_events: event_cards,
        categories: category_pills,
        dining_slugs,
        spotlights: Some(spotlight_list),
    }))
}

/// GET /api/v1/zaarhub/search — search businesses across the network
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub city: Option<String>,
    pub category: Option<String>,
    pub page: Option<i32>,
    pub limit: Option<i32>,
    /// Latitude for "near me" proximity search
    pub lat: Option<f64>,
    /// Longitude for "near me" proximity search
    pub lng: Option<f64>,
    /// Radius in meters for proximity search (default 5000 when lat/lng provided)
    pub radius: Option<f64>,
}

pub async fn search_businesses(
    State(s): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let search_term = query.q.unwrap_or_default();
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    // Rebuild SQL with proper parameterized queries instead of string formatting
    let search_pattern = if search_term.is_empty() {
        String::new()
    } else {
        format!("%{}%", search_term.replace('%', "").replace('_', ""))
    };

    // Use a parameterized approach: build query with numbered placeholders
    // Since the ILIKE/field selection varies by what's provided, use a raw query
    // with sqlx::query_as bound parameters rather than format! injection.
    // We use a CTE pattern: always include the search term for parameter consistency.

    // Capture proximity params before the block so they're in scope for the results builder
    let proximity = if let (Some(lat), Some(lng)) = (query.lat, query.lng) {
        let radius = query.radius.unwrap_or(5000.0); // default 5km
        Some((lat, lng, radius))
    } else {
        None
    };

    let rows: Vec<(
        Uuid,
        String,
        String,
        Option<String>,
        Option<f64>,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        String,
        String,
    )> = {
        let (city_param, category_param) = (query.city.clone(), query.category.clone());

        // Build base query
        let mut sql = String::from(
            r#"SELECT b.id, b.name, b.slug, b.description, b.rating, b.review_count, 
                      b.phone, b.website, b.address, b.city, b.latitude, b.longitude,
                      d.name as dir_name, d.slug as dir_slug
               FROM businesses b
               JOIN directories d ON d.id = b.directory_id
               WHERE b.is_active = true"#,
        );

        let mut param_count: i32 = 0;

        if !search_term.is_empty() && !search_pattern.is_empty() {
            param_count += 1;
            sql.push_str(&format!(
                " AND (b.name ILIKE ${0} OR b.description ILIKE ${0} OR b.city ILIKE ${0} OR b.category_id IN (
                    SELECT id FROM directory_categories WHERE name ILIKE ${0}
                ))",
                param_count
            ));
        }

        if let Some(ref _city) = city_param {
            param_count += 1;
            sql.push_str(&format!(" AND d.slug = ${}", param_count));
        }

        if let Some(ref _category) = category_param {
            param_count += 1;
            sql.push_str(&format!(
                " AND b.category_id IN (SELECT id FROM directory_categories WHERE slug = ${})",
                param_count
            ));
        }

        // Add proximity clause if lat/lng provided — inlined as numeric literals (safe for f64)
        if let Some((lat, lng, radius)) = proximity {
            sql.push_str(&format!(
                " AND b.latitude IS NOT NULL AND b.longitude IS NOT NULL
                  AND (6371000 * acos(cos(radians({lat})) * cos(radians(b.latitude)) * cos(radians(b.longitude) - radians({lng})) + sin(radians({lat})) * sin(radians(b.latitude)))) < {radius}",
                lat = lat, lng = lng, radius = radius
            ));
        }

        sql.push_str(" ORDER BY ");
        if let Some((lat, lng, _radius)) = proximity {
            // Sort by distance ascending when proximity is active
            sql.push_str(&format!(
                "(6371000 * acos(cos(radians({lat})) * cos(radians(b.latitude)) * cos(radians(b.longitude) - radians({lng})) + sin(radians({lat})) * sin(radians(b.latitude)))) ASC,",
                lat = lat, lng = lng
            ));
        }
        sql.push_str(" b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST");
        sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

        // Build query with proper binds (only string params use binds — lat/lng inlined as numeric literals)
        let mut q = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                Option<String>,
                Option<f64>,
                Option<i32>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<f64>,
                Option<f64>,
                String,
                String,
            ),
        >(&sql);

        param_count = 0;
        if !search_term.is_empty() && !search_pattern.is_empty() {
            param_count += 1;
            q = q.bind(&search_pattern);
        }
        if let Some(ref city) = city_param {
            param_count += 1;
            q = q.bind(city);
        }
        if let Some(ref category) = category_param {
            param_count += 1;
            q = q.bind(category);
        }

        q.fetch_all(&s.db).await?
    };

    let results: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                name,
                slug,
                desc,
                rating,
                review_count,
                phone,
                website,
                address,
                city,
                lat,
                lng,
                dir_name,
                dir_slug,
            )| {
                // Calculate distance from search center if proximity is active
                let distance: Option<f64> = if let Some((slat, slng, _)) = proximity {
                    if let (Some(blat), Some(blng)) = (lat, lng) {
                        // Haversine in JS-compatible form; compute server-side as well
                        let dlat = (blat - slat).to_radians();
                        let dlng = (blng - slng).to_radians();
                        let a = (dlat / 2.0).sin().powi(2)
                            + slat.to_radians().cos()
                                * blat.to_radians().cos()
                                * (dlng / 2.0).sin().powi(2);
                        let c = 2.0 * a.sqrt().asin();
                        Some((6371000.0 * c).round() / 1000.0) // distance in km, rounded to 3 decimals
                    } else {
                        None
                    }
                } else {
                    None
                };

                json!({
                    "id": id, "name": name, "slug": slug,
                    "description": desc, "rating": rating,
                    "review_count": review_count, "phone": phone,
                    "website": website, "address": address,
                    "city": city, "latitude": lat, "longitude": lng,
                    "directory_name": dir_name, "directory_slug": dir_slug,
                    "distance_km": distance,
                })
            },
        )
        .collect();

    let total: i64 = if search_term.is_empty() && query.city.is_none() && query.category.is_none() {
        sqlx::query_scalar("SELECT COUNT(*) FROM businesses WHERE is_active = true")
            .fetch_one(&s.db)
            .await
            .unwrap_or(0)
    } else {
        0 // rough count not critical for MVP
    };

    Ok(Json(json!({
        "results": results,
        "total": total,
        "page": page,
        "limit": limit,
    })))
}

/// One business detail, from EITHER source table, normalised to ONE shape.
///
/// The detail page and the SPA both read these fields FLAT off the response root, so the
/// normalisation happens here instead of leaking a nested object into the contract.
struct BizDetail {
    id: Uuid,
    name: String,
    slug: String,
    description: Option<String>,
    phone: Option<String>,
    website: Option<String>,
    address: Option<String>,
    city: Option<String>,
    state: Option<String>,
    zip: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    rating: Option<f64>,
    review_count: i32,
    category_id: Option<Uuid>,
    /// Only the legacy `business_listings` source carries a text category.
    category_name: Option<String>,
    images: Vec<String>,
    logo_url: Option<String>,
    cover_url: Option<String>,
    claimed: bool,
    verified: bool,
    /// TRUE when the row came from `business_listings` (a different UUID space).
    is_listing: bool,
}

/// Where uploaded business images actually live (the container mounts this path at the
/// SAME path — see the docker mount for /opt/swift/www/zaarhub.com/uploads).
const UPLOADS_ROOT: &str = "/opt/swift/www/zaarhub.com/uploads";

/// TRUE only for a photo reference the browser can really load.
///
/// The `images` column holds THREE different things and only some of them are URLs:
/// absolute `http(s)://` uploads, root-relative `/uploads/...` paths, and Google Places photo
/// RESOURCE NAMES (`places/ChIJ.../photos/AWC...`) which are not fetchable URLs at all — an
/// `<img src>` on one is a guaranteed broken image. A dead photo is worse than no photo, so a
/// root-relative path counts only when the file is on disk.
fn photo_is_renderable(url: &str) -> bool {
    let u = url.trim();
    if u.starts_with("http://") || u.starts_with("https://") {
        return true;
    }
    match u.strip_prefix("/uploads/") {
        Some(rel) if !rel.is_empty() && !rel.contains("..") => {
            std::path::Path::new(UPLOADS_ROOT).join(rel).is_file()
        }
        _ => false,
    }
}

/// Accepts both shapes a photos column takes in this database: `["url", ...]` and
/// `[{"url": ...}, ...]`, and keeps only references that will actually render (see
/// `photo_is_renderable`). Nothing is ever invented or substituted.
fn image_urls(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr {
            let url = match item {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o
                    .get("url")
                    .or_else(|| o.get("photo_reference"))
                    .and_then(|u| u.as_str())
                    .map(str::to_string),
                _ => None,
            };
            if let Some(u) = url {
                if photo_is_renderable(&u) && !out.contains(&u) {
                    out.push(u);
                }
            }
        }
    }
    out
}

/// GET /api/v1/zaarhub/business/:slug/:id — business detail page
pub async fn get_business_detail(
    State(s): State<AppState>,
    Path((slug, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    // Directory slug resolution. When the caller has no directory slug (the public
    // business-detail page passes the placeholder "z"), resolve the directory from the
    // business id itself instead of 404ing.
    let dir_id: Uuid = match sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM directories WHERE slug = $1",
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    {
        Some(id) => id,
        None => {
            let bid = Uuid::parse_str(&id)
                .map_err(|_| AppError::NotFound(format!("Directory '{}' not found", slug)))?;
            sqlx::query_scalar::<_, Uuid>(
                "SELECT d.id FROM directories d JOIN businesses b ON b.directory_id = d.id WHERE b.id = $1 LIMIT 1",
            )
            .bind(bid)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Directory '{}' not found", slug)))?
        }
    };

    // Try UUID lookup first, then slug. Columns are read BY NAME (never as a positional
    // tuple): a positional tuple shifts every field onto the wrong column when one is added,
    // which is exactly how a `name` ends up rendering as somebody's phone number.
    let brow = if let Ok(bid) = Uuid::parse_str(&id) {
        sqlx::query(
            r#"SELECT b.id, b.name, b.slug, b.description, b.phone, b.website, b.address,
                      b.city, b.state, b.zip,
                      COALESCE(b.latitude, b.lat)  AS latitude,
                      COALESCE(b.longitude, b.lng) AS longitude,
                      b.rating, b.review_count, b.category_id,
                      b.images, b.logo_url, b.cover_url, b.claimed, b.verified
               FROM businesses b
               WHERE b.id = $1 AND b.directory_id = $2 AND b.is_active = true"#,
        )
        .bind(bid)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query(
            r#"SELECT b.id, b.name, b.slug, b.description, b.phone, b.website, b.address,
                      b.city, b.state, b.zip,
                      COALESCE(b.latitude, b.lat)  AS latitude,
                      COALESCE(b.longitude, b.lng) AS longitude,
                      b.rating, b.review_count, b.category_id,
                      b.images, b.logo_url, b.cover_url, b.claimed, b.verified
               FROM businesses b
               WHERE b.slug = $1 AND b.directory_id = $2 AND b.is_active = true"#,
        )
        .bind(&id)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    };

    // The business cards on city pages are built from the `business_listings` table, whose IDs
    // live in a different UUID space than `businesses`. When a card is clicked, the SPA routes
    // here with a `business_listings.id`, so fall back to that table before 404.
    let biz: BizDetail = if let Some(r) = brow {
        BizDetail {
            id: r.try_get("id")?,
            name: r.try_get("name")?,
            slug: r.try_get("slug")?,
            description: r.try_get("description")?,
            phone: r.try_get("phone")?,
            website: r.try_get("website")?,
            address: r.try_get("address")?,
            city: r.try_get("city")?,
            state: r.try_get("state")?,
            zip: r.try_get("zip")?,
            latitude: r.try_get("latitude")?,
            longitude: r.try_get("longitude")?,
            rating: r.try_get("rating")?,
            review_count: r.try_get::<Option<i32>, _>("review_count")?.unwrap_or(0),
            category_id: r.try_get("category_id")?,
            category_name: None,
            images: image_urls(
                &r.try_get::<Option<Value>, _>("images")?
                    .unwrap_or(Value::Null),
            ),
            logo_url: r.try_get("logo_url")?,
            cover_url: r.try_get("cover_url")?,
            claimed: r.try_get::<Option<bool>, _>("claimed")?.unwrap_or(false),
            verified: r.try_get::<Option<bool>, _>("verified")?.unwrap_or(false),
            is_listing: false,
        }
    } else if let Ok(bid) = Uuid::parse_str(&id) {
        let r = sqlx::query(
            r#"SELECT bl.id, bl.business_name, bl.category, bl.description, bl.phone,
                      bl.website, bl.address, cp.city_name, cp.state, bl.rating,
                      bl.review_count, bl.coordinates_lat, bl.coordinates_lng,
                      bl.logo_url, bl.cover_image_url, bl.is_claimed
               FROM business_listings bl
               JOIN city_pages cp ON bl.city_page_id = cp.id
               WHERE bl.id = $1 AND cp.city_slug = $2"#,
        )
        .bind(bid)
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;

        let listing_logo: Option<String> = r.try_get("logo_url")?;
        let listing_cover: Option<String> = r.try_get("cover_image_url")?;
        let mut listing_images: Vec<String> = Vec::new();
        for u in [listing_logo.clone(), listing_cover.clone()]
            .into_iter()
            .flatten()
        {
            if photo_is_renderable(&u) && !listing_images.contains(&u) {
                listing_images.push(u);
            }
        }

        BizDetail {
            id: r.try_get("id")?,
            name: r.try_get("business_name")?,
            slug: slug.clone(),
            description: r.try_get("description")?,
            phone: r.try_get("phone")?,
            website: r.try_get("website")?,
            address: r.try_get("address")?,
            city: r.try_get("city_name")?,
            state: r.try_get("state")?,
            zip: None,
            latitude: r.try_get("coordinates_lat")?,
            longitude: r.try_get("coordinates_lng")?,
            rating: r.try_get("rating")?,
            review_count: r.try_get::<Option<i32>, _>("review_count")?.unwrap_or(0),
            category_id: None,
            category_name: r.try_get("category")?,
            images: listing_images,
            logo_url: listing_logo,
            cover_url: listing_cover,
            claimed: r.try_get::<Option<bool>, _>("is_claimed")?.unwrap_or(false),
            verified: false,
            is_listing: true,
        }
    } else {
        return Err(AppError::NotFound("Business not found".to_string()));
    };

    // Category name: `businesses` stores a category_id, a legacy listing stores the text.
    let category_name: Option<String> = match biz.category_id {
        Some(cat_id) => sqlx::query_scalar("SELECT name FROM directory_categories WHERE id = $1")
            .bind(cat_id)
            .fetch_optional(&s.db)
            .await?
            .flatten(),
        None => biz.category_name.clone(),
    };

    // Get directory name
    let dir_name: String = sqlx::query_scalar("SELECT name FROM directories WHERE id = $1")
        .bind(dir_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or_default();

    // Get recent reviews for this business
    let reviews = sqlx::query_as::<
        _,
        (
            Uuid,
            Option<String>,
            i32,
            Option<String>,
            Option<DateTime<Utc>>,
        ),
    >(
        r#"SELECT id, reviewer_name, rating, content, created_at
           FROM reviews
           WHERE business_id = $1 AND status = 'approved'
           ORDER BY created_at DESC
           LIMIT 10"#,
    )
    .bind(biz.id)
    .fetch_all(&s.db)
    .await?;

    let review_list: Vec<Value> = reviews
        .into_iter()
        .map(|(id, reviewer, rating, content, ts)| {
            json!({
                "id": id,
                "reviewer_name": reviewer,
                "rating": rating,
                "content": content,
                "created_at": ts,
            })
        })
        .collect();

    // Get active deals for this business
    let deals = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i32>,
            Option<DateTime<Utc>>,
        ),
    >(
        r#"SELECT id, title, description, deal_price, original_price, discount_percent, end_date
           FROM deals
           WHERE business_id = $1 AND status = 'active'
           ORDER BY created_at DESC"#,
    )
    .bind(biz.id)
    .fetch_all(&s.db)
    .await?;

    let deal_list: Vec<Value> = deals
        .into_iter()
        .map(
            |(id, title, desc, deal_price, orig_price, discount, end_date)| {
                json!({
                    "id": id, "title": title, "description": desc,
                    "deal_price": deal_price, "original_price": orig_price,
                    "discount_percent": discount, "end_date": end_date,
                })
            },
        )
        .collect();

    // Claimed if the businesses row says so OR a claim record exists (the claim table is the
    // older path and both are live).
    let is_claimed: bool = biz.claimed
        || sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM claimed_businesses WHERE business_id = $1)",
        )
        .bind(biz.id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(false);

    // Hours. The previous query selected a `hours` COLUMN from business_meta, which has never
    // existed on that table (its payload lives in `meta_data`), so it errored on EVERY request
    // and `.ok()` swallowed it: a dead query that cost a round trip and served nothing.
    let hours: Option<Value> = sqlx::query_scalar::<_, Option<Value>>(
        r#"SELECT COALESCE(meta_data->'hours', meta_data->'opening_hours')
           FROM business_meta
           WHERE business_id = $1
             AND (meta_data ? 'hours' OR meta_data ? 'opening_hours')
           LIMIT 1"#,
    )
    .bind(biz.id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    // Photos: the stored `images` array, plus a cover image if one is set. Never a placeholder
    // image — an empty list is what tells the page to hide the gallery block entirely.
    let mut photos: Vec<String> = biz.images.clone();
    if let Some(cover) = biz.cover_url.clone() {
        if photo_is_renderable(&cover) && !photos.contains(&cover) {
            photos.push(cover);
        }
    }
    let image_url = photos.first().cloned();

    // Nearby businesses in the SAME city of the SAME directory. Empty means "hide the block" —
    // the page never shows an empty shell.
    let nearby: Vec<Value> = if let Some(city) = biz.city.as_deref() {
        let rows = sqlx::query(
            r#"SELECT b.id, b.name, b.slug, b.rating, b.review_count, c.name AS category
               FROM businesses b
               LEFT JOIN directory_categories c ON c.id = b.category_id
               WHERE b.directory_id = $1
                 AND b.is_active = true
                 AND b.id <> $2
                 AND b.city = $3
               ORDER BY b.rating DESC NULLS LAST, b.review_count DESC, b.name ASC
               LIMIT 6"#,
        )
        .bind(dir_id)
        .bind(biz.id)
        .bind(city)
        .fetch_all(&s.db)
        .await?;

        rows.iter()
            .map(|r| {
                json!({
                    "id": r.try_get::<Uuid, _>("id").ok(),
                    "name": r.try_get::<String, _>("name").ok(),
                    "slug": r.try_get::<String, _>("slug").ok(),
                    "rating": r.try_get::<Option<f64>, _>("rating").ok().flatten(),
                    "review_count": r.try_get::<Option<i32>, _>("review_count").ok().flatten(),
                    "category": r.try_get::<Option<String>, _>("category").ok().flatten(),
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    // ONE FLAT shape. This is the contract every consumer reads (`bizData.name`, `b.name`), so
    // the business fields sit at the TOP LEVEL next to deals / reviews / hours — there is no
    // nested `business` object any more. `lat`/`lng` are aliases of latitude/longitude because
    // the database itself carries both column pairs and callers used one each.
    Ok(Json(json!({
        "id": biz.id,
        "name": biz.name,
        "slug": biz.slug,
        "description": biz.description,
        "phone": biz.phone,
        "website": biz.website,
        "address": biz.address,
        "city": biz.city,
        "state": biz.state,
        "zip": biz.zip,
        "latitude": biz.latitude,
        "longitude": biz.longitude,
        "lat": biz.latitude,
        "lng": biz.longitude,
        "rating": biz.rating,
        "review_count": biz.review_count,
        "category_name": category_name.clone(),
        "category": category_name,
        "is_claimed": is_claimed,
        "is_verified": biz.verified,
        "source": if biz.is_listing { "listing" } else { "business" },
        "images": photos.clone(),
        "photos": photos,
        "image_url": image_url,
        "logo_url": biz.logo_url,
        "cover_url": biz.cover_url,
        "directory_name": dir_name,
        "directory_slug": slug,
        "reviews": review_list,
        "deals": deal_list,
        "hours": hours,
        "nearby": nearby,
    })))
}

// ── Standalone ZaarHub API routes ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CityFilterQuery {
    pub city: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ZaarhubCategory {
    pub category_name: String,
    pub directory_count: i64,
    pub icon: Option<String>,
}

/// GET /api/v1/zaarhub/categories — list aggregated categories across all zaarhub-enabled directories
pub async fn list_categories(
    State(s): State<AppState>,
    Query(query): Query<CityFilterQuery>,
) -> ApiResult<Json<Vec<ZaarhubCategory>>> {
    let mut sql = String::from(
        r#"WITH dir_categories AS (
            SELECT DISTINCT c.name AS category_name, c.icon, d.id AS dir_id
            FROM directory_categories c
            JOIN businesses b ON b.category_id = c.id AND b.is_active = true
            JOIN directories d ON d.id = b.directory_id
            WHERE d.zaarhub_config IS NOT NULL
              AND (d.zaarhub_config->>'network_visible')::boolean = true"#,
    );

    if query.city.is_some() {
        sql.push_str(" AND d.slug = $1");
    }

    sql.push_str(
        r#"
        )
        SELECT category_name, MAX(icon) as icon, COUNT(DISTINCT dir_id) as directory_count
        FROM dir_categories
        GROUP BY category_name
        ORDER BY
          CASE category_name
            WHEN 'Fine Dining' THEN 1 WHEN 'Fitness Studio' THEN 2 WHEN 'Day Spa' THEN 3
            WHEN 'Dentist' THEN 4 WHEN 'Real Estate Agent' THEN 5 WHEN 'Hair Salon' THEN 6
            WHEN 'Auto Repair' THEN 7 WHEN 'Plumber' THEN 8
            ELSE 100
          END ASC,
          directory_count DESC, category_name ASC"#,
    );

    let rows: Vec<(String, Option<String>, i64)> = if let Some(ref city) = query.city {
        sqlx::query_as(&sql).bind(city).fetch_all(&s.db).await?
    } else {
        sqlx::query_as(&sql).fetch_all(&s.db).await?
    };

    let categories: Vec<ZaarhubCategory> = rows
        .into_iter()
        .map(|(name, icon, count)| ZaarhubCategory {
            category_name: name,
            directory_count: count,
            icon,
        })
        .collect();

    Ok(Json(categories))
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub city: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

/// GET /api/v1/zaarhub/deals — featured deals across zaarhub directories
pub async fn list_featured_deals(
    State(s): State<AppState>,
    Query(query): Query<PaginationQuery>,
) -> ApiResult<Json<Value>> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let mut base_sql = String::from(
        r#"FROM deals de
           JOIN businesses b ON b.id = de.business_id
           JOIN directories d ON d.id = de.directory_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE de.status = 'active'
             AND de.zaarhub_featured = true
             AND (d.zaarhub_config->>'show_deals')::boolean = true
             AND (d.zaarhub_config->>'network_visible')::boolean = true"#,
    );

    let mut param_idx = 0;
    if query.city.is_some() {
        param_idx += 1;
        base_sql.push_str(&format!(" AND d.slug = ${}", param_idx));
    }

    // Count total
    let count_sql = format!("SELECT COUNT(*) {}", base_sql);
    let total: i64 = if let Some(ref city) = query.city {
        sqlx::query_scalar(&count_sql)
            .bind(city)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0)
    } else {
        sqlx::query_scalar(&count_sql)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0)
    };

    // Fetch page
    let data_sql = format!(
        r#"SELECT de.id, de.title, de.description, de.deal_price, de.original_price,
                  de.discount_percent, de.image_url, b.name as biz_name, b.slug as biz_slug,
                  d.slug as dir_slug, d.city as dir_city, de.end_date, de.zaarhub_featured,
                  dc.name as business_category, dc.slug as business_category_slug
           {}
           ORDER BY de.created_at DESC
           LIMIT {} OFFSET {}"#,
        base_sql, limit, offset
    );

    let deals: Vec<(
        Uuid,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<String>,
        String,
        String,
        String,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<bool>,
        Option<String>,
        Option<String>,
    )> = if let Some(ref city) = query.city {
        sqlx::query_as(&data_sql)
            .bind(city)
            .fetch_all(&s.db)
            .await?
    } else {
        sqlx::query_as(&data_sql).fetch_all(&s.db).await?
    };

    let deal_list: Vec<Value> = deals
        .into_iter()
        .map(
            |(
                id,
                title,
                desc,
                deal_price,
                orig_price,
                discount,
                img,
                biz_name,
                biz_slug,
                dir_slug,
                dir_city,
                end_date,
                featured,
                biz_cat,
                biz_cat_slug,
            )| {
                json!({
                    "id": id,
                    "title": title,
                    "description": desc,
                    "deal_price": deal_price,
                    "original_price": orig_price,
                    "discount_percent": discount,
                    "image_url": img,
                    "business_name": biz_name,
                    "business_slug": biz_slug,
                    "directory_slug": dir_slug,
                    "directory_city": dir_city,
                    "end_date": end_date,
                    "featured": featured,
                    "business_category": biz_cat,
                    "business_category_slug": biz_cat_slug,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "deals": deal_list,
        "total": total,
        "page": page,
        "limit": limit,
    })))
}

/// GET /api/v1/zaarhub/events — featured events across zaarhub directories
pub async fn list_featured_events(
    State(s): State<AppState>,
    Query(query): Query<PaginationQuery>,
) -> ApiResult<Json<Value>> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(20).min(100);
    let offset = (page - 1) * limit;

    let mut base_sql = String::from(
        r#"FROM community_events e
           LEFT JOIN businesses b ON b.id = e.business_id
           JOIN directories d ON d.id = e.directory_id
           WHERE e.status = 'active'
             AND e.zaarhub_featured = true
             AND (d.zaarhub_config->>'show_events')::boolean = true
             AND (d.zaarhub_config->>'network_visible')::boolean = true"#,
    );

    let mut param_idx = 0;
    if query.city.is_some() {
        param_idx += 1;
        base_sql.push_str(&format!(" AND d.slug = ${}", param_idx));
    }

    // Count total
    let count_sql = format!("SELECT COUNT(*) {}", base_sql);
    let total: i64 = if let Some(ref city) = query.city {
        sqlx::query_scalar(&count_sql)
            .bind(city)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0)
    } else {
        sqlx::query_scalar(&count_sql)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0)
    };

    // Fetch page
    let data_sql = format!(
        r#"SELECT e.id, e.title, e.description, e.event_date, e.location,
                  e.image_url, b.name as biz_name, b.slug as biz_slug,
                  d.slug as dir_slug, d.city as dir_city,
                  (SELECT COUNT(*) FROM event_rsvps r WHERE r.event_id = e.id) as rsvp_count
           {}
           ORDER BY e.event_date ASC
           LIMIT {} OFFSET {}"#,
        base_sql, limit, offset
    );

    let events: Vec<(
        Uuid,
        String,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<i64>,
    )> = if let Some(ref city) = query.city {
        sqlx::query_as(&data_sql)
            .bind(city)
            .fetch_all(&s.db)
            .await?
    } else {
        sqlx::query_as(&data_sql).fetch_all(&s.db).await?
    };

    let event_list: Vec<Value> = events
        .into_iter()
        .map(
            |(
                id,
                title,
                desc,
                event_date,
                location,
                img,
                biz_name,
                biz_slug,
                dir_slug,
                dir_city,
                rsvp_count,
            )| {
                json!({
                    "id": id,
                    "title": title,
                    "description": desc,
                    "event_date": event_date,
                    "location": location,
                    "image_url": img,
                    "business_name": biz_name,
                    "business_slug": biz_slug,
                    "directory_slug": dir_slug,
                    "directory_city": dir_city,
                    "rsvp_count": rsvp_count,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "events": event_list,
        "total": total,
        "page": page,
        "limit": limit,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ToggleFeaturedRequest {
    /// "deal" or "event"
    #[serde(rename = "type")]
    pub item_type: String,
    pub featured: bool,
}

/// POST /api/v1/spotlight/:id/feature — toggle zaarhub_featured on a deal or event
pub async fn toggle_spotlight_featured(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ToggleFeaturedRequest>,
) -> ApiResult<Json<Value>> {
    match body.item_type.as_str() {
        "deal" => {
            let existing = sqlx::query_as::<_, (Uuid, String, Option<bool>)>(
                "SELECT id, title, zaarhub_featured FROM deals WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Deal '{}' not found", id)))?;

            sqlx::query("UPDATE deals SET zaarhub_featured = $1, updated_at = NOW() WHERE id = $2")
                .bind(body.featured)
                .bind(existing.0)
                .execute(&s.db)
                .await?;

            Ok(Json(json!({
                "id": existing.0,
                "title": existing.1,
                "type": "deal",
                "featured": body.featured,
            })))
        }
        "event" => {
            let existing = sqlx::query_as::<_, (Uuid, String, Option<bool>)>(
                "SELECT id, title, zaarhub_featured FROM community_events WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Event '{}' not found", id)))?;

            sqlx::query(
                "UPDATE community_events SET zaarhub_featured = $1, updated_at = NOW() WHERE id = $2"
            )
            .bind(body.featured)
            .bind(existing.0)
            .execute(&s.db)
            .await?;

            Ok(Json(json!({
                "id": existing.0,
                "title": existing.1,
                "type": "event",
                "featured": body.featured,
            })))
        }
        other => Err(AppError::BadRequest(format!(
            "Invalid type '{}'. Must be 'deal' or 'event'",
            other
        ))),
    }
}

// ── Public city blog listing ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CityBlogQuery {
    /// Page size; defaulted and clamped server-side so the feed cannot be over-pulled.
    pub limit: Option<i64>,
}

/// GET /api/v1/zaarhub/cities/:slug/blog-posts — PUBLIC blog listing for a city.
///
/// Backs the city blog list view. Resolves the city by slug exactly like the other
/// public zaarhub city routes and 404s for an unknown slug rather than answering an
/// empty 200. Returns ONLY published posts — both the `published` flag and the
/// `status` enum must say published, and a future `scheduled_at` is excluded so a
/// not-yet-live post can never leak early. Drafts, pending-review and archived rows
/// are never selected.
pub async fn list_city_blog_posts(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<CityBlogQuery>,
) -> ApiResult<Json<Value>> {
    let dir = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directories WHERE slug = $1",
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("City '{}' not found", slug)))?;

    let (dir_id, dir_name, dir_slug) = dir;
    let limit = query.limit.unwrap_or(20).clamp(1, 50);

    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Option<String>,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"SELECT id, slug, title, excerpt, scheduled_at, created_at,
                  featured_image_url, author_name, blog_category
           FROM blog_posts
           WHERE directory_id = $1
             AND published = true
             AND status = 'published'
             AND (scheduled_at IS NULL OR scheduled_at <= NOW())
           ORDER BY COALESCE(scheduled_at, created_at) DESC
           LIMIT $2"#,
    )
    .bind(dir_id)
    .bind(limit)
    .fetch_all(&s.db)
    .await?;

    let posts: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, post_slug, title, excerpt, scheduled_at, created_at, image, author, category)| {
                let date = scheduled_at.or(created_at);
                let post_slug = post_slug.unwrap_or_default();
                json!({
                    "id": id,
                    "slug": post_slug,
                    "title": title,
                    "excerpt": excerpt,
                    "date": date,
                    "url": format!("/api/v1/d/{}/blog/{}", dir_slug, post_slug),
                    "featured_image_url": image,
                    "author_name": author,
                    "category": category,
                })
            },
        )
        .collect();

    let count = posts.len();
    Ok(Json(json!({
        "city": { "slug": dir_slug, "name": dir_name },
        "count": count,
        "posts": posts,
    })))
}
