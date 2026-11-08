//! Walk Score / Bike Score — cached per listing, refreshed quarterly.
//!
//! Rules this module enforces:
//!
//! * **Never on a page view.** Only [`refresh_due`] (the hourly scheduler or the `walk-scores`
//!   CLI) calls the vendor. The public detail endpoint merely reads `business_walk_scores`.
//! * **One credential resolver.** The key comes from
//!   [`resolve_provider_key_scoped`](crate::handlers::provider_keys_handler::resolve_provider_key_scoped),
//!   i.e. directory -> its network -> platform. A city never falls back to another city's key.
//! * **No fabricated numbers.** A transport error, a non-`status: 1` payload, or a missing
//!   `walkscore` stores no score at all. The page renders nothing rather than a placeholder.
//! * **Neutral output only.** We keep the integers. The vendor's `description` verdict,
//!   logo/link and branding are deliberately discarded.
//!
//! Bike Score is NESTED in the response (`bike.score`), not a top-level field. One request with
//! `bike=1` returns both numbers, so one listing costs one API call per quarter.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use crate::handlers::provider_keys_handler::resolve_provider_key_scoped;

/// Provider id in `available_providers` / `provider_keys`.
pub const PROVIDER: &str = "walkscore";

const API_URL: &str = "https://api.walkscore.com/score";

/// Scores change rarely; a quarter is the refresh cadence the owner asked for.
const REFRESH_AFTER_DAYS: i32 = 90;

/// Listings per hourly batch. 3988 listings / 200 an hour completes a full pass in about a day
/// without hammering the vendor's API.
const DEFAULT_BATCH: i64 = 200;

/// Politeness delay between vendor calls inside one batch.
const CALL_SPACING: Duration = Duration::from_millis(120);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The two numbers we keep. Either may be absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scores {
    pub walk: Option<i32>,
    pub bike: Option<i32>,
}

impl Scores {
    fn is_empty(&self) -> bool {
        self.walk.is_none() && self.bike.is_none()
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Candidate {
    business_id: Uuid,
    directory_id: Uuid,
    lat: f64,
    lon: f64,
}

/// Outcome of one refresh pass, for the CLI and the log line.
#[derive(Debug, Default, Clone, Copy)]
pub struct RefreshReport {
    pub attempted: u32,
    pub stored: u32,
    pub failed: u32,
    /// Listings whose directory has no usable key. Skipped silently, never guessed.
    pub unconfigured: u32,
}

/// Start the hourly scheduler. Each tick refreshes ONE bounded batch of listings whose last
/// attempt is older than a quarter — mirroring `reminders::start_reminder_cron`.
pub fn start_walk_score_cron(db: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            match refresh_due(&db, DEFAULT_BATCH).await {
                Ok(r) if r.attempted > 0 => tracing::info!(
                    "[walk_scores] attempted={} stored={} failed={} unconfigured={}",
                    r.attempted,
                    r.stored,
                    r.failed,
                    r.unconfigured
                ),
                Ok(_) => {}
                Err(e) => tracing::error!("[walk_scores] refresh pass failed: {e}"),
            }
        }
    });
}

/// Refresh up to `limit` listings that are due. Uses ONE request per listing (`bike=1` returns
/// walk and the nested bike object together).
pub async fn refresh_due(db: &PgPool, limit: i64) -> Result<RefreshReport, sqlx::Error> {
    let candidates = sqlx::query_as::<_, Candidate>(
        r#"
        SELECT b.id            AS business_id,
               b.directory_id  AS directory_id,
               COALESCE(b.latitude, b.lat)   AS lat,
               COALESCE(b.longitude, b.lng)  AS lon
        FROM businesses b
        LEFT JOIN business_walk_scores w ON w.business_id = b.id
        WHERE b.is_active = true
          AND COALESCE(b.latitude, b.lat) IS NOT NULL
          AND COALESCE(b.longitude, b.lng) IS NOT NULL
          AND (w.last_attempt_at IS NULL
               OR w.last_attempt_at < now() - make_interval(days => $1))
        ORDER BY w.last_attempt_at ASC NULLS FIRST, b.id
        LIMIT $2
        "#,
    )
    .bind(REFRESH_AFTER_DAYS)
    .bind(limit)
    .fetch_all(db)
    .await?;

    let mut report = RefreshReport::default();
    if candidates.is_empty() {
        return Ok(report);
    }

    let client = match reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[walk_scores] could not build HTTP client: {e}");
            return Ok(report);
        }
    };

    // Resolve each directory's key ONCE per pass (the integration decrypts through pgcrypto).
    let mut keys: HashMap<Uuid, Option<String>> = HashMap::new();

    for c in &candidates {
        if let std::collections::hash_map::Entry::Vacant(e) = keys.entry(c.directory_id) {
            e.insert(resolve_provider_key_scoped(db, c.directory_id, PROVIDER).await);
        }
        let Some(key) = keys.get(&c.directory_id).and_then(|k| k.as_deref()) else {
            // No key for this directory: nothing is fetched and nothing is written.
            report.unconfigured += 1;
            continue;
        };

        report.attempted += 1;
        let scores = fetch_scores(&client, key, c.lat, c.lon).await;
        match scores {
            Some(s) if !s.is_empty() => {
                if let Err(e) = store_scores(db, c, &s).await {
                    tracing::error!(
                        "[walk_scores] could not store scores for {}: {e}",
                        c.business_id
                    );
                    report.failed += 1;
                } else {
                    report.stored += 1;
                }
            }
            _ => {
                // Store NO scores. Mark the attempt so the next try is next quarter, and keep
                // whatever earlier valid numbers this listing already had.
                if let Err(e) = mark_attempt(db, c).await {
                    tracing::error!(
                        "[walk_scores] could not record failed attempt for {}: {e}",
                        c.business_id
                    );
                }
                report.failed += 1;
            }
        }

        tokio::time::sleep(CALL_SPACING).await;
    }

    Ok(report)
}

/// One API call. `bike=1` makes the response carry the nested `bike` object as well.
async fn fetch_scores(client: &reqwest::Client, key: &str, lat: f64, lon: f64) -> Option<Scores> {
    let mut url = reqwest::Url::parse(API_URL).ok()?;
    url.query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("lat", &format!("{lat:.6}"))
        .append_pair("lon", &format!("{lon:.6}"))
        .append_pair("wsapikey", key)
        .append_pair("bike", "1");

    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        tracing::warn!("[walk_scores] vendor returned HTTP {}", resp.status());
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    parse_scores(&body)
}

/// Parse the vendor payload. `status != 1` is a hard failure (`None`), never a zero.
fn parse_scores(body: &Value) -> Option<Scores> {
    let status = body.get("status").and_then(Value::as_i64);
    if status != Some(1) {
        return None;
    }

    let walk = body
        .get("walkscore")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| (0..=100).contains(n));

    // Bike Score is NESTED: { "bike": { "score": 43, "description": "…" } }.
    let bike = body
        .get("bike")
        .and_then(|b| b.get("score"))
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| (0..=100).contains(n));

    let scores = Scores { walk, bike };
    if scores.is_empty() {
        None
    } else {
        Some(scores)
    }
}

async fn store_scores(db: &PgPool, c: &Candidate, s: &Scores) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO business_walk_scores
            (business_id, directory_id, walk_score, bike_score, last_attempt_at, refreshed_at)
        VALUES ($1, $2, $3, $4, now(), now())
        ON CONFLICT (business_id) DO UPDATE
           SET directory_id    = EXCLUDED.directory_id,
               walk_score      = EXCLUDED.walk_score,
               bike_score      = EXCLUDED.bike_score,
               last_attempt_at = now(),
               refreshed_at    = now()
        "#,
    )
    .bind(c.business_id)
    .bind(c.directory_id)
    .bind(s.walk)
    .bind(s.bike)
    .execute(db)
    .await
    .map(|_| ())
}

/// Record that we tried, without touching previously valid scores.
async fn mark_attempt(db: &PgPool, c: &Candidate) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO business_walk_scores
            (business_id, directory_id, walk_score, bike_score, last_attempt_at, refreshed_at)
        VALUES ($1, $2, NULL, NULL, now(), NULL)
        ON CONFLICT (business_id) DO UPDATE SET last_attempt_at = now()
        "#,
    )
    .bind(c.business_id)
    .bind(c.directory_id)
    .execute(db)
    .await
    .map(|_| ())
}

/// What the public detail endpoint reads: the cached pair for one business. `None`-only rows
/// (failed attempts) and never-refreshed listings both come back as `(None, None)`.
pub async fn cached_for_business(
    db: &PgPool,
    business_id: Uuid,
) -> Result<(Option<i32>, Option<i32>), sqlx::Error> {
    sqlx::query_as::<_, (Option<i32>, Option<i32>)>(
        "SELECT walk_score, bike_score FROM business_walk_scores WHERE business_id = $1",
    )
    .bind(business_id)
    .fetch_optional(db)
    .await
    .map(|row| row.unwrap_or((None, None)))
}

/// Human-readable age for logs/CLI (`refreshed_at` may be NULL after a failed attempt).
#[allow(dead_code)]
pub fn describe_refresh(ts: Option<DateTime<Utc>>) -> String {
    match ts {
        Some(t) => t.to_rfc3339(),
        None => "never".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_walk_and_nested_bike() {
        let body = json!({
            "status": 1,
            "walkscore": 47,
            "description": "Car-Dependent",
            "bike": { "score": 43, "description": "Somewhat Bikeable" }
        });
        assert_eq!(
            parse_scores(&body),
            Some(Scores {
                walk: Some(47),
                bike: Some(43)
            })
        );
    }

    #[test]
    fn bike_is_not_read_from_the_top_level() {
        let body = json!({ "status": 1, "walkscore": 47, "bike_score": 43 });
        assert_eq!(
            parse_scores(&body),
            Some(Scores {
                walk: Some(47),
                bike: None
            })
        );
    }

    #[test]
    fn non_one_status_stores_nothing() {
        let body = json!({ "status": 2, "walkscore": 47 });
        assert_eq!(parse_scores(&body), None);
    }

    #[test]
    fn payload_without_scores_is_not_a_zero() {
        let body = json!({ "status": 1 });
        assert_eq!(parse_scores(&body), None);
    }
}
