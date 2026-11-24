//! Ad-fulfillment writer — fan a booking's positions out to CoreSwift's `ad_fulfillment`
//! custom object, one row per position, replay-safe.
//!
//! Wire contract (parent card t_5b6ec9d2, live on CoreSwift):
//!   * `ad_fulfillment` carries `unique_together = ["booking_ref", "position"]`, so the
//!     upsert door (`POST /api/objects/ad_fulfillment/records?upsert=booking_ref,position`)
//!     makes a re-run update the SAME row instead of inserting a second one.
//!   * `DELETE /api/objects/ad_fulfillment/records/{id}` soft-archives (frees the key).
//!   * The tenant is resolved from the credential (`csk_…` personal key), never from the body.
//!
//! The credential lives in `CORESWIFT_AD_FULFILLMENT_KEY` (0600 env, never the repo), not in the
//! per-directory DB link: the object is the ZAARHUB workspace's and is shared across every city,
//! so the writer holds ONE fixed key.

use serde_json::{json, Value};

const OBJECT_SLUG: &str = "ad_fulfillment";

/// The `csk_…` personal key that authenticates the `ad_fulfillment` object. `None` means the
/// credential is not configured — callers skip the write (and log) rather than send an empty key,
/// because CoreSwift fails closed on a blank credential.
pub fn fulfillment_key() -> Option<String> {
    std::env::var("CORESWIFT_AD_FULFILLMENT_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
}

/// Base URL for the CoreSwift objects API — the same resolution every other CoreSwift call uses.
pub fn base_url() -> String {
    crate::coreswift::coreswift_url()
}

lazy_static::lazy_static! {
    static ref HTTP: reqwest::Client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("ad_fulfillment reqwest client");
}

/// The `city` enum options declared on the live `ad_fulfillment` object. Kept as ONE list so a
/// value outside it can never be written: a caller that cannot map a directory's city to an enum
/// value fails loudly instead of pushing a value the object would reject (or worse, a free-text
/// guess).
const CITY_ENUM: [&str; 10] = [
    "Apopka",
    "Boca Raton",
    "Hollywood",
    "Lake Nona",
    "Palm Bay",
    "Palm Coast",
    "Pompano Beach",
    "St Cloud",
    "St Petersburg",
    "Winter Garden",
];

/// Map a directory's `city` column to the object's `city` enum value.
///
/// `directories.city` stores the human form ("St. Cloud, FL", "Palm Bay, FL"). The object's enum
/// stores the bare name WITHOUT the period ("St Cloud", "St Petersburg"). Normalisation therefore
/// drops the `, ST` suffix, strips periods and matches case-insensitively. `None` = the city is
/// NOT one of the ten the object accepts — the caller must fail loudly and write nothing.
pub fn normalize_city(raw: &str) -> Option<String> {
    let base = raw.split(',').next().unwrap_or(raw).trim();
    let bare = base.replace('.', "").trim().to_lowercase();
    CITY_ENUM
        .iter()
        .find(|c| c.to_lowercase() == bare)
        .map(|c| (*c).to_string())
}

/// One fulfillment row. `booking_ref` + `position` is the object's composite key.
#[derive(Debug, Clone)]
pub struct FulfillmentRow {
    pub booking_ref: String,
    pub position: String,
    pub city: String,
    pub issue_date: String,
    pub buyer_email: String,
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

/// POST /api/objects/ad_fulfillment/records?upsert=booking_ref,position
///
/// Idempotent: same key values → same row, supplied fields merged. Returns the record id and
/// whether the write INSERTED (true) or updated an existing row (false, i.e. a replay).
pub async fn upsert(key: &str, base: &str, row: &FulfillmentRow) -> Result<(String, bool), String> {
    let url = format!("{base}/api/objects/{OBJECT_SLUG}/records?upsert=booking_ref,position");
    let body = json!({
        "booking_ref": row.booking_ref,
        "position": row.position,
        "city": row.city,
        "issue_date": row.issue_date,
        "buyer_email": row.buyer_email,
        "artwork_status": "Requested",
        "ran": false,
    });

    let resp = HTTP
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ad_fulfillment upsert failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "ad_fulfillment upsert {status}: {}",
            truncate(&text, 300)
        ));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("ad_fulfillment upsert bad JSON: {e}"))?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "ad_fulfillment upsert returned no id: {}",
                truncate(&text, 300)
            )
        })?;
    let inserted = v
        .get("upsert")
        .and_then(|u| u.get("inserted"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    Ok((id, inserted))
}

/// List LIVE records matching a booking (and optionally one position). The object's list door
/// returns only non-archived rows, so this is exactly the set that still needs archiving.
async fn list(
    key: &str,
    base: &str,
    booking_ref: &str,
    position: Option<&str>,
) -> Result<Vec<Value>, String> {
    let url = format!("{base}/api/objects/{OBJECT_SLUG}/records");
    let mut query: Vec<(&str, &str)> = vec![("filter.booking_ref", booking_ref)];
    if let Some(p) = position {
        query.push(("filter.position", p));
    }
    let resp = HTTP
        .get(&url)
        .bearer_auth(key)
        .query(&query)
        .send()
        .await
        .map_err(|e| format!("ad_fulfillment list failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "ad_fulfillment list {status}: {}",
            truncate(&text, 300)
        ));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("ad_fulfillment list bad JSON: {e}"))?;
    Ok(v.get("records")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default())
}

async fn soft_delete(key: &str, base: &str, id: &str) -> Result<(), String> {
    let url = format!("{base}/api/objects/{OBJECT_SLUG}/records/{id}");
    let resp = HTTP
        .delete(&url)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| format!("ad_fulfillment archive failed: {e}"))?;
    let status = resp.status();
    // A row already archived answers 404 on a second archive — that is the expected idempotent
    // outcome, not an error. Anything else that is not success is a real failure.
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let text = resp.text().await.unwrap_or_default();
    Err(format!(
        "ad_fulfillment archive {status}: {}",
        truncate(&text, 300)
    ))
}

/// Soft-archive every LIVE fulfillment row for a booking (and optionally a single position).
/// Idempotent: rows already archived are skipped. Returns the number of rows archived.
pub async fn archive(
    key: &str,
    base: &str,
    booking_ref: &str,
    position: Option<&str>,
) -> Result<usize, String> {
    let records = list(key, base, booking_ref, position).await?;
    let mut archived = 0usize;
    for rec in records {
        if let Some(id) = rec.get("id").and_then(|x| x.as_str()) {
            soft_delete(key, base, id).await?;
            archived += 1;
        }
    }
    Ok(archived)
}
