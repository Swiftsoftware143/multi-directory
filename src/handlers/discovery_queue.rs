//! Discovery queue (round 5, T3) — David's populate workflow.
//!
//! 1. the admin picks a category (the query is derived from category + city)
//! 2. search results are PUSHED INTO A QUEUE that survives a page reload
//! 3. every queued row carries a category auto-mapped from the Google `types`
//! 4. `src/utils/franchise.rs` runs on every row: matches get `is_franchise`
//!    (badge, auto-unchecked, one-click override in the UI)
//! 5. rows already in the directory are marked `is_duplicate` and cannot be published
//! 6. the admin ticks rows and bulk-adds them to the directory
//!
//! Endpoints (all admin-authenticated — see routes.rs, the /admin/ namespace is not public):
//! - POST   /api/v1/zaarhub/admin/discovery/queue               push search results
//! - GET    /api/v1/zaarhub/admin/discovery/queue?directory_id= list + counts
//! - POST   /api/v1/zaarhub/admin/discovery/queue/add-selected  bulk publish
//! - DELETE /api/v1/zaarhub/admin/discovery/queue?directory_id= clear the queue

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct QueueItem {
    pub place_id: Option<String>,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub rating: Option<f64>,
    pub review_count: Option<i64>,
    pub types: Option<Vec<String>>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct QueuePushRequest {
    pub directory_id: Uuid,
    pub items: Vec<QueueItem>,
    /// The category the admin picked in the panel — the fallback when the Google
    /// types cannot be mapped to an existing directory category.
    pub category_id: Option<Uuid>,
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DirectoryQuery {
    pub directory_id: Uuid,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddSelectedRequest {
    pub directory_id: Uuid,
    pub ids: Vec<Uuid>,
}

/// Google Places `types` -> keywords that appear in the directory's own category names.
const TYPE_HINTS: &[(&str, &[&str])] = &[
    ("dentist", &["dent", "dental", "health", "medical"]),
    ("doctor", &["health", "medical", "doctor", "clinic"]),
    ("hospital", &["health", "medical", "hospital"]),
    ("pharmacy", &["health", "pharmacy", "retail"]),
    ("veterinary_care", &["pet", "vet", "animal"]),
    ("restaurant", &["restaurant", "food", "dining", "eat"]),
    ("cafe", &["cafe", "coffee", "restaurant", "food"]),
    ("bakery", &["bakery", "food", "restaurant"]),
    ("bar", &["bar", "nightlife", "restaurant"]),
    (
        "meal_takeaway",
        &["restaurant", "food", "takeout", "dining"],
    ),
    (
        "car_repair",
        &["auto", "automotive", "car", "mechanic", "repair"],
    ),
    ("car_dealer", &["auto", "automotive", "car", "dealer"]),
    ("car_wash", &["auto", "car wash", "automotive"]),
    ("lawyer", &["legal", "law", "attorney"]),
    (
        "real_estate_agency",
        &["real estate", "realtor", "property"],
    ),
    ("insurance_agency", &["insurance", "financial", "finance"]),
    (
        "accounting",
        &["accounting", "tax", "financial", "bookkeeping"],
    ),
    ("bank", &["bank", "financial", "finance"]),
    ("beauty_salon", &["beauty", "salon", "spa"]),
    ("hair_care", &["hair", "beauty", "salon", "barber"]),
    ("spa", &["spa", "beauty", "wellness"]),
    ("gym", &["fitness", "gym", "health"]),
    ("plumber", &["plumb", "home service", "contractor", "trade"]),
    (
        "electrician",
        &["electric", "home service", "contractor", "trade"],
    ),
    (
        "roofing_contractor",
        &["roof", "home service", "contractor", "trade"],
    ),
    (
        "general_contractor",
        &["contractor", "construction", "home service", "trade"],
    ),
    ("painter", &["paint", "contractor", "home service", "trade"]),
    ("locksmith", &["lock", "home service", "security"]),
    ("moving_company", &["moving", "home service", "storage"]),
    ("storage", &["storage", "self storage"]),
    ("laundry", &["laundry", "clean", "dry clean"]),
    ("lodging", &["hotel", "lodging", "motel", "travel"]),
    ("travel_agency", &["travel", "tourism"]),
    ("school", &["school", "education", "tutor"]),
    ("child_care_agency", &["child", "daycare", "education"]),
    ("church", &["church", "religious", "place of worship"]),
    ("real_estate", &["real estate", "property"]),
    ("florist", &["florist", "flower", "gift"]),
    ("jewelry_store", &["jewel", "retail", "shopping"]),
    ("furniture_store", &["furniture", "home", "retail"]),
    (
        "hardware_store",
        &["hardware", "home improvement", "retail"],
    ),
    ("home_goods_store", &["home", "retail", "shopping"]),
    (
        "clothing_store",
        &["clothing", "apparel", "retail", "shopping"],
    ),
    ("shoe_store", &["shoe", "retail", "shopping"]),
    ("electronics_store", &["electronic", "retail", "shopping"]),
    ("book_store", &["book", "retail", "shopping"]),
    ("liquor_store", &["liquor", "retail", "shopping"]),
    ("pet_store", &["pet", "retail", "animal"]),
    ("supermarket", &["grocery", "supermarket", "retail", "food"]),
    ("convenience_store", &["convenience", "grocery", "retail"]),
    ("department_store", &["department", "retail", "shopping"]),
    ("shopping_mall", &["shopping", "mall", "retail"]),
    ("movie_theater", &["entertainment", "movie", "theater"]),
    ("gym_and_fitness", &["fitness", "gym"]),
    ("painting", &["paint", "contractor"]),
    ("physiotherapist", &["health", "therapy", "medical"]),
    ("chiropractor", &["health", "chiro", "medical"]),
    ("dentist_office", &["dental", "health"]),
    (
        "local_government_office",
        &["government", "civic", "public"],
    ),
    ("post_office", &["post", "government", "shipping"]),
    ("gas_station", &["gas", "fuel", "auto"]),
    ("fire_station", &["fire", "emergency", "public"]),
    ("police", &["police", "emergency", "public"]),
    ("locksmith_service", &["lock", "security"]),
    ("barber_shop", &["barber", "hair", "beauty"]),
    ("nails_salon", &["nail", "beauty", "salon"]),
    ("tattoo", &["tattoo", "beauty"]),
];

/// Slugify a business name for the `businesses.slug` column.
fn slugify(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "business".to_string()
    } else {
        out.chars().take(80).collect()
    }
}

/// Map Google Places `types` to the closest EXISTING directory category.
/// Falls back to the category the admin picked when nothing matches.
async fn map_category(
    s: &AppState,
    directory_id: Uuid,
    types: &[String],
    fallback_id: Option<Uuid>,
    fallback_name: Option<String>,
) -> (Option<Uuid>, Option<String>) {
    let rows = sqlx::query(
        "SELECT id, name FROM directory_categories \
         WHERE directory_id IS NULL OR directory_id = $1",
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    let cats: Vec<(Uuid, String)> = rows
        .iter()
        .map(|r| {
            (
                r.get::<Uuid, _>("id"),
                r.get::<Option<String>, _>("name").unwrap_or_default(),
            )
        })
        .collect();

    for t in types {
        let tl = t.to_lowercase();
        if tl == "point_of_interest" || tl == "establishment" {
            continue;
        }
        let hints: Vec<String> = TYPE_HINTS
            .iter()
            .filter(|(gt, _)| *gt == tl.as_str())
            .flat_map(|(_, hs)| hs.iter().map(|h| h.to_string()))
            .collect();
        let mut cleaned = tl.replace('_', " ");
        if cleaned == "food" {
            cleaned = "food".to_string();
        }
        for (id, name) in &cats {
            let nl = name.to_lowercase();
            if nl.contains(&cleaned)
                || hints
                    .iter()
                    .any(|h| nl.contains(&h.to_lowercase().as_str() as &str))
            {
                return (Some(*id), Some(name.clone()));
            }
        }
    }

    (fallback_id, fallback_name)
}

/// True when this business is already in the directory (name + address, the
/// strongest match available: `businesses` has no place_id column).
async fn already_listed(
    s: &AppState,
    directory_id: Uuid,
    name: &str,
    address: &str,
) -> Result<bool, AppError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses \
         WHERE directory_id = $1 \
           AND lower(name) = lower($2) \
           AND lower(COALESCE(address, '')) = lower(COALESCE($3, ''))",
    )
    .bind(directory_id)
    .bind(name)
    .bind(address)
    .fetch_one(&s.db)
    .await?;
    Ok(n > 0)
}

async fn queue_snapshot(s: &AppState, directory_id: Uuid, status: Option<&str>) -> Value {
    let status = status.unwrap_or("queued");
    let rows = sqlx::query(
        "SELECT id, place_id, name, address, city, phone, website, rating, review_count, \
                types, mapped_category, mapped_category_id, is_franchise, is_duplicate, \
                selected, status, created_at::text \
         FROM discovery_queue WHERE directory_id = $1 AND status = $2 \
         ORDER BY is_franchise ASC, is_duplicate ASC, name ASC",
    )
    .bind(directory_id)
    .bind(status)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "place_id": r.get::<Option<String>, _>("place_id"),
                "name": r.get::<String, _>("name"),
                "address": r.get::<Option<String>, _>("address"),
                "city": r.get::<Option<String>, _>("city"),
                "phone": r.get::<Option<String>, _>("phone"),
                "website": r.get::<Option<String>, _>("website"),
                "rating": r.get::<Option<f64>, _>("rating"),
                "review_count": r.get::<Option<i32>, _>("review_count"),
                "types": r.get::<Vec<String>, _>("types"),
                "category": r.get::<Option<String>, _>("mapped_category"),
                "category_id": r.get::<Option<Uuid>, _>("mapped_category_id"),
                "is_franchise": r.get::<bool, _>("is_franchise"),
                "is_duplicate": r.get::<bool, _>("is_duplicate"),
                "selected": r.get::<bool, _>("selected"),
                "status": r.get::<String, _>("status"),
            })
        })
        .collect();

    let total = items.len();
    let franchises = items
        .iter()
        .filter(|i| i["is_franchise"].as_bool().unwrap_or(false))
        .count();
    let duplicates = items
        .iter()
        .filter(|i| i["is_duplicate"].as_bool().unwrap_or(false))
        .count();
    let selected = items
        .iter()
        .filter(|i| i["selected"].as_bool().unwrap_or(false))
        .count();

    json!({
        "directory_id": directory_id,
        "items": items,
        "counts": { "total": total, "franchise": franchises,
                    "duplicate": duplicates, "selected": selected }
    })
}

/// POST /api/v1/zaarhub/admin/discovery/queue — push search results into the queue.
pub async fn push_results(
    State(s): State<AppState>,
    Json(req): Json<QueuePushRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.items.is_empty() {
        return Err(AppError::BadRequest("items is empty".into()));
    }

    let mut queued = 0usize;
    let mut dupes = 0usize;
    let mut franchises = 0usize;

    for it in &req.items {
        let name = it.name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let address = it.address.clone().unwrap_or_default();
        let types: Vec<String> = it.types.clone().unwrap_or_default();

        let is_franchise = crate::utils::franchise::is_likely_franchise(&name, &types);
        if is_franchise {
            franchises += 1;
        }
        let listed = already_listed(&s, req.directory_id, &name, &address).await?;
        if listed {
            dupes += 1;
        }
        let (cat_id, cat_name) = map_category(
            &s,
            req.directory_id,
            &types,
            req.category_id,
            req.category.clone(),
        )
        .await;

        let raw = json!({
            "types": types.clone(),
            "lat": it.lat, "lng": it.lng,
        });

        let res = sqlx::query(
            "INSERT INTO discovery_queue \
                (directory_id, place_id, name, address, city, phone, website, rating, \
                 review_count, types, mapped_category_id, mapped_category, is_franchise, \
                 is_duplicate, selected, raw) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) \
             ON CONFLICT DO NOTHING",
        )
        .bind(req.directory_id)
        .bind(it.place_id.clone())
        .bind(&name)
        .bind(&address)
        .bind(it.city.clone())
        .bind(it.phone.clone())
        .bind(it.website.clone())
        .bind(it.rating.unwrap_or(0.0))
        .bind(it.review_count.unwrap_or(0) as i32)
        .bind(&types)
        .bind(cat_id)
        .bind(cat_name.clone())
        .bind(is_franchise)
        .bind(listed)
        // Excluded by default: franchises and already-listed rows start unticked.
        .bind(!is_franchise && !listed)
        .bind(&raw)
        .execute(&s.db)
        .await?;
        if res.rows_affected() > 0 {
            queued += 1;
        }
    }

    let snap = queue_snapshot(&s, req.directory_id, None).await;
    Ok(Json(json!({
        "success": true,
        "queued": queued,
        "franchises": franchises,
        "already_listed": dupes,
        "stored": snap,
    })))
}

/// GET /api/v1/zaarhub/admin/discovery/queue?directory_id= — the persisted queue.
pub async fn list_queue(
    State(s): State<AppState>,
    Query(q): Query<DirectoryQuery>,
) -> ApiResult<impl IntoResponse> {
    let snap = queue_snapshot(&s, q.directory_id, q.status.as_deref()).await;
    Ok(Json(json!({ "success": true, "data": snap })))
}

/// POST /api/v1/zaarhub/admin/discovery/queue/add-selected — bulk publish.
pub async fn add_selected(
    State(s): State<AppState>,
    Json(req): Json<AddSelectedRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.ids.is_empty() {
        return Err(AppError::BadRequest("no rows selected".into()));
    }

    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for id in &req.ids {
        let row = sqlx::query(
            "SELECT name, address, city, phone, website, rating, review_count, types, \
                    mapped_category_id, is_franchise \
             FROM discovery_queue WHERE id = $1 AND directory_id = $2",
        )
        .bind(id)
        .bind(req.directory_id)
        .fetch_optional(&s.db)
        .await?;

        let Some(r) = row else {
            skipped += 1;
            continue;
        };

        let name: String = r.get("name");
        let address: String = r.get::<Option<String>, _>("address").unwrap_or_default();
        if already_listed(&s, req.directory_id, &name, &address).await? {
            skipped += 1;
            let _ = sqlx::query(
                "UPDATE discovery_queue SET is_duplicate = true, selected = false, updated_at = NOW() \
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&s.db)
            .await;
            continue;
        }

        let mut slug = slugify(&name);
        // Guarantee a unique slug without a second round trip for the common case.
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM businesses WHERE slug = $1")
            .bind(&slug)
            .fetch_one(&s.db)
            .await?;
        if exists > 0 {
            let suffix = id.simple().to_string();
            slug = format!(
                "{}-{}",
                slug.chars().take(70).collect::<String>(),
                &suffix[..6]
            );
        }

        let category_id: Option<Uuid> = r.get("mapped_category_id");
        let types: Vec<String> = r.get("types");

        let ins = sqlx::query(
            "INSERT INTO businesses \
                (directory_id, name, slug, description, category_id, address, city, phone, \
                 website, rating, review_count, business_type, is_franchise) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'local',$12)",
        )
        .bind(req.directory_id)
        .bind(&name)
        .bind(&slug)
        .bind(&address)
        .bind(category_id)
        .bind(&address)
        .bind(r.get::<Option<String>, _>("city"))
        .bind(r.get::<Option<String>, _>("phone"))
        .bind(r.get::<Option<String>, _>("website"))
        .bind(r.get::<Option<f64>, _>("rating").unwrap_or(0.0))
        .bind(r.get::<Option<i32>, _>("review_count").unwrap_or(0))
        .bind(r.get::<bool, _>("is_franchise"))
        .execute(&s.db)
        .await;

        match ins {
            Ok(_) => {
                added += 1;
                sqlx::query(
                    "UPDATE discovery_queue SET status = 'added', selected = false, updated_at = NOW() \
                     WHERE id = $1",
                )
                .bind(id)
                .execute(&s.db)
                .await?;
            }
            Err(e) => {
                errors.push(format!("{}: {}", name, e));
            }
        }
        let _ = types; // kept in the queue row for the category mapping trail
    }

    let snap = queue_snapshot(&s, req.directory_id, None).await;
    Ok(Json(json!({
        "success": true,
        "added": added,
        "skipped": skipped,
        "errors": errors,
        "stored": snap,
    })))
}

/// Round 6 (U2) — tick / untick queue rows from the panel (Select all + per-row check).
#[derive(Debug, Deserialize)]
pub struct QueueSelectionRequest {
    pub directory_id: Uuid,
    pub ids: Vec<Uuid>,
    pub selected: bool,
}

/// Round 6 (U2) — re-assign the auto-mapped category of ONE queue row before publishing.
#[derive(Debug, Deserialize)]
pub struct QueueCategoryRequest {
    pub directory_id: Uuid,
    pub id: Uuid,
    pub category_id: Uuid,
}

/// POST /api/v1/zaarhub/admin/discovery/queue/select — bulk tick/untick.
pub async fn set_selection(
    State(s): State<AppState>,
    Json(req): Json<QueueSelectionRequest>,
) -> ApiResult<impl IntoResponse> {
    if !req.ids.is_empty() {
        sqlx::query(
            "UPDATE discovery_queue SET selected = $3, updated_at = NOW() \
             WHERE directory_id = $1 AND id = ANY($2)",
        )
        .bind(req.directory_id)
        .bind(&req.ids)
        .bind(req.selected)
        .execute(&s.db)
        .await?;
    }
    let snap = queue_snapshot(&s, req.directory_id, None).await;
    Ok(Json(json!({ "success": true, "stored": snap })))
}

/// POST /api/v1/zaarhub/admin/discovery/queue/category — override the mapped category.
/// The category name is read from the `categories` table, never trusted from the client.
pub async fn set_category(
    State(s): State<AppState>,
    Json(req): Json<QueueCategoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
        .bind(req.category_id)
        .fetch_optional(&s.db)
        .await?;
    let Some(name) = name else {
        return Err(AppError::BadRequest("unknown category".into()));
    };

    let res = sqlx::query(
        "UPDATE discovery_queue SET mapped_category_id = $3, mapped_category = $4, \
         updated_at = NOW() WHERE directory_id = $1 AND id = $2",
    )
    .bind(req.directory_id)
    .bind(req.id)
    .bind(req.category_id)
    .bind(&name)
    .execute(&s.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("queue row not found".into()));
    }

    let snap = queue_snapshot(&s, req.directory_id, None).await;
    Ok(Json(json!({ "success": true, "stored": snap })))
}

/// DELETE /api/v1/zaarhub/admin/discovery/queue?directory_id= — clear queued rows.
pub async fn clear_queue(
    State(s): State<AppState>,
    Query(q): Query<DirectoryQuery>,
) -> ApiResult<impl IntoResponse> {
    let res =
        sqlx::query("DELETE FROM discovery_queue WHERE directory_id = $1 AND status = 'queued'")
            .bind(q.directory_id)
            .execute(&s.db)
            .await?;
    Ok(Json(
        json!({ "success": true, "cleared": res.rows_affected() }),
    ))
}
