//! ZaarHub community directory frontend API endpoints
//! New endpoints for the community-driven directory experience.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
}

#[derive(Debug, Deserialize)]
pub struct DirectoryListQuery {
    pub status: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/v1/zaarhub/cities — list active cities with counts
pub async fn list_cities(
    State(s): State<AppState>,
) -> ApiResult<Json<Vec<CityHub>>> {
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

    let cities: Vec<CityHub> = rows.into_iter().map(|(id, name, slug, desc, count, status)| {
        CityHub {
            id,
            name,
            slug,
            description: desc,
            business_count: count,
            featured_image: None,
            status,
        }
    }).collect();

    Ok(Json(cities))
}

/// GET /api/v1/zaarhub/activity — recent platform-wide activity
pub async fn get_activity(
    State(s): State<AppState>,
) -> ApiResult<Json<Vec<ActivityItem>>> {
    // Recent reviews (only from directories visible on network)
    let recent_reviews: Vec<(Uuid, String, Option<String>, i32, Option<String>, Option<DateTime<Utc>>, String, String)> = sqlx::query_as(
        r#"SELECT r.id, b.name, r.reviewer_name, r.rating, r.content, r.created_at,
                  b.slug, d.slug as dir_slug
           FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           JOIN directories d ON d.id = b.directory_id
           WHERE r.status = 'approved'
             AND (d.zaarhub_config->>'network_visible')::boolean = true
             AND (d.zaarhub_config->>'show_reviews')::boolean = true
           ORDER BY r.created_at DESC
           LIMIT 10"#
    )
    .fetch_all(&s.db)
    .await?;

    let mut items: Vec<ActivityItem> = recent_reviews.into_iter().map(|(id, biz_name, reviewer, rating, _comment, ts, biz_slug, dir_slug)| {
        let reviewer_name = reviewer.unwrap_or_else(|| "Someone".to_string());
        let stars = "★".repeat(rating as usize);
        ActivityItem {
            id: format!("review-{}", id),
            activity_type: "review".to_string(),
            message: format!("{} left a {}-star review for {}", reviewer_name, rating, biz_name),
            business_name: Some(biz_name),
            business_slug: Some(biz_slug),
            directory_slug: Some(dir_slug),
            directory_name: None,
            timestamp: ts.unwrap_or_else(Utc::now),
        }
    }).collect();

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
           LIMIT 5"#
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
           LIMIT 5"#
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
pub async fn get_homepage(
    State(s): State<AppState>,
) -> ApiResult<Json<Value>> {
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

    let city_list: Vec<Value> = cities.into_iter().map(|(id, name, slug, desc, count, zh_config)| {
        let featured_url = zh_config
            .and_then(|c| c.get("featured_image_url").and_then(|v| v.as_str().map(|s| s.to_string())));
        json!({
            "id": id,
            "name": name,
            "slug": slug,
            "description": desc,
            "business_count": count,
            "featured_image": featured_url,
        })
    }).collect();

    // Featured deals across the network (respects zaarhub_config.show_deals per directory)
    let deals = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<String>, String, String, String, Option<DateTime<Utc>>, Option<bool>)>(
        r#"SELECT de.id, de.title, de.description, de.deal_price, de.original_price, de.discount_percent,
                  de.image_url, b.name as biz_name, b.slug as biz_slug, d.slug as dir_slug,
                  de.end_date, de.zaarhub_featured
           FROM deals de
           JOIN businesses b ON b.id = de.business_id
           JOIN directories d ON d.id = de.directory_id
           WHERE de.status = 'active'
             AND de.zaarhub_featured = true
             AND (d.zaarhub_config->>'show_deals')::boolean = true
             AND (d.zaarhub_config->>'network_visible')::boolean = true
           ORDER BY de.created_at DESC
           LIMIT 8"#
    )
    .fetch_all(&s.db)
    .await?;

    let deal_list: Vec<DealCard> = deals.into_iter().map(|(id, title, desc, deal_price, orig_price, discount, img, biz_name, biz_slug, dir_slug, end_date, featured)| {
        DealCard {
            id, title, description: desc,
            deal_price, original_price: orig_price,
            discount_percent: discount, image_url: img,
            business_name: biz_name, business_slug: biz_slug,
            directory_slug: dir_slug, end_date, featured,
        }
    }).collect();

    // Upcoming events across the network
    let events = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<DateTime<Utc>>, Option<String>, Option<String>, Option<String>, String, Option<i64>)>(
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
           LIMIT 6"#
    )
    .fetch_all(&s.db)
    .await?;

    let event_list: Vec<EventCard> = events.into_iter().map(|(id, title, desc, event_date, location, img, biz_slug, dir_slug, rsvp_count)| {
        EventCard {
            id, title, description: desc, event_date, location,
            image_url: img,
            business_name: None,
            business_slug: biz_slug,
            directory_slug: Some(dir_slug),
            rsvp_count,
        }
    }).collect();

    // Network-wide stats
    let total_businesses: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM businesses WHERE is_active = true")
        .fetch_one(&s.db).await.unwrap_or(0);
    let total_reviews: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reviews WHERE status = 'approved'")
        .fetch_one(&s.db).await.unwrap_or(0);
    let total_cities: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM directories WHERE (status = 'active' OR status IS NULL) AND (zaarhub_config->>'network_visible')::boolean = true"
    )
        .fetch_one(&s.db).await.unwrap_or(0);

    // Recent activity feed (top 8)
    let activity = sqlx::query_as::<_, (Uuid, String, Option<String>, i32, Option<String>, Option<DateTime<Utc>>, String, String)>(
        r#"SELECT r.id, b.name, r.reviewer_name, r.rating, r.content, r.created_at,
                  b.slug, d.slug as dir_slug
           FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           JOIN directories d ON d.id = b.directory_id
           WHERE r.status = 'approved'
           ORDER BY r.created_at DESC
           LIMIT 8"#
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
    let categories = sqlx::query_as::<_, (Uuid, String, String, Option<i64>)>(
        r#"SELECT c.id, c.name, c.slug,
                  (SELECT COUNT(*) FROM businesses b WHERE b.category_id = c.id AND b.is_active = true) as biz_count
           FROM directory_categories c
           ORDER BY biz_count DESC
           LIMIT 12"#
    )
    .fetch_all(&s.db)
    .await?;

    let category_pills: Vec<Value> = categories.into_iter().map(|(id, name, slug, count)| {
        json!({
            "id": id, "name": name, "slug": slug, "business_count": count.unwrap_or(0),
        })
    }).collect();

    // ??? Phase 4: Spotlight/sponsored listings across active directories
    let spotlights = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>, Option<f64>, Option<i32>, Option<String>, String, Option<String>)>(
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
           LIMIT 12"#
    )
    .fetch_all(&s.db)
    .await.unwrap_or_default();

    let spotlight_list: Vec<Value> = spotlights.into_iter().map(|(id, name, slug, desc, cat, rating, rv_count, badge, pos, dir_slug)| {
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
    }).collect();

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
        "SELECT id, name, slug, description, city FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("City '{}' not found", slug)))?;

    let (dir_id, dir_name, dir_slug, dir_desc, dir_city) = dir;

    // Business count
    let biz_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses WHERE directory_id = $1 AND is_active = true"
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Total reviews in this directory
    let total_reviews: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           WHERE b.directory_id = $1 AND r.status = 'approved'"#
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Active deals
    let total_deals: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM deals de
           JOIN businesses b ON b.id = de.business_id
           WHERE b.directory_id = $1 AND de.status = 'active'"#
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Upcoming events
    let total_events: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM community_events e
           JOIN directories d ON d.id = e.directory_id
           WHERE d.id = $1 AND (e.event_date >= NOW() OR e.event_date IS NULL)"#
    )
    .bind(dir_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Featured businesses (with rating)
    let businesses = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<f64>, Option<i32>, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<f64>, Option<Uuid>)>(
        r#"SELECT b.id, b.name, b.slug, b.description, b.rating, b.review_count,
                  b.phone, b.website, b.address, b.city, b.latitude, b.longitude, b.category_id
           FROM businesses b
           WHERE b.directory_id = $1 AND b.is_active = true
           ORDER BY b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST
           LIMIT 50"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let featured: Vec<BusinessCard> = businesses.into_iter().map(|(id, name, slug, desc, rating, review_count, phone, website, address, city, lat, lng, cat_id)| {
        BusinessCard {
            id, name, slug, description: desc,
            category: None, category_slug: None,
            rating, review_count,
            phone, website, address, city,
            image_url: None,
            latitude: lat, longitude: lng,
            is_claimed: false,
            has_deal: false,
        }
    }).collect();

    // Recent reviews
    let reviews = sqlx::query_as::<_, (Uuid, String, String, Option<String>, i32, Option<String>, Option<DateTime<Utc>>)>(
        r#"SELECT r.id, b.name, b.slug, r.reviewer_name, r.rating, r.content, r.created_at
           FROM reviews r
           JOIN businesses b ON b.id = r.business_id
           WHERE b.directory_id = $1 AND r.status = 'approved'
           ORDER BY r.created_at DESC
           LIMIT 10"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let review_cards: Vec<ReviewCard> = reviews.into_iter().map(|(id, biz_name, biz_slug, reviewer, rating, comment, ts)| {
        ReviewCard {
            id,
            business_name: biz_name,
            business_slug: biz_slug,
            reviewer_name: reviewer,
            rating,
            comment,
            created_at: ts.unwrap_or_else(Utc::now),
        }
    }).collect();

    // Active deals in this city
    let deals = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<String>, String, String, Option<DateTime<Utc>>, Option<bool>)>(
        r#"SELECT de.id, de.title, de.description, de.deal_price, de.original_price, de.discount_percent,
                  de.image_url, b.name as biz_name, b.slug as biz_slug, de.end_date, de.featured
           FROM deals de
           JOIN businesses b ON b.id = de.business_id
           WHERE b.directory_id = $1 AND de.status = 'active'
           ORDER BY de.created_at DESC
           LIMIT 8"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let deal_cards: Vec<DealCard> = deals.into_iter().map(|(id, title, desc, deal_price, orig_price, discount, img, biz_name, biz_slug, end_date, featured)| {
        DealCard {
            id, title, description: desc,
            deal_price, original_price: orig_price,
            discount_percent: discount, image_url: img,
            business_name: biz_name, business_slug: biz_slug,
            directory_slug: dir_slug.clone(),
            end_date, featured,
        }
    }).collect();

    // Upcoming events
    let events = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<DateTime<Utc>>, Option<String>, Option<String>, Option<String>, Option<i64>)>(
        r#"SELECT e.id, e.title, e.description, e.event_date, e.location,
                  e.image_url, b.slug as biz_slug,
                  (SELECT COUNT(*) FROM event_rsvps r WHERE r.event_id = e.id) as rsvp_count
           FROM community_events e
           LEFT JOIN businesses b ON b.id = e.business_id
           WHERE e.directory_id = $1 AND (e.event_date >= NOW() - INTERVAL '1 day')
           ORDER BY e.event_date ASC
           LIMIT 6"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let event_cards: Vec<EventCard> = events.into_iter().map(|(id, title, desc, event_date, location, img, biz_slug, rsvp_count)| {
        EventCard {
            id, title, description: desc, event_date, location,
            image_url: img,
            business_name: None,
            business_slug: biz_slug,
            directory_slug: Some(dir_slug.clone()),
            rsvp_count,
        }
    }).collect();

    // Categories for filtering
    let categories = sqlx::query_as::<_, (Uuid, String, String, Option<i64>)>(
        r#"SELECT c.id, c.name, c.slug,
                  (SELECT COUNT(*) FROM businesses b WHERE b.category_id = c.id AND b.directory_id = $1 AND b.is_active = true) as biz_count
           FROM directory_categories c
           WHERE EXISTS (SELECT 1 FROM businesses b WHERE b.category_id = c.id AND b.directory_id = $1)
           ORDER BY biz_count DESC
           LIMIT 12"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await?;

    let category_pills: Vec<CategoryPill> = categories.into_iter().map(|(id, name, slug, count)| {
        CategoryPill { id, name, slug, business_count: count.unwrap_or(0), icon: None }
    }).collect();

    // ??? Phase 4: Spotlights for this directory
    let spotlights = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>, Option<f64>, Option<i32>, Option<String>, i32, Option<bool>)>(
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
           ORDER BY sl.slot_position ASC, sl.featured DESC"#
    )
    .bind(dir_id)
    .fetch_all(&s.db)
    .await.unwrap_or_default();

    let spotlight_list: Vec<Value> = spotlights.into_iter().map(|(id, name, slug, desc, cat, rating, rv_count, badge, pos, featured)| {
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
    }).collect();

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
    
    let rows: Vec<(Uuid, String, String, Option<String>, Option<f64>, Option<i32>, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<f64>, String, String)> = {
        let (city_param, category_param) = (query.city.clone(), query.category.clone());
        
        // Build base query
        let mut sql = String::from(
            r#"SELECT b.id, b.name, b.slug, b.description, b.rating, b.review_count, 
                      b.phone, b.website, b.address, b.city, b.latitude, b.longitude,
                      d.name as dir_name, d.slug as dir_slug
               FROM businesses b
               JOIN directories d ON d.id = b.directory_id
               WHERE b.is_active = true"#
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
            sql.push_str(&format!(" AND b.category_id IN (SELECT id FROM directory_categories WHERE slug = ${})", param_count));
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
        let mut q = sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<f64>, Option<i32>, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<f64>, String, String)>(&sql);

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

    let results: Vec<Value> = rows.into_iter().map(|(id, name, slug, desc, rating, review_count, phone, website, address, city, lat, lng, dir_name, dir_slug)| {
        // Calculate distance from search center if proximity is active
        let distance: Option<f64> = if let Some((slat, slng, _)) = proximity {
            if let (Some(blat), Some(blng)) = (lat, lng) {
                // Haversine in JS-compatible form; compute server-side as well
                let dlat = (blat - slat).to_radians();
                let dlng = (blng - slng).to_radians();
                let a = (dlat / 2.0).sin().powi(2)
                    + slat.to_radians().cos() * blat.to_radians().cos() * (dlng / 2.0).sin().powi(2);
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
    }).collect();

    let total: i64 = if search_term.is_empty() && query.city.is_none() && query.category.is_none() {
        sqlx::query_scalar("SELECT COUNT(*) FROM businesses WHERE is_active = true")
            .fetch_one(&s.db).await.unwrap_or(0)
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

/// GET /api/v1/zaarhub/business/:slug/:id — business detail page
pub async fn get_business_detail(
    State(s): State<AppState>,
    Path((slug, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let dir_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM directories WHERE slug = $1"
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    // Try UUID lookup first, then slug
    let business = if let Ok(bid) = Uuid::parse_str(&id) {
        sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>, Option<f64>, Option<i32>, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<f64>, Option<Uuid>)>(
            r#"SELECT b.id, b.name, b.slug, b.description, b.phone, b.rating, b.review_count,
                      b.website, b.address, b.city, b.state, b.latitude, b.longitude, b.category_id
               FROM businesses b
               WHERE b.id = $1 AND b.directory_id = $2 AND b.is_active = true"#
        )
        .bind(bid)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, (Uuid, String, String, Option<String>, Option<String>, Option<f64>, Option<i32>, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<f64>, Option<Uuid>)>(
            r#"SELECT b.id, b.name, b.slug, b.description, b.phone, b.rating, b.review_count,
                      b.website, b.address, b.city, b.state, b.latitude, b.longitude, b.category_id
               FROM businesses b
               WHERE b.slug = $1 AND b.directory_id = $2 AND b.is_active = true"#
        )
        .bind(&id)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    };

    let (biz_id, biz_name, biz_slug, biz_desc, biz_phone, biz_rating, biz_review_count,
         biz_website, biz_address, biz_city, biz_state, biz_lat, biz_lng, biz_cat_id) = business
        .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;

    // Get category name
    let category_name: Option<String> = if let Some(cat_id) = biz_cat_id {
        sqlx::query_scalar("SELECT name FROM directory_categories WHERE id = $1")
            .bind(cat_id)
            .fetch_optional(&s.db)
            .await?
            .flatten()
    } else {
        None
    };

    // Get directory name
    let dir_name: String = sqlx::query_scalar("SELECT name FROM directories WHERE id = $1")
        .bind(dir_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or_default();

    // Get recent reviews for this business
    let reviews = sqlx::query_as::<_, (Uuid, Option<String>, i32, Option<String>, Option<DateTime<Utc>>)>(
        r#"SELECT id, reviewer_name, rating, content, created_at
           FROM reviews
           WHERE business_id = $1 AND status = 'approved'
           ORDER BY created_at DESC
           LIMIT 10"#
    )
    .bind(biz_id)
    .fetch_all(&s.db)
    .await?;

    let review_list: Vec<Value> = reviews.into_iter().map(|(id, reviewer, rating, content, ts)| {
        json!({
            "id": id,
            "reviewer_name": reviewer,
            "rating": rating,
            "content": content,
            "created_at": ts,
        })
    }).collect();

    // Get active deals for this business
    let deals = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<i32>, Option<DateTime<Utc>>)>(
        r#"SELECT id, title, description, deal_price, original_price, discount_percent, end_date
           FROM deals
           WHERE business_id = $1 AND status = 'active'
           ORDER BY created_at DESC"#
    )
    .bind(biz_id)
    .fetch_all(&s.db)
    .await?;

    let deal_list: Vec<Value> = deals.into_iter().map(|(id, title, desc, deal_price, orig_price, discount, end_date)| {
        json!({
            "id": id, "title": title, "description": desc,
            "deal_price": deal_price, "original_price": orig_price,
            "discount_percent": discount, "end_date": end_date,
        })
    }).collect();

    // Check if business is verified/claimed
    let is_claimed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM claimed_businesses WHERE business_id = $1)"
    )
    .bind(biz_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(false);

    // Hours
    let hours: Option<Value> = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT hours FROM business_meta WHERE business_id = $1 AND hours IS NOT NULL LIMIT 1"
    )
    .bind(biz_id)
    .fetch_optional(&s.db)
    .await
    .ok()
    .flatten();

    Ok(Json(json!({
        "business": {
            "id": biz_id,
            "name": biz_name,
            "slug": biz_slug,
            "description": biz_desc,
            "phone": biz_phone,
            "website": biz_website,
            "address": biz_address,
            "city": biz_city,
            "state": biz_state,
            "latitude": biz_lat,
            "longitude": biz_lng,
            "category": category_name,
            "rating": biz_rating,
            "review_count": biz_review_count,
            "is_claimed": is_claimed,
            "image_url": null,
        },
        "directory_name": dir_name,
        "directory_slug": slug,
        "reviews": review_list,
        "deals": deal_list,
        "hours": hours,
    })))
}
