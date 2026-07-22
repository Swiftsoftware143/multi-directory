//! Neighborhood Feed handler — personalized landing page for visitors.
//!
//! Aggregates bookmarks, upcoming RSVP'd events, active polls, and
//! tailored business suggestions based on visitor interests.
//!
//! GET /api/v1/feed — JSON feed data (visitor JWT required)
//! GET /api/v1/feed-page — server-rendered feed.hbs template (visitor JWT required)

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::verify_token;
use crate::error::{AppError, ApiResult};
use crate::AppState;
use crate::template_engine;

// ── Data Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct VisitorAccount {
    pub id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub directory_id: Option<Uuid>,
    pub is_active: bool,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct FeedBusiness {
    pub id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub category: Option<String>,
    pub category_id: Option<Uuid>,
    pub image_url: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub rating: Option<f64>,
    pub review_count: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct SavedBusiness {
    pub id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub category: Option<String>,
    pub image_url: Option<String>,
    pub bookmarked_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct UpcomingEvent {
    pub id: Uuid,
    pub title: String,
    pub event_date: DateTime<Utc>,
    pub location: Option<String>,
    pub image_url: Option<String>,
    pub rsvp_status: String,
    pub attendee_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ActivePoll {
    pub id: Uuid,
    pub question: String,
    pub options: Vec<String>,
    pub option_votes: Vec<i64>,
    pub total_votes: i64,
    pub user_vote: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct FeedResponse {
    pub greeting: String,
    pub directory: DirectoryInfo,
    pub saved_businesses: Vec<SavedBusiness>,
    pub upcoming_events: Vec<UpcomingEvent>,
    pub active_polls: Vec<ActivePoll>,
    pub suggested_businesses: Vec<FeedBusiness>,
}

#[derive(Debug, Serialize)]
pub struct DirectoryInfo {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

// ── Auth Helper ─────────────────────────────────────────────────────────────

/// Extract visitor ID from JWT Authorization header.
pub fn extract_visitor_account_id(headers: &HeaderMap, jwt_secret: &str) -> Result<Uuid, AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

/// Extract visitor ID optionally (no error if missing).
pub fn extract_visitor_id_optional(headers: &HeaderMap, jwt_secret: &str) -> Option<Uuid> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())?;

    let token = auth_header.strip_prefix("Bearer ")?;

    let claims = verify_token(token, jwt_secret).ok()?;

    Uuid::parse_str(&claims.sub).ok()
}

// ── Time-of-day greeting ────────────────────────────────────────────────────

fn time_of_day_greeting() -> &'static str {
    let hour = chrono::Local::now().hour();
    if hour < 12 {
        "Good morning"
    } else if hour < 17 {
        "Good afternoon"
    } else {
        "Good evening"
    }
}

// ── Feed Handler ────────────────────────────────────────────────────────────

/// GET /api/v1/feed — JSON neighborhood feed data
pub async fn get_feed(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = extract_visitor_account_id(&headers, &s.config.jwt_secret)?;

    // ── 1. Get visitor account info ──
    let visitor = sqlx::query_as::<_, VisitorAccount>(
        "SELECT * FROM visitor_accounts WHERE id = $1"
    )
    .bind(visitor_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Visitor not found".to_string()))?;

    let directory_id = visitor.directory_id
        .ok_or_else(|| AppError::NotFound("Visitor has no directory assigned".to_string()))?;

    // ── 2. Get directory info ──
    let dir_row = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directories WHERE id = $1"
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let directory = DirectoryInfo {
        id: dir_row.0,
        name: dir_row.1,
        slug: dir_row.2,
    };

    // ── 3. Fetch saved businesses (bookmarks) ──
    let saved_raw = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<serde_json::Value>, DateTime<Utc>)>(
        r#"SELECT b.id, b.name, b.slug, dc.name as category_name, b.images, vf.created_at as bookmarked_at
           FROM visitor_favorites vf
           JOIN businesses b ON b.id = vf.business_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE vf.visitor_account_id = $1
           ORDER BY vf.created_at DESC
           LIMIT 6"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    let saved_businesses: Vec<SavedBusiness> = saved_raw.into_iter().map(|(id, name, slug, category, images, bookmarked_at)| {
        let image_url = images.as_ref()
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        SavedBusiness { id, name, slug, category, image_url, bookmarked_at }
    }).collect();

    // Collect categories from bookmarked businesses for suggestions
    let bookmarked_categories: Vec<String> = saved_businesses.iter()
        .filter_map(|b| b.category.clone())
        .collect();

    // ── 4. Fetch upcoming RSVP'd events ──
    let events_raw = sqlx::query_as::<_, (Uuid, String, DateTime<Utc>, Option<String>, String, Option<serde_json::Value>)>(
        r#"SELECT ce.id, ce.title, ce.event_date, ce.location, er.status, ce.image_url
           FROM event_rsvps er
           JOIN community_events ce ON ce.id = er.event_id
           WHERE er.visitor_account_id = $1
             AND er.status IN ('going', 'maybe')
             AND ce.event_date > NOW()
             AND ce.status = 'active'
           ORDER BY ce.event_date ASC
           LIMIT 5"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    let mut upcoming_events: Vec<UpcomingEvent> = Vec::new();
    for (id, title, event_date, location, rsvp_status, image_url) in events_raw {
        let attendee_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM event_rsvps WHERE event_id = $1 AND status IN ('going', 'maybe')"
        )
        .bind(id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        let img = image_url.as_ref()
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        upcoming_events.push(UpcomingEvent {
            id,
            title,
            event_date,
            location,
            image_url: img,
            rsvp_status,
            attendee_count,
        });
    }

    // ── 5. Fetch active polls ──
    let polls_raw = sqlx::query_as::<_, (Uuid, String, Vec<String>, i32)>(
        r#"SELECT id, question, options, 0 as dummy
           FROM polls
           WHERE directory_id = $1 AND status = 'active'
           ORDER BY created_at DESC
           LIMIT 3"#
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let mut active_polls: Vec<ActivePoll> = Vec::new();
    for (poll_id, question, options, _) in polls_raw {
        let total_votes = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM poll_votes WHERE poll_id = $1"
        )
        .bind(poll_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        let mut option_votes: Vec<i64> = vec![0i64; options.len()];
        let vote_rows = sqlx::query_as::<_, (i32, i64)>(
            "SELECT option_index, COUNT(*)::bigint FROM poll_votes WHERE poll_id = $1 GROUP BY option_index"
        )
        .bind(poll_id)
        .fetch_all(&s.db)
        .await
        .unwrap_or_default();

        for (idx, count) in vote_rows {
            if (idx as usize) < option_votes.len() {
                option_votes[idx as usize] = count;
            }
        }

        let user_vote: Option<i32> = sqlx::query_scalar::<_, i32>(
            "SELECT option_index FROM poll_votes WHERE poll_id = $1 AND visitor_account_id = $2"
        )
        .bind(poll_id)
        .bind(visitor_id)
        .fetch_optional(&s.db)
        .await
        .unwrap_or(None)
        .map(|v| v as i32);

        active_polls.push(ActivePoll {
            id: poll_id,
            question,
            options,
            option_votes,
            total_votes,
            user_vote,
        });
    }

    // ── 6. Fetch suggested businesses ──
    // Build interest categories from:
    // a) categories from bookmarked businesses
    // b) categories from survey responses
    let mut interest_categories: Vec<String> = bookmarked_categories.clone();

    // Get survey category data — survey questions may have 'category' or 'interest' type answers
    let survey_interests: Vec<String> = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT jsonb_array_elements_text(
               CASE WHEN answers ? 'categories' THEN answers->'categories'
                    WHEN answers ? 'interests' THEN answers->'interests'
                    WHEN answers ? 'business_type' THEN jsonb_build_array(answers->>'business_type')
                    ELSE '[]'::jsonb
               END
           ) FROM survey_responses WHERE visitor_account_id = $1"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    for interest in survey_interests {
        let trimmed = interest.trim().to_lowercase();
        if !trimmed.is_empty() && !interest_categories.iter().any(|c| c.to_lowercase() == trimmed) {
            interest_categories.push(interest.trim().to_string());
        }
    }

    // Get already-bookmarked business IDs to exclude
    let bookmarked_ids: Vec<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT business_id FROM visitor_favorites WHERE visitor_account_id = $1"
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    // Fetch suggested businesses
    let suggested_businesses: Vec<FeedBusiness> = if interest_categories.is_empty() {
        // No interests — show popular businesses in directory
        let raw = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<String>, Option<String>, Option<f64>, Option<i32>)>(
            r#"SELECT b.id, b.name, b.slug, dc.name as category, b.images, b.city, b.state, b.rating, b.review_count
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.directory_id = $1
               ORDER BY b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST
               LIMIT 6"#
        )
        .bind(directory_id)
        .fetch_all(&s.db)
        .await?;

        raw.into_iter().filter_map(|(id, name, slug, category, images, city, state, rating, review_count)| {
            if bookmarked_ids.contains(&id) { return None; }
            let image_url = images.as_ref()
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(FeedBusiness { id, name, slug, category, category_id: None, image_url, city, state, rating, review_count })
        }).collect()
    } else {
        // Match businesses by category name matching our interest categories
        let raw = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<String>, Option<String>, Option<f64>, Option<i32>, Option<Uuid>)>(
            r#"SELECT b.id, b.name, b.slug, dc.name as category, b.images, b.city, b.state, b.rating, b.review_count, dc.id as category_id
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.directory_id = $1
                 AND LOWER(dc.name) = ANY($2)
               ORDER BY b.rating DESC NULLS LAST, b.review_count DESC NULLS LAST
               LIMIT 12"#
        )
        .bind(directory_id)
        .bind(&interest_categories.iter().map(|c| c.to_lowercase()).collect::<Vec<_>>())
        .fetch_all(&s.db)
        .await?;

        raw.into_iter().filter_map(|(id, name, slug, category, images, city, state, rating, review_count, category_id)| {
            if bookmarked_ids.contains(&id) { return None; }
            let image_url = images.as_ref()
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(FeedBusiness { id, name, slug, category, category_id, image_url, city, state, rating, review_count })
        }).take(6).collect()
    };

    // ── 7. Build greeting ──
    let first_name = visitor.name
        .as_deref()
        .and_then(|n| n.split_whitespace().next())
        .unwrap_or("there");
    let greeting = format!("{}, {}! 👋", time_of_day_greeting(), first_name);

    // ── 8. Return response ──
    let response = FeedResponse {
        greeting,
        directory,
        saved_businesses,
        upcoming_events,
        active_polls,
        suggested_businesses,
    };

    Ok(Json(json!(response)))
}

// ── Server-rendered Feed Page ───────────────────────────────────────────────

/// GET /api/v1/feed-page — server-rendered feed.hbs template page
pub async fn feed_page(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = match extract_visitor_id_optional(&headers, &s.config.jwt_secret) {
        Some(id) => id,
        None => {
            return Ok(axum::response::Redirect::to("/visitor").into_response());
        }
    };

    // ── Get visitor info ──
    let visitor = sqlx::query_as::<_, VisitorAccount>(
        "SELECT * FROM visitor_accounts WHERE id = $1"
    )
    .bind(visitor_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Visitor not found".to_string()))?;

    let directory_id = match visitor.directory_id {
        Some(id) => id,
        None => {
            return Ok(axum::response::Redirect::to("/visitor").into_response());
        }
    };

    // ── Get directory info ──
    let dir_row = sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<serde_json::Value>)>(
        r#"SELECT id, name, description, slug, color_scheme FROM directories WHERE id = $1"#
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".to_string()))?;

    let (dir_uuid, dir_name, _dir_desc, dir_slug, color_scheme) = dir_row;

    // ── Build all the data sections (same queries as JSON endpoint) ──

    // Saved businesses
    let saved_raw = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<serde_json::Value>, DateTime<Utc>)>(
        r#"SELECT b.id, b.name, b.slug, dc.name as category_name, b.images, vf.created_at as bookmarked_at
           FROM visitor_favorites vf
           JOIN businesses b ON b.id = vf.business_id
           LEFT JOIN directory_categories dc ON dc.id = b.category_id
           WHERE vf.visitor_account_id = $1
           ORDER BY vf.created_at DESC
           LIMIT 6"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    let saved_places: Vec<serde_json::Value> = saved_raw.into_iter().map(|(id, name, slug, category, images, bookmarked_at)| {
        let image_url = images.as_ref()
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        json!({
            "id": id,
            "name": name,
            "slug": slug,
            "category_name": category,
            "image_url": image_url,
            "bookmarked_at": bookmarked_at,
        })
    }).collect();

    // Upcoming events
    let events_raw = sqlx::query_as::<_, (Uuid, String, DateTime<Utc>, Option<String>, String, Option<serde_json::Value>)>(
        r#"SELECT ce.id, ce.title, ce.event_date, ce.location, er.status, ce.image_url
           FROM event_rsvps er
           JOIN community_events ce ON ce.id = er.event_id
           WHERE er.visitor_account_id = $1
             AND er.status IN ('going', 'maybe')
             AND ce.event_date > NOW()
             AND ce.status = 'active'
           ORDER BY ce.event_date ASC
           LIMIT 5"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await?;

    let mut events_list: Vec<serde_json::Value> = Vec::new();
    for (id, title, event_date, location, rsvp_status, image_url) in events_raw {
        let attendee_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM event_rsvps WHERE event_id = $1 AND status IN ('going', 'maybe')"
        )
        .bind(id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        let img = image_url.as_ref()
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        events_list.push(json!({
            "id": id,
            "title": title,
            "event_date": event_date,
            "location": location,
            "rsvp_status": rsvp_status,
            "attendee_count": attendee_count,
            "image_url": img,
        }));
    }

    // Active polls
    let polls_raw = sqlx::query_as::<_, (Uuid, String, Vec<String>)>(
        r#"SELECT id, question, options FROM polls WHERE directory_id = $1 AND status = 'active' ORDER BY created_at DESC LIMIT 3"#
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let mut polls_list: Vec<serde_json::Value> = Vec::new();
    for (poll_id, question, options) in polls_raw {
        let total_votes = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM poll_votes WHERE poll_id = $1"
        )
        .bind(poll_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        let mut option_votes: Vec<i64> = vec![0i64; options.len()];
        let vote_rows = sqlx::query_as::<_, (i32, i64)>(
            "SELECT option_index, COUNT(*)::bigint FROM poll_votes WHERE poll_id = $1 GROUP BY option_index"
        )
        .bind(poll_id)
        .fetch_all(&s.db)
        .await
        .unwrap_or_default();

        for (idx, count) in vote_rows {
            if (idx as usize) < option_votes.len() {
                option_votes[idx as usize] = count;
            }
        }

        let user_vote: Option<i32> = sqlx::query_scalar::<_, i32>(
            "SELECT option_index FROM poll_votes WHERE poll_id = $1 AND visitor_account_id = $2"
        )
        .bind(poll_id)
        .bind(visitor_id)
        .fetch_optional(&s.db)
        .await
        .unwrap_or(None)
        .map(|v| v as i32);

        polls_list.push(json!({
            "id": poll_id,
            "question": question,
            "options": options,
            "option_votes": option_votes,
            "total_votes": total_votes,
            "user_vote": user_vote,
        }));
    }

    // Suggested businesses
    let bookmarked_categories: Vec<String> = saved_places.iter()
        .filter_map(|b| b.get("category_name").and_then(|c| c.as_str()).map(|s| s.to_lowercase()))
        .collect();

    let survey_interests: Vec<String> = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT jsonb_array_elements_text(
               CASE WHEN answers ? 'categories' THEN answers->'categories'
                    WHEN answers ? 'interests' THEN answers->'interests'
                    WHEN answers ? 'business_type' THEN jsonb_build_array(answers->>'business_type')
                    ELSE '[]'::jsonb
               END
           ) FROM survey_responses WHERE visitor_account_id = $1"#
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    let mut interest_categories: Vec<String> = bookmarked_categories.clone();
    for interest in survey_interests {
        let trimmed = interest.trim().to_lowercase();
        if !trimmed.is_empty() && !interest_categories.contains(&trimmed) {
            interest_categories.push(trimmed);
        }
    }

    let bookmarked_ids: Vec<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT business_id FROM visitor_favorites WHERE visitor_account_id = $1"
    )
    .bind(visitor_id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    let suggestions: Vec<serde_json::Value> = if interest_categories.is_empty() {
        sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<String>, Option<String>)>(
            r#"SELECT b.id, b.name, b.slug, dc.name as category, b.images, b.city, b.state
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.directory_id = $1
               ORDER BY b.rating DESC NULLS LAST
               LIMIT 10"#
        )
        .bind(directory_id)
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .filter_map(|(id, name, slug, category, images, city, state)| {
            if bookmarked_ids.contains(&id) { return None; }
            let image_url = images.as_ref()
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(json!({
                "id": id, "name": name, "slug": slug,
                "category": category, "image_url": image_url,
                "city": city, "state": state,
            }))
        })
        .take(6)
        .collect()
    } else {
        sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<String>, Option<String>)>(
            r#"SELECT b.id, b.name, b.slug, dc.name as category, b.images, b.city, b.state
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.directory_id = $1
                 AND LOWER(dc.name) = ANY($2)
               ORDER BY b.rating DESC NULLS LAST
               LIMIT 10"#
        )
        .bind(directory_id)
        .bind(&interest_categories.iter().map(|c| c.to_lowercase()).collect::<Vec<_>>())
        .fetch_all(&s.db)
        .await?
        .into_iter()
        .filter_map(|(id, name, slug, category, images, city, state)| {
            if bookmarked_ids.contains(&id) { return None; }
            let image_url = images.as_ref()
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(json!({
                "id": id, "name": name, "slug": slug,
                "category": category, "image_url": image_url,
                "city": city, "state": state,
            }))
        })
        .take(6)
        .collect()
    };

    // ── Greeting ──
    let first_name = visitor.name
        .as_deref()
        .and_then(|n| n.split_whitespace().next())
        .unwrap_or("there");
    let greeting = format!("{}, {}! 👋", time_of_day_greeting(), first_name);

    let colors = template_engine::normalize_color_scheme(color_scheme);

    let context = json!({
        "greeting": greeting,
        "directory": {
            "id": dir_uuid,
            "name": dir_name,
            "slug": dir_slug,
        },
        "colors": colors,
        "saved_places": saved_places,
        "has_saved_places": !saved_places.is_empty(),
        "upcoming_events": events_list,
        "has_upcoming_events": !events_list.is_empty(),
        "active_polls": polls_list,
        "has_active_polls": !polls_list.is_empty(),
        "suggested_businesses": suggestions,
        "has_suggestions": !suggestions.is_empty(),
        // Referral info for feed page
        "referral": get_visitor_referral_context(&s.db, visitor_id).await.unwrap_or_default(),
    });

    let engine = s.template_engine.lock().unwrap();
    let html = engine
        .render_directory_page(template_engine::TEMPLATE_FEED, &context)
        .map_err(|e| AppError::Internal(e))?;

    Ok(axum::response::Html(html).into_response())
}

/// Get referral code and stats for the visitor — returns serde_json Value for template context
async fn get_visitor_referral_context(
    db: &sqlx::PgPool,
    visitor_id: Uuid,
) -> Result<serde_json::Value, sqlx::Error> {
    // Get the visitor's email for looking up referrals
    let email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM visitor_accounts WHERE id = $1"
    )
    .bind(visitor_id)
    .fetch_optional(db)
    .await?
    .flatten();

    // If no email found, return empty referral context
    let email = match email {
        Some(e) => e,
        None => return Ok(serde_json::json!(null)),
    };

    // Find existing referral code for this visitor
    let code: Option<String> = sqlx::query_scalar(
        "SELECT referral_code FROM referrals WHERE referrer_id = $1::text::uuid AND referrer_type = 'visitor' AND status != 'expired' LIMIT 1"
    )
    .bind(visitor_id.to_string())
    .fetch_optional(db)
    .await?
    .flatten();

    // Get referral stats
    let total_referrals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM referrals WHERE referrer_email = $1 AND referrer_type = 'visitor' AND referee_id IS NOT NULL"
    )
    .bind(&email)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    let confirmed_referrals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM referrals WHERE referrer_email = $1 AND referrer_type = 'visitor' AND status = 'paid'"
    )
    .bind(&email)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    let zaarcash_earned: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(zaarcash_earned), 0) FROM referrals WHERE referrer_email = $1 AND referrer_type = 'visitor' AND status = 'paid'"
    )
    .bind(&email)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    Ok(serde_json::json!({
        "code": code,
        "link": code.as_ref().map(|c| format!("zaarhub.com/join?ref={}", c)),
        "total_referrals": total_referrals,
        "confirmed_referrals": confirmed_referrals,
        "zaarcash_earned": zaarcash_earned,
        "has_referral_code": code.is_some(),
    }))
}
