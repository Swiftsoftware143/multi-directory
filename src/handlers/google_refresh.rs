//! W25 — **REFRESH FROM GOOGLE**: one batched run per directory.
//!
//! The directory already had two halves of this job as separate manual screens: a per-business
//! "enrich" call and a search → queue → publish flow an admin clicks through by hand. Neither
//! keeps 4,000 listings CURRENT, and nothing could be scheduled. This module is the single job
//! the card asks for:
//!
//! * **(A) UPDATE** every UNCLAIMED listing in the city from Google, in place and by identity:
//!   a claimed listing is never touched (its owner owns that data), a name+address that Google
//!   cannot corroborate is never written (see [`listing_agrees`]), and no row is ever deleted or
//!   duplicated — `place_id` is the identity, exactly as W29 established.
//! * **(B) DISCOVER** businesses Google knows in the city that are not listed yet, and add them,
//!   deduped by place id AND by name+address so a second run cannot create a second copy.
//!
//! Every run is written to `google_refresh_runs` so the panel reports
//! `added / updated / unchanged / unreachable` for the last run instead of "done".
//!
//! Schedule is an admin setting stored in `directories.zaarhub_config->google_refresh->schedule`
//! (`manual` | `daily` | `weekly` | `monthly`); `/cron/google-refresh` runs the ones that are due.

use axum::{
    extract::{Extension, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashSet;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::places_client::{
    find_place_then_details, find_place_then_details_v1, listing_agrees, PlacesOutcome,
    LEGACY_PLACES_BASE,
};
use crate::handlers::tenant_scope::assert_directory_admin;
use crate::AppState;

/// Default batch sizes. A run is bounded so an admin's click answers inside a request rather
/// than holding a connection open for 4,000 Google lookups.
const DEFAULT_MAX_UPDATE: i64 = 20;
const DEFAULT_MAX_ADD: i64 = 15;
const HARD_CAP: i64 = 200;
const DEFAULT_RADIUS_M: i64 = 12_000;
const MAX_RADIUS_M: i64 = 50_000;
/// How many categories one discovery pass searches. Each is one Google call; the city's
/// six biggest categories cover the bulk of what a directory lists.
const DISCOVER_CATEGORIES: i64 = 6;
/// A run older than this with no `finished_at` is treated as dead, not as "still running"
/// (a container restart mid-run must not lock the button forever).
const STALE_RUN_MINUTES: i32 = 30;

// ─────────────────────────────────────────────────────────────────────────────
// Request / response shapes
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub directory_id: Uuid,
    /// Run the update arm (default true).
    #[serde(default)]
    pub update: Option<bool>,
    /// Run the discover arm (default true).
    #[serde(default)]
    pub discover: Option<bool>,
    #[serde(default)]
    pub max_update: Option<i64>,
    #[serde(default)]
    pub max_add: Option<i64>,
    /// Metres around the city centre for discovery.
    #[serde(default)]
    pub radius_m: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    pub directory_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct ScheduleRequest {
    pub directory_id: Uuid,
    pub schedule: String,
}

#[derive(Debug, Default)]
struct Counts {
    added: usize,
    updated: usize,
    identified: usize,
    unchanged: usize,
    unreachable: usize,
    skipped_claimed: usize,
}

struct Opts {
    update: bool,
    discover: bool,
    max_update: i64,
    max_add: i64,
    radius_m: i64,
    trigger: String,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            update: true,
            discover: true,
            max_update: DEFAULT_MAX_UPDATE,
            max_add: DEFAULT_MAX_ADD,
            radius_m: DEFAULT_RADIUS_M,
            trigger: "manual".to_string(),
        }
    }
}

/// The few columns the refresh reads about the city it refreshes.
struct DirectoryRow {
    id: Uuid,
    name: String,
    city: String,
    state: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP handlers (all directory-scoped: operator OR the city's own admin)
// ─────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/zaarhub/admin/google-refresh/run
pub async fn run(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<RunRequest>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, req.directory_id).await?;
    let opts = Opts {
        update: req.update.unwrap_or(true),
        discover: req.discover.unwrap_or(true),
        max_update: req
            .max_update
            .unwrap_or(DEFAULT_MAX_UPDATE)
            .clamp(1, HARD_CAP),
        max_add: req.max_add.unwrap_or(DEFAULT_MAX_ADD).clamp(0, HARD_CAP),
        radius_m: req
            .radius_m
            .unwrap_or(DEFAULT_RADIUS_M)
            .clamp(1_000, MAX_RADIUS_M),
        trigger: "manual".to_string(),
    };
    match run_batch(&s, req.directory_id, opts).await {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(AppError::BadRequest(e)),
    }
}

/// GET /api/v1/zaarhub/admin/google-refresh/status?directory_id=
///
/// What the panel needs to be honest: the schedule, the last run's numbers, and how much work is
/// left (how many unclaimed listings exist and how many already carry a Google identity).
pub async fn status(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<StatusQuery>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, q.directory_id).await?;

    let schedule = schedule_of(&s, q.directory_id).await?;

    let totals: (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE COALESCE(claimed, false)), \
                COUNT(*) FILTER (WHERE NOT COALESCE(claimed, false)), \
                COUNT(*) FILTER (WHERE COALESCE(place_id, '') <> '') \
         FROM businesses WHERE directory_id = $1 AND is_active = true",
    )
    .bind(q.directory_id)
    .fetch_one(&s.db)
    .await?;

    let last: Option<(
        Uuid,
        String,
        i32,
        i32,
        i32,
        i32,
        i32,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, trigger, added, updated, identified, unchanged, unreachable, started_at \
             FROM google_refresh_runs WHERE directory_id = $1 \
             ORDER BY started_at DESC LIMIT 1",
    )
    .bind(q.directory_id)
    .fetch_optional(&s.db)
    .await?;

    let running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM google_refresh_runs \
         WHERE directory_id = $1 AND finished_at IS NULL \
           AND started_at > NOW() - ($2 || ' minutes')::interval",
    )
    .bind(q.directory_id)
    .bind(STALE_RUN_MINUTES.to_string())
    .fetch_one(&s.db)
    .await?;

    Ok(Json(match last {
        Some((id, trigger, added, updated, identified, unchanged, unreachable, started)) => json!({
            "directory_id": q.directory_id,
            "schedule": schedule,
            "running": running > 0,
            "claimed": totals.0,
            "unclaimed": totals.1,
            "with_place_id": totals.2,
            "last_run": {
                "id": id,
                "trigger": trigger,
                "started_at": started,
                "added": added,
                "updated": updated,
                "identified": identified,
                "unchanged": unchanged,
                "unreachable": unreachable,
            }
        }),
        None => json!({
            "directory_id": q.directory_id,
            "schedule": schedule,
            "running": running > 0,
            "claimed": totals.0,
            "unclaimed": totals.1,
            "with_place_id": totals.2,
            "last_run": Value::Null,
        }),
    }))
}

/// PUT /api/v1/zaarhub/admin/google-refresh/schedule — manual | daily | weekly | monthly
pub async fn set_schedule(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<ScheduleRequest>,
) -> ApiResult<impl IntoResponse> {
    assert_directory_admin(&s.db, &claims, req.directory_id).await?;
    let schedule = normalize_schedule(&req.schedule).ok_or_else(|| {
        AppError::BadRequest("schedule must be manual, daily, weekly or monthly".into())
    })?;

    let updated = sqlx::query(
        // Two nested jsonb_set calls on purpose: PostgreSQL's `create_missing` only creates the
        // LAST path element, so a single set of '{google_refresh,schedule}' on a row that has no
        // `google_refresh` key is a silent no-op (measured — the panel said "saved" and the value
        // never landed). Create the parent object first, then the key inside it.
        "UPDATE directories SET zaarhub_config = jsonb_set( \
             jsonb_set(COALESCE(zaarhub_config, '{}'::jsonb), \
                       '{google_refresh}', \
                       COALESCE(zaarhub_config->'google_refresh', '{}'::jsonb), true), \
             '{google_refresh,schedule}', to_jsonb($2::text), true), \
         updated_at = NOW() WHERE id = $1",
    )
    .bind(req.directory_id)
    .bind(&schedule)
    .execute(&s.db)
    .await?
    .rows_affected();

    if updated == 0 {
        return Err(AppError::NotFound("directory not found".into()));
    }
    Ok(Json(
        json!({ "directory_id": req.directory_id, "schedule": schedule }),
    ))
}

/// POST /api/v1/cron/google-refresh — start every city that is DUE, in the background.
///
/// Returns as soon as the work is queued (a batch of cities takes minutes; the scheduler must
/// not sit on the connection). Each city's run is written to `google_refresh_runs` either way,
/// so a silent failure is visible in the panel afterwards.
pub async fn cron_refresh(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT d.id, COALESCE(d.zaarhub_config->'google_refresh'->>'schedule', 'weekly') \
         FROM directories d \
         WHERE COALESCE(d.status, 'active') <> 'deleted' \
           AND COALESCE(zaarhub_config->'google_refresh'->>'schedule', 'weekly') <> 'manual'",
    )
    .fetch_all(&s.db)
    .await?;

    let mut due: Vec<Uuid> = Vec::new();
    for (id, schedule) in rows {
        if is_due(&s, id, &schedule).await.unwrap_or(false) {
            due.push(id);
        }
    }
    let count = due.len();
    if count > 0 {
        let state = s.clone();
        // One task, cities sequentially: a burst of parallel Google batches would burn the quota
        // and slow every other screen down.
        tokio::spawn(async move {
            for id in due {
                if let Err(e) = run_batch(
                    &state,
                    id,
                    Opts {
                        trigger: "scheduled".into(),
                        ..Opts::default()
                    },
                )
                .await
                {
                    tracing::warn!(directory = %id, error = %e, "scheduled Google refresh failed");
                }
            }
        });
    }
    Ok(Json(
        json!({ "started": count > 0, "directories_due": count }),
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// The run itself
// ─────────────────────────────────────────────────────────────────────────────

async fn run_batch(s: &AppState, directory_id: Uuid, opts: Opts) -> Result<Value, String> {
    let dir: Option<(Uuid, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, name, city, state FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_optional(&s.db)
            .await
            .map_err(|e| format!("could not read the directory: {e}"))?;

    let Some((id, name, city, state)) = dir else {
        return Err("directory not found".to_string());
    };
    let dir = DirectoryRow {
        id,
        name,
        city: city.unwrap_or_default(),
        state: state.unwrap_or_default(),
    };

    // One run per city at a time. A stale row (container restarted mid-run) is not a lock.
    let running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM google_refresh_runs \
         WHERE directory_id = $1 AND finished_at IS NULL \
           AND started_at > NOW() - ($2 || ' minutes')::interval",
    )
    .bind(directory_id)
    .bind(STALE_RUN_MINUTES.to_string())
    .fetch_one(&s.db)
    .await
    .map_err(|e| format!("could not check for a running refresh: {e}"))?;
    if running > 0 {
        return Err(
            "A refresh is already running for this city — wait for it to finish, then try again."
                .to_string(),
        );
    }

    let key = crate::handlers::data_company::get_google_api_key(s, Some(&directory_id.to_string()))
        .await
        .map_err(|e| {
            format!(
                "No Google Places key is configured for this city, so nothing could be refreshed. \
                 Add one in Provider API Keys (or scope one to this city). ({e})"
            )
        })?;

    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO google_refresh_runs \
            (directory_id, trigger, update_arm, discover_arm, batch_updates, batch_adds) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(directory_id)
    .bind(&opts.trigger)
    .bind(opts.update)
    .bind(opts.discover)
    .bind(opts.max_update as i32)
    .bind(opts.max_add as i32)
    .fetch_one(&s.db)
    .await
    .map_err(|e| format!("could not open a refresh run: {e}"))?;

    let mut counts = Counts::default();
    let mut samples: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    if opts.update {
        match update_arm(s, &dir, &key, opts.max_update, &mut counts, &mut samples).await {
            Ok(claimed) => counts.skipped_claimed += claimed,
            Err(e) => notes.push(format!("existing-row arm: {e}")),
        }
    }

    if opts.discover {
        if let Err(e) = discover_arm(
            s,
            &dir,
            &key,
            opts.max_add,
            opts.radius_m,
            &mut counts,
            &mut samples,
        )
        .await
        {
            notes.push(format!("discover arm: {e}"));
        }
    }

    let details = json!({
        "samples": samples.into_iter().take(20).collect::<Vec<_>>(),
        "notes": notes,
    });

    let _ = sqlx::query(
        "UPDATE google_refresh_runs SET added = $2, updated = $3, identified = $4, \
                unchanged = $5, unreachable = $6, skipped_claimed = $7, details = $8, \
                finished_at = NOW() \
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(counts.added as i32)
    .bind(counts.updated as i32)
    .bind(counts.identified as i32)
    .bind(counts.unchanged as i32)
    .bind(counts.unreachable as i32)
    .bind(counts.skipped_claimed as i32)
    .bind(&details)
    .execute(&s.db)
    .await;

    Ok(json!({
        "success": true,
        "run_id": run_id,
        "directory_id": directory_id,
        "directory": dir.name,
        "trigger": opts.trigger,
        "added": counts.added,
        "updated": counts.updated,
        "identified": counts.identified,
        "unchanged": counts.unchanged,
        "unreachable": counts.unreachable,
        "skipped_claimed": counts.skipped_claimed,
        "samples": details["samples"],
        "notes": notes,
    }))
}

/// (A) Update unclaimed listings in place. Returns how many CLAIMED listings were skipped.
async fn update_arm(
    s: &AppState,
    dir: &DirectoryRow,
    key: &str,
    max: i64,
    counts: &mut Counts,
    samples: &mut Vec<Value>,
) -> Result<usize, String> {
    let claimed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM businesses WHERE directory_id = $1 AND COALESCE(claimed, false)",
    )
    .bind(dir.id)
    .fetch_one(&s.db)
    .await
    .map_err(|e| e.to_string())?;

    // Oldest-refreshed first, so repeated runs walk the whole city instead of re-doing page one.
    let rows = sqlx::query(
        "SELECT id, name, address, city, state, zip, phone, website, latitude, longitude, \
                rating, review_count, place_id \
         FROM businesses \
         WHERE directory_id = $1 AND is_active = true AND NOT COALESCE(claimed, false) \
         ORDER BY enriched_at NULLS FIRST, enriched_at ASC, id \
         LIMIT $2",
    )
    .bind(dir.id)
    .bind(max)
    .fetch_all(&s.db)
    .await
    .map_err(|e| format!("could not read the listings: {e}"))?;

    for row in rows {
        let bid: Uuid = row.get("id");
        let bname: String = row.get("name");
        let baddr: String = row.get::<Option<String>, _>("address").unwrap_or_default();
        let bstate: String = row.get::<Option<String>, _>("state").unwrap_or_default();
        let cur_place: Option<String> = row.get("place_id");

        let query = places_query(&bname, &baddr, &row);
        let answer = match lookup(s, key, &query).await {
            Ok(Some(a)) => a,
            Ok(None) => {
                counts.unchanged += 1;
                samples.push(json!({"business_id": bid, "name": bname, "result": "unchanged", "why": "Google found no matching listing"}));
                mark_enriched(&s.db, bid).await;
                continue;
            }
            Err(e) => {
                counts.unreachable += 1;
                samples.push(
                    json!({"business_id": bid, "name": bname, "result": "unreachable", "why": e}),
                );
                continue;
            }
        };

        let cand_name = answer
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let cand_addr = answer
            .get("formatted_address")
            .and_then(Value::as_str)
            .unwrap_or_default();

        // The identity guard: never write a stranger's details onto a listing.
        if !listing_agrees(&bname, &baddr, &bstate, cand_name, cand_addr) {
            counts.unchanged += 1;
            samples.push(json!({"business_id": bid, "name": bname, "result": "unchanged", "why": format!("Google's best match looks like a different business ({cand_name}) — not written")}));
            mark_enriched(&s.db, bid).await;
            continue;
        }

        let new_addr = differs_str(row.get::<Option<String>, _>("address"), Some(cand_addr));
        let new_phone = differs_phone(
            row.get::<Option<String>, _>("phone"),
            answer.get("formatted_phone_number").and_then(Value::as_str),
        );
        let new_site = differs_str(
            row.get::<Option<String>, _>("website"),
            answer.get("website").and_then(Value::as_str),
        );
        let new_lat = differs_f64(
            row.get::<Option<f64>, _>("latitude"),
            answer
                .pointer("/geometry/location/lat")
                .and_then(Value::as_f64),
        );
        let new_lng = differs_f64(
            row.get::<Option<f64>, _>("longitude"),
            answer
                .pointer("/geometry/location/lng")
                .and_then(Value::as_f64),
        );
        let new_rating = differs_f64(
            row.get::<Option<f64>, _>("rating"),
            answer.get("rating").and_then(Value::as_f64),
        );
        let new_reviews = differs_i32(
            row.get::<Option<i32>, _>("review_count"),
            answer
                .get("user_ratings_total")
                .and_then(Value::as_i64)
                .map(|v| v as i32),
        );
        // Identity is stamped only when the row has none, and never when another listing in this
        // directory already holds that place id (that is a duplicate to merge, not to steal).
        let mut new_place: Option<String> = None;
        if cur_place.as_deref().unwrap_or("").trim().is_empty() {
            if let Some(pid) = answer
                .get("place_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|p| !p.is_empty())
            {
                let taken: Option<Uuid> = sqlx::query_scalar(
                    "SELECT id FROM businesses WHERE directory_id = $1 AND place_id = $2 AND id <> $3",
                )
                .bind(dir.id)
                .bind(pid)
                .bind(bid)
                .fetch_optional(&s.db)
                .await
                .map_err(|e| e.to_string())?;
                if taken.is_none() {
                    new_place = Some(pid.to_string());
                }
            }
        }

        let changed = new_addr.is_some()
            || new_phone.is_some()
            || new_site.is_some()
            || new_lat.is_some()
            || new_lng.is_some()
            || new_rating.is_some()
            || new_reviews.is_some();
        let touched = changed || new_place.is_some();

        let res = sqlx::query(
            "UPDATE businesses SET \
                address = COALESCE($2, address), \
                phone = COALESCE($3, phone), \
                website = COALESCE($4, website), \
                latitude = COALESCE($5, latitude), \
                longitude = COALESCE($6, longitude), \
                lat = COALESCE($5, lat), \
                lng = COALESCE($6, lng), \
                rating = COALESCE($7, rating), \
                review_count = COALESCE($8, review_count), \
                place_id = COALESCE(NULLIF(place_id, ''), $9, place_id), \
                enriched_at = NOW(), \
                updated_at = CASE WHEN $10::boolean THEN NOW() ELSE updated_at END \
             WHERE id = $1 AND NOT COALESCE(claimed, false)",
        )
        .bind(bid)
        .bind(&new_addr)
        .bind(&new_phone)
        .bind(&new_site)
        .bind(new_lat)
        .bind(new_lng)
        .bind(new_rating)
        .bind(new_reviews)
        .bind(&new_place)
        .bind(touched)
        .execute(&s.db)
        .await
        .map_err(|e| format!("could not write an update: {e}"))?;

        if res.rows_affected() == 0 {
            // The listing was claimed between our read and our write — leave it alone.
            counts.skipped_claimed += 1;
            continue;
        }

        if changed {
            counts.updated += 1;
            let fields: Vec<&str> = [
                new_addr.is_some().then_some("address"),
                new_phone.is_some().then_some("phone"),
                new_site.is_some().then_some("website"),
                (new_lat.is_some() || new_lng.is_some()).then_some("map location"),
                new_rating.is_some().then_some("rating"),
                new_place.is_some().then_some("Google identity"),
            ]
            .into_iter()
            .flatten()
            .collect();
            samples.push(json!({
                "business_id": bid,
                "name": bname,
                "result": "updated",
                "fields": fields,
            }));
        } else if new_place.is_some() {
            // The listing already matched Google on every visible field; the only change is the
            // durable place id. Report it separately so "updated" always means "the page changed".
            counts.identified += 1;
            samples.push(json!({
                "business_id": bid,
                "name": bname,
                "result": "identified",
                "why": "already matched Google — only the Google identity was stored",
            }));
        } else {
            counts.unchanged += 1;
            samples.push(json!({"business_id": bid, "name": bname, "result": "unchanged", "why": "already matches Google"}));
        }
    }

    Ok(claimed as usize)
}

/// (B) Add businesses Google knows in this city that the directory does not list yet.
async fn discover_arm(
    s: &AppState,
    dir: &DirectoryRow,
    key: &str,
    max_add: i64,
    radius_m: i64,
    counts: &mut Counts,
    samples: &mut Vec<Value>,
) -> Result<(), String> {
    if max_add <= 0 {
        return Ok(());
    }
    if dir.city.trim().is_empty() {
        return Err("this city has no city name set, so there is nothing to search around".into());
    }

    // `directories.city` can be stored as "Palm Bay, FL" while `state` is "FL", so a naive
    // "in {city}, {state}" reads "in Palm Bay, FL, FL". Take the first comma segment of each.
    let city_short = dir.city.split(',').next().unwrap_or("").trim().to_string();
    let state_short = dir.state.split(',').next().unwrap_or("").trim().to_string();
    if city_short.is_empty() {
        return Err("this city has no city name set, so there is nothing to search around".into());
    }

    let centre: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT AVG(latitude), AVG(longitude) FROM businesses \
         WHERE directory_id = $1 AND is_active = true AND latitude IS NOT NULL",
    )
    .bind(dir.id)
    .fetch_optional(&s.db)
    .await
    .map_err(|e| e.to_string())?;
    let Some((Some(lat), Some(lng))) = centre else {
        return Err(
            "this city has no listings with a map location yet, so a radius search has no centre"
                .into(),
        );
    };

    // The city's own postal codes — the test that saves a neighbourhood directory (Lake Nona's
    // businesses are addressed "Orlando, FL 32827"). Built ONLY from rows whose own city is this
    // city, so a long-standing mis-filed listing cannot widen the fence: Palm Bay's directory also
    // holds a handful of Melbourne addresses from the July import, and deriving the zips from all
    // rows would have let Melbourne businesses straight back in.
    let city_zips: Vec<String> = sqlx::query_scalar(
        "SELECT zip FROM ( \
             SELECT trim(zip) AS zip, COUNT(*) AS c FROM businesses \
             WHERE directory_id = $1 AND coalesce(trim(zip), '') <> '' \
               AND lower(replace(trim(coalesce(city, '')), '.', '')) = \
                   lower(replace(trim($2), '.', '')) \
             GROUP BY 1 ORDER BY c DESC LIMIT 6) z",
    )
    .bind(dir.id)
    .bind(&city_short)
    .fetch_all(&s.db)
    .await
    .map_err(|e| e.to_string())?;
    // A directory whose rows carry no city at all (an unpopulated import) falls back to its own
    // zip mix — otherwise discovery could never add anything to it.
    let city_zips: Vec<String> = if city_zips.is_empty() {
        sqlx::query_scalar(
            "SELECT zip FROM ( \
                 SELECT trim(zip) AS zip, COUNT(*) AS c FROM businesses \
                 WHERE directory_id = $1 AND coalesce(trim(zip), '') <> '' \
                 GROUP BY 1 ORDER BY c DESC LIMIT 6) z",
        )
        .bind(dir.id)
        .fetch_all(&s.db)
        .await
        .map_err(|e| e.to_string())?
    } else {
        city_zips
    };

    let cats: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT c.id, c.name FROM directory_categories c \
         LEFT JOIN businesses b ON b.category_id = c.id AND b.directory_id = $1 \
         WHERE c.directory_id IS NULL OR c.directory_id = $1 \
         GROUP BY c.id, c.name ORDER BY COUNT(b.id) DESC, c.name LIMIT $2",
    )
    .bind(dir.id)
    .bind(DISCOVER_CATEGORIES)
    .fetch_all(&s.db)
    .await
    .map_err(|e| e.to_string())?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut errors: Vec<String> = Vec::new();
    let mut skipped_outside = 0usize;

    for (cat_fallback_id, cat) in cats {
        if counts.added as i64 >= max_add {
            break;
        }
        let q = format!("{} in {}, {}", cat, city_short, state_short);
        let found = match search_places(key, &q, lat, lng, radius_m).await {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{cat}: {e}"));
                continue;
            }
        };
        for cand in found {
            if counts.added as i64 >= max_add {
                break;
            }
            let cname = cand
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if cname.is_empty() {
                continue;
            }
            let caddr = cand
                .get("address")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let pid = cand
                .get("place_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from);

            // In-batch dedupe (Google returns the same shop for two neighbouring categories).
            let batch_key = format!(
                "{}|{}",
                pid.clone().unwrap_or_default().to_lowercase(),
                format!("{cname}|{caddr}").to_lowercase()
            );
            if !seen.insert(batch_key) {
                continue;
            }

            // ── ONE CITY PER RECORD (W22's rule, applied to adds) ───────────────────────────
            // Google's radius is a BIAS, not a fence: searching around Apopka happily returned
            // spas in Orlando, Altamonte Springs and Mt Dora, and stamping the directory's own
            // city on them re-created the exact defect W22 fixed (a Melbourne business filed
            // under Palm Bay). A candidate is only filed here when its OWN address names this
            // city; everything else belongs to the directory that names it. The city/state
            // stamped on the record are the business's own, never the directory's.
            let (cand_city, cand_state) = address_locality(&caddr);
            let cand_zip = zip_of(&caddr);
            let in_city = same_city(&cand_city, &city_short)
                || (!cand_zip.is_empty() && city_zips.contains(&cand_zip));
            if !in_city {
                skipped_outside += 1;
                continue;
            }

            if crate::handlers::businesses::find_business_by_identity(
                &s.db,
                dir.id,
                pid.as_deref(),
                &cname,
                &caddr,
            )
            .await
            .map_err(|e| e.to_string())?
            .is_some()
            {
                continue; // already listed — not an add, and not a duplicate either
            }

            let types: Vec<String> = cand
                .get("types")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();

            let slug = crate::handlers::businesses::unique_business_slug(
                &s.db,
                dir.id,
                &crate::handlers::discovery_queue::slugify(&cname),
            )
            .await
            .map_err(|e| e.to_string())?;

            let (cat_id, cat_name) = crate::handlers::discovery_queue::map_category(
                s,
                dir.id,
                &types,
                Some(cat_fallback_id),
                Some(cat.clone()),
            )
            .await;

            let clat = cand.get("lat").and_then(Value::as_f64);
            let clng = cand.get("lng").and_then(Value::as_f64);
            let rating = cand.get("rating").and_then(Value::as_f64);
            let reviews = cand
                .get("review_count")
                .and_then(Value::as_i64)
                .map(|v| v as i32);
            let franchise = crate::utils::franchise::is_likely_franchise(&cname, &types);

            let inserted: Result<Option<Uuid>, sqlx::Error> = sqlx::query_scalar(
                "INSERT INTO businesses \
                    (directory_id, name, slug, category_id, address, city, state, zip, phone, \
                     website, latitude, longitude, lat, lng, rating, review_count, business_type, \
                     is_franchise, place_id, claimed, is_active, enriched_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$11,$12,$13,$14,'local',$15,$16,\
                         false,true,NOW()) \
                 ON CONFLICT DO NOTHING RETURNING id",
            )
            .bind(dir.id)
            .bind(&cname)
            .bind(&slug)
            .bind(cat_id)
            .bind(&caddr)
            .bind(if cand_city.is_empty() {
                None
            } else {
                Some(cand_city.clone())
            })
            .bind(if cand_state.is_empty() {
                None
            } else {
                Some(cand_state.clone())
            })
            .bind(if cand_zip.is_empty() {
                None
            } else {
                Some(cand_zip.clone())
            })
            .bind(cand.get("phone").and_then(Value::as_str))
            .bind(cand.get("website").and_then(Value::as_str))
            .bind(clat)
            .bind(clng)
            .bind(rating)
            .bind(reviews)
            .bind(franchise)
            .bind(&pid)
            .fetch_optional(&s.db)
            .await;

            match inserted {
                Ok(Some(new_id)) => {
                    counts.added += 1;
                    samples.push(json!({
                        "business_id": new_id,
                        "name": cname,
                        "result": "added",
                        "address": caddr,
                        "category": cat_name,
                        "place_id": pid,
                    }));
                }
                Ok(None) => {}
                Err(e) => errors.push(format!("{cname}: {e}")),
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    if skipped_outside > 0 {
        samples.push(json!({
            "result": "skipped_outside",
            "count": skipped_outside,
            "why": "Google returned businesses whose own address is another city — not filed here",
        }));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Google lookups
// ─────────────────────────────────────────────────────────────────────────────

/// One business's Google answer: the merged legacy/New field set, or a refusal we can quote.
async fn lookup(s: &AppState, key: &str, query: &str) -> Result<Option<Value>, String> {
    match find_place_then_details(LEGACY_PLACES_BASE, key, query).await {
        Ok(PlacesOutcome::Found(l)) => Ok(Some(l.merged())),
        Ok(PlacesOutcome::NoMatch { .. }) => Ok(None),
        Err(legacy_err) => {
            // A legacy refusal is not proof that no key works — the same install can hold a key
            // restricted to Places API (New) (measured 2026-11-12). Try those before reporting.
            let mut last = format!(
                "{} refused ({})",
                legacy_err.call, legacy_err.provider_status
            );
            for (provider, k, _active, _label) in
                crate::handlers::satellite::google_key_candidates(&s.db).await
            {
                if !provider.starts_with("google") {
                    continue;
                }
                match find_place_then_details_v1(&k, query).await {
                    Ok(PlacesOutcome::Found(l)) => return Ok(Some(l.merged())),
                    Ok(PlacesOutcome::NoMatch { .. }) => return Ok(None),
                    Err(e) => last = format!("{} refused ({})", e.call, e.provider_status),
                }
            }
            Err(format!("Google could not be asked: {last}"))
        }
    }
}

/// Search results for discovery, normalized to `[{name,address,place_id,lat,lng,rating,
/// review_count,types,phone,website}]`. Legacy Text Search first (it honours a radius), then
/// Places API (New) when the legacy endpoint refuses the key.
async fn search_places(
    key: &str,
    query: &str,
    lat: f64,
    lng: f64,
    radius_m: i64,
) -> Result<Vec<Value>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("could not start the request: {e}"))?;

    let legacy = format!(
        "{LEGACY_PLACES_BASE}/textsearch/json?query={}&location={lat},{lng}&radius={radius_m}&key={key}",
        pct(query)
    );
    let resp = client
        .get(&legacy)
        .send()
        .await
        .map_err(|e| format!("Google could not be reached: {e}"))?;
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let status = body.get("status").and_then(Value::as_str).unwrap_or("");
    if status == "OK" || status == "ZERO_RESULTS" {
        let mut out = Vec::new();
        for r in body
            .get("results")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            out.push(json!({
                "name": r.get("name").and_then(Value::as_str).unwrap_or_default(),
                "address": r.get("formatted_address").and_then(Value::as_str).unwrap_or_default(),
                "place_id": r.get("place_id").and_then(Value::as_str).unwrap_or_default(),
                "lat": r.pointer("/geometry/location/lat").and_then(Value::as_f64),
                "lng": r.pointer("/geometry/location/lng").and_then(Value::as_f64),
                "rating": r.get("rating").and_then(Value::as_f64),
                "review_count": r.get("user_ratings_total").and_then(Value::as_i64),
                "types": r.get("types").cloned().unwrap_or_else(|| json!([])),
            }));
        }
        if !out.is_empty() || status == "OK" {
            return Ok(out);
        }
    }

    // Places API (New)
    let body_req = json!({
        "textQuery": query,
        "maxResultCount": 20,
        "locationBias": { "circle": {
            "center": { "latitude": lat, "longitude": lng },
            "radius": radius_m as f64
        }},
    });
    let resp = client
        .post("https://places.googleapis.com/v1/places:searchText")
        .header("X-Goog-Api-Key", key)
        .header(
            "X-Goog-FieldMask",
            "places.id,places.displayName,places.formattedAddress,places.location,\
             places.rating,places.userRatingCount,places.types,places.nationalPhoneNumber,\
             places.websiteUri",
        )
        .json(&body_req)
        .send()
        .await
        .map_err(|e| format!("Google could not be reached: {e}"))?;
    let http = resp.status().as_u16();
    let parsed: Value = resp.json().await.unwrap_or(Value::Null);
    if !(200..300).contains(&http) {
        let msg = parsed
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Google refused the search");
        return Err(format!("Google refused the search ({http}): {msg}"));
    }
    let mut out = Vec::new();
    for p in parsed
        .get("places")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        out.push(json!({
            "name": p.pointer("/displayName/text").and_then(Value::as_str).unwrap_or_default(),
            "address": p.get("formattedAddress").and_then(Value::as_str).unwrap_or_default(),
            "place_id": p.get("id").and_then(Value::as_str).unwrap_or_default(),
            "lat": p.pointer("/location/latitude").and_then(Value::as_f64),
            "lng": p.pointer("/location/longitude").and_then(Value::as_f64),
            "rating": p.get("rating").and_then(Value::as_f64),
            "review_count": p.get("userRatingCount").and_then(Value::as_i64),
            "types": p.get("types").cloned().unwrap_or_else(|| json!([])),
            "phone": p.get("nationalPhoneNumber").and_then(Value::as_str),
            "website": p.get("websiteUri").and_then(Value::as_str),
        }));
    }
    Ok(out)
}

/// `"<name>, <address>"`, falling back to city/state/zip when the row has no address — the same
/// rule enrichment uses (`build_places_query`), restated here because the row is a raw sqlx row.
fn places_query(name: &str, address: &str, row: &sqlx::postgres::PgRow) -> String {
    let addr = address
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_end_matches(", USA")
        .trim_end_matches(", United States")
        .trim()
        .to_string();
    let mut parts = vec![name.trim().to_string()];
    if !addr.is_empty() {
        parts.push(addr);
    } else {
        for col in ["city", "state", "zip"] {
            if let Ok(Some(v)) = row.try_get::<Option<String>, _>(col) {
                if !v.trim().is_empty() {
                    parts.push(v.trim().to_string());
                }
            }
        }
    }
    parts.join(", ")
}

/// Percent-encode a value for a URL query string.
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Small helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn mark_enriched(db: &sqlx::PgPool, business_id: Uuid) {
    let _ = sqlx::query("UPDATE businesses SET enriched_at = NOW() WHERE id = $1")
        .bind(business_id)
        .execute(db)
        .await;
}

fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The city and state of a Google formatted address: `"508 N Mills Ave, Orlando, FL 32803, USA"`
/// ⇒ `("Orlando", "FL")`. An unparseable address yields empty strings, which the caller treats as
/// "cannot prove where this business is" and therefore does not add.
fn address_locality(address: &str) -> (String, String) {
    let parts: Vec<&str> = address.split(',').map(str::trim).collect();
    let mut end = parts.len();
    if end > 0
        && matches!(
            parts[end - 1].to_ascii_lowercase().as_str(),
            "usa" | "us" | "united states" | "united states of america"
        )
    {
        end -= 1;
    }
    if end >= 2 {
        let city = parts[end - 2].to_string();
        let state = parts[end - 1]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        (city, state)
    } else {
        (String::new(), String::new())
    }
}

/// Same city, allowing the spelling differences Google and a human use ("St. Cloud" vs
/// "Saint Cloud", "St Petersburg" vs "St. Petersburg").
fn same_city(a: &str, b: &str) -> bool {
    fn key(s: &str) -> String {
        let lower = s.to_lowercase().replace('.', " ");
        let squeezed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
        squeezed.replace("saint ", "st ").trim().to_string()
    }
    let (ka, kb) = (key(a), key(b));
    !ka.is_empty() && ka == kb
}

/// The 5-digit ZIP in a Google formatted address (`"…, Orlando, FL 32827, USA"` ⇒ `"32827"`).
fn zip_of(address: &str) -> String {
    let parts: Vec<&str> = address.split(',').map(str::trim).collect();
    let tail = parts
        .iter()
        .rev()
        .find(|p| {
            let l = p.to_ascii_lowercase();
            !matches!(
                l.as_str(),
                "usa" | "us" | "united states" | "united states of america"
            )
        })
        .copied()
        .unwrap_or("");
    // "FL 32827" / "FL 32827-1234" — take the first 5-digit run.
    let digits: String = tail
        .split_whitespace()
        .find(|w| w.chars().filter(char::is_ascii_digit).count() >= 5)
        .map(|w| w.chars().filter(char::is_ascii_digit).take(5).collect())
        .unwrap_or_default();
    digits
}

fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// A new value only when Google's answer is non-empty AND actually differs.
fn differs_str(current: Option<String>, google: Option<&str>) -> Option<String> {
    let g = google.map(str::trim).filter(|v| !v.is_empty())?;
    match current.as_deref().map(str::trim) {
        Some(c) if !c.is_empty() && normalize(c) == normalize(g) => None,
        _ => Some(g.to_string()),
    }
}

fn differs_phone(current: Option<String>, google: Option<&str>) -> Option<String> {
    let g = google.map(str::trim).filter(|v| !v.is_empty())?;
    let gd = digits(g);
    if gd.len() < 7 {
        return None;
    }
    match current.as_deref() {
        Some(c) if !c.trim().is_empty() => {
            let cd = digits(c);
            if cd == gd
                || (cd.len() >= 10 && gd.len() >= 10 && cd[cd.len() - 10..] == gd[gd.len() - 10..])
            {
                None
            } else {
                Some(g.to_string())
            }
        }
        _ => Some(g.to_string()),
    }
}

fn differs_f64(current: Option<f64>, google: Option<f64>) -> Option<f64> {
    let g = google.filter(|v| v.is_finite() && *v != 0.0)?;
    match current {
        Some(c) if (c - g).abs() < 1e-6 => None,
        _ => Some(g),
    }
}

fn differs_i32(current: Option<i32>, google: Option<i32>) -> Option<i32> {
    let g = google.filter(|v| *v >= 0)?;
    match current {
        Some(c) if c == g => None,
        _ => Some(g),
    }
}

fn normalize_schedule(raw: &str) -> Option<String> {
    match raw.trim().to_lowercase().as_str() {
        "manual" => Some("manual".into()),
        "daily" => Some("daily".into()),
        "weekly" => Some("weekly".into()),
        "monthly" => Some("monthly".into()),
        _ => None,
    }
}

async fn schedule_of(s: &AppState, directory_id: Uuid) -> Result<String, AppError> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT zaarhub_config->'google_refresh'->>'schedule' FROM directories WHERE id = $1",
    )
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?
    .flatten();
    Ok(normalize_schedule(raw.as_deref().unwrap_or("weekly")).unwrap_or_else(|| "weekly".into()))
}

/// Is a city's scheduled refresh due? `manual` is never due; anything else counts from the last
/// FINISHED run (never from the last attempt, so a failing key cannot re-run every hour).
async fn is_due(s: &AppState, directory_id: Uuid, schedule: &str) -> Result<bool, sqlx::Error> {
    let days = match normalize_schedule(schedule).as_deref() {
        Some("daily") => 1,
        Some("weekly") => 7,
        Some("monthly") => 30,
        _ => return Ok(false),
    };
    let last: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT MAX(finished_at) FROM google_refresh_runs WHERE directory_id = $1",
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;
    match last {
        None => Ok(true),
        Some(t) => Ok((chrono::Utc::now() - t).num_days() >= days),
    }
}
