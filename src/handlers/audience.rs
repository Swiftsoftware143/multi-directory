//! Per-city audience lists, campaign sequences and image media — kanban t_63ffc2de.
//!
//! Three gaps were closed here:
//!   A. the Sponsors list had no membership push at all (see `coreswift::push_sponsor_business`);
//!      this module exposes the explicit "move to Sponsors" transition for a city, which creates or
//!      reactivates the sponsored listing AND fires that push.
//!   B. one per-city surface over the city's three built-in lists (Subscribers / Business Listings /
//!      Sponsors) with counts and a city filter — the NETWORK ADMIN = ONE SURFACE rule: everything
//!      belonging to a network's cities is managed in one place, city shown per row.
//!   C. campaign SEQUENCES (steps with delay + subject/body + image) — MD owns the definition and the
//!      audience, CoreSwift owns the SEND (David's binding MAIL decision). There is no SMTP sender
//!      here and there must never be one.
//!   D. a generic image endpoint (base64) returning a STABLE url that templates, blog posts and
//!      campaign emails can all point at, plus the public route that actually serves it.
//!
//! Every route reads and verifies the bearer token inside the handler (`claims_from_headers`) rather
//! than trusting an `Extension<Claims>` — a route's position in the router does not guarantee the
//! extension was injected, and a missing one panics into a 500 for every caller.
//!
//! All SQL is a COMPLETE compile-time literal at every call site (gate rule 5d); every value is bound.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::monetization::spawn_sponsor_push;
use crate::handlers::tenant_scope::{
    assert_directory_admin, assert_directory_admin_by_slug, caller_tenant, claims_from_headers,
    is_platform_operator,
};
use crate::AppState;

/// Where uploaded image bytes live by default. Bind-mounted into the container at the SAME path
/// (`docker inspect multidirectory`: /opt/swift/www/zaarhub.com/uploads -> itself), and served back
/// by `serve_media` through the public `/uploads/*` route — so the URL a buyer copies out of the
/// panel actually loads. Writing an asset we cannot serve would be a dead capability.
///
/// MEASURED 2026-10-03: the container runs as uid 999 while this mount is root-owned, so the write
/// failed with EACCES and every upload 500d. The mount is now owned by the container user, and
/// `MD_UPLOADS_ROOT` lets a deployment point the store anywhere writable.
const DEFAULT_UPLOADS_ROOT: &str = "/opt/swift/www/zaarhub.com/uploads";

fn uploads_root() -> String {
    std::env::var("MD_UPLOADS_ROOT")
        .map(|v| v.trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_UPLOADS_ROOT.to_string())
}

/// Refuse anything bigger than this. A base64 JPEG out of a phone camera is ~4-6 MB.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

const LIST_SUBSCRIBERS: &str = "subscribers";
const LIST_BUSINESSES: &str = "businesses";
const LIST_SPONSORS: &str = "sponsors";

fn claims_of(s: &AppState, headers: &HeaderMap) -> Result<Claims, AppError> {
    claims_from_headers(headers, &s.config.jwt_secret)
}

/// Network-level guard (NETWORK ADMIN = ONE SURFACE): the platform operator, or a tenant admin of
/// any city in the network. 404 on failure so a network's existence is not disclosed.
async fn assert_network_admin(
    db: &PgPool,
    claims: &Claims,
    network_id: Uuid,
) -> Result<(), AppError> {
    if is_platform_operator(claims) {
        return Ok(());
    }
    let tid = caller_tenant(claims)?;
    let ok = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM directories d JOIN users u ON u.id = d.owner_id \
         WHERE d.network_id = $1 AND u.tenant_id = $2)",
    )
    .bind(network_id)
    .bind(tid)
    .fetch_one(db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::NotFound("network not found".into()))
    }
}

/// Is this a list name we serve? Anything else is a 404, never a silent empty list.
fn parse_list(name: &str) -> Result<&'static str, AppError> {
    match name {
        LIST_SUBSCRIBERS => Ok(LIST_SUBSCRIBERS),
        LIST_BUSINESSES => Ok(LIST_BUSINESSES),
        LIST_SPONSORS => Ok(LIST_SPONSORS),
        other => Err(AppError::NotFound(format!(
            "no such list '{other}' (expected subscribers, businesses or sponsors)"
        ))),
    }
}

/// A query param that must be a uuid when present. An unparseable value is a 400, NEVER silently
/// dropped: a dropped filter turns "this city's sequences" into "EVERY city's sequences" for an
/// operator, and a 403 for a city admin. (Found in live smoke: the panel used to send the slug.)
fn parse_uuid_param(params: &HashMap<String, String>, key: &str) -> Result<Option<Uuid>, AppError> {
    match params.get(key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => Uuid::parse_str(v)
            .map(Some)
            .map_err(|_| AppError::Validation(format!("{key} must be a uuid (got '{v}')"))),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// B. The per-city list surface
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/admin/audience/overview?network_id=<uuid>
/// Every city the caller may administer, each with the counts of its three built-in lists.
/// The network's cities come back in ONE response (one surface), city shown per row.
pub async fn list_overview(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let network_id = parse_uuid_param(&params, "network_id")?;

    if let Some(nid) = network_id {
        assert_network_admin(&s.db, &claims, nid).await?;
    }

    // Two COMPLETE literals: the operator sees every city, a tenant admin only their own. The
    // network filter is bound in both, and list ids fall back to the parent network's when the city
    // has none of its own (a city inherits its network's CoreSwift lists, never anything above it).
    const OVERVIEW_ALL: &str = "SELECT d.id AS directory_id, d.slug, d.name, d.city, d.state, \
            d.network_id, n.name AS network_name, \
            COALESCE(d.coreswift_list_id_claimed, n.coreswift_list_id_claimed) AS list_claimed, \
            COALESCE(d.coreswift_list_id_newsletter, n.coreswift_list_id_newsletter) AS list_newsletter, \
            COALESCE(d.coreswift_list_id_sponsors, n.coreswift_list_id_sponsors) AS list_sponsors, \
            (SELECT COUNT(*) FROM newsletter_subscribers s \
              WHERE s.directory_id = d.id AND s.status = 'active') AS subscribers, \
            (SELECT COUNT(*) FROM newsletter_subscribers s \
              WHERE s.directory_id = d.id AND s.status = 'unsubscribed') AS subscribers_unsubscribed, \
            (SELECT COUNT(*) FROM claimed_businesses cb JOIN businesses b ON b.id = cb.business_id \
              WHERE b.directory_id = d.id AND COALESCE(cb.is_active, true)) AS claimed_businesses, \
            (SELECT COUNT(*) FROM sponsored_listings sl \
              WHERE sl.directory_id = d.id AND sl.is_active) AS sponsors, \
            (SELECT COUNT(*) FROM sponsored_listings sl \
              WHERE sl.directory_id = d.id) AS sponsors_total, \
            (SELECT COUNT(*) FROM businesses b WHERE b.directory_id = d.id) AS businesses_total \
        FROM directories d LEFT JOIN networks n ON n.id = d.network_id \
        WHERE ($1::uuid IS NULL OR d.network_id = $1::uuid) \
        ORDER BY n.name NULLS FIRST, d.name";

    const OVERVIEW_MINE: &str = "SELECT d.id AS directory_id, d.slug, d.name, d.city, d.state, \
            d.network_id, n.name AS network_name, \
            COALESCE(d.coreswift_list_id_claimed, n.coreswift_list_id_claimed) AS list_claimed, \
            COALESCE(d.coreswift_list_id_newsletter, n.coreswift_list_id_newsletter) AS list_newsletter, \
            COALESCE(d.coreswift_list_id_sponsors, n.coreswift_list_id_sponsors) AS list_sponsors, \
            (SELECT COUNT(*) FROM newsletter_subscribers s \
              WHERE s.directory_id = d.id AND s.status = 'active') AS subscribers, \
            (SELECT COUNT(*) FROM newsletter_subscribers s \
              WHERE s.directory_id = d.id AND s.status = 'unsubscribed') AS subscribers_unsubscribed, \
            (SELECT COUNT(*) FROM claimed_businesses cb JOIN businesses b ON b.id = cb.business_id \
              WHERE b.directory_id = d.id AND COALESCE(cb.is_active, true)) AS claimed_businesses, \
            (SELECT COUNT(*) FROM sponsored_listings sl \
              WHERE sl.directory_id = d.id AND sl.is_active) AS sponsors, \
            (SELECT COUNT(*) FROM sponsored_listings sl \
              WHERE sl.directory_id = d.id) AS sponsors_total, \
            (SELECT COUNT(*) FROM businesses b WHERE b.directory_id = d.id) AS businesses_total \
        FROM directories d \
        LEFT JOIN networks n ON n.id = d.network_id \
        JOIN users u ON u.id = d.owner_id \
        WHERE u.tenant_id = $2 \
          AND ($1::uuid IS NULL OR d.network_id = $1::uuid) \
        ORDER BY n.name NULLS FIRST, d.name";

    let rows = if is_platform_operator(&claims) {
        sqlx::query(OVERVIEW_ALL)
            .bind(network_id)
            .fetch_all(&s.db)
            .await?
    } else {
        let tid = caller_tenant(&claims)?;
        sqlx::query(OVERVIEW_MINE)
            .bind(network_id)
            .bind(tid)
            .fetch_all(&s.db)
            .await?
    };

    let mut cities: Vec<Value> = Vec::with_capacity(rows.len());
    let mut t_sub = 0i64;
    let mut t_claim = 0i64;
    let mut t_spon = 0i64;
    for r in &rows {
        let subscribers: i64 = r.try_get("subscribers").unwrap_or(0);
        let claimed: i64 = r.try_get("claimed_businesses").unwrap_or(0);
        let sponsors: i64 = r.try_get("sponsors").unwrap_or(0);
        t_sub += subscribers;
        t_claim += claimed;
        t_spon += sponsors;
        cities.push(json!({
            "directory_id": r.try_get::<Uuid, _>("directory_id").ok(),
            "slug": r.try_get::<String, _>("slug").ok(),
            "name": r.try_get::<String, _>("name").ok(),
            "city": r.try_get::<Option<String>, _>("city").ok().flatten(),
            "state": r.try_get::<Option<String>, _>("state").ok().flatten(),
            "network_id": r.try_get::<Option<Uuid>, _>("network_id").ok().flatten(),
            "network_name": r.try_get::<Option<String>, _>("network_name").ok().flatten(),
            "counts": {
                "subscribers": subscribers,
                "subscribers_unsubscribed": r.try_get::<i64, _>("subscribers_unsubscribed").unwrap_or(0),
                "claimed_businesses": claimed,
                "businesses_total": r.try_get::<i64, _>("businesses_total").unwrap_or(0),
                "sponsors": sponsors,
                "sponsors_total": r.try_get::<i64, _>("sponsors_total").unwrap_or(0),
            },
            "coreswift_lists": {
                "claimed": r.try_get::<Option<Uuid>, _>("list_claimed").ok().flatten(),
                "newsletter": r.try_get::<Option<Uuid>, _>("list_newsletter").ok().flatten(),
                "sponsors": r.try_get::<Option<Uuid>, _>("list_sponsors").ok().flatten(),
            },
        }));
    }

    Ok(Json(json!({
        "network_id": network_id,
        "cities": cities,
        "totals": {
            "cities": cities.len(),
            "subscribers": t_sub,
            "claimed_businesses": t_claim,
            "sponsors": t_spon,
        },
        "lists": [LIST_SUBSCRIBERS, LIST_BUSINESSES, LIST_SPONSORS],
    })))
}

/// GET /api/v1/admin/audience/:slug/:list?status=&per_page=&page=
/// The members of one of the city's three lists. `subscribers` is PII and directory-admin only.
pub async fn list_members(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((slug, list)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let directory_id = assert_directory_admin_by_slug(&s.db, &claims, &slug).await?;
    let list = parse_list(&list)?;

    let page: i64 = params
        .get("page")
        .and_then(|p| p.parse().ok())
        .unwrap_or(1)
        .max(1);
    let per_page: i64 = params
        .get("per_page")
        .and_then(|p| p.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let offset = (page - 1) * per_page;

    let (members, total): (Vec<Value>, i64) = match list {
        LIST_SUBSCRIBERS => {
            let status = params.get("status").map(|v| v.as_str()).unwrap_or("active");
            let rows = sqlx::query(
                "SELECT id, email, name, status, subscribed_at, unsubscribed_at \
                 FROM newsletter_subscribers WHERE directory_id = $1 AND status = $2 \
                 ORDER BY subscribed_at DESC LIMIT $3 OFFSET $4",
            )
            .bind(directory_id)
            .bind(status)
            .bind(per_page)
            .bind(offset)
            .fetch_all(&s.db)
            .await?;
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM newsletter_subscribers WHERE directory_id = $1 AND status = $2",
            )
            .bind(directory_id)
            .bind(status)
            .fetch_one(&s.db)
            .await?;
            let out = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.try_get::<Uuid, _>("id").ok(),
                        "email": r.try_get::<String, _>("email").ok(),
                        "name": r.try_get::<Option<String>, _>("name").ok().flatten(),
                        "status": r.try_get::<Option<String>, _>("status").ok().flatten(),
                        "subscribed_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("subscribed_at").ok().flatten(),
                        "unsubscribed_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("unsubscribed_at").ok().flatten(),
                    })
                })
                .collect();
            (out, total)
        }
        LIST_BUSINESSES => {
            let rows = sqlx::query(
                "SELECT cb.id AS claim_id, b.id AS business_id, b.name, b.email, b.city, \
                        cb.owner_email, cb.owner_name, cb.verified_at, COALESCE(cb.is_active, true) AS is_active \
                 FROM claimed_businesses cb JOIN businesses b ON b.id = cb.business_id \
                 WHERE b.directory_id = $1 \
                 ORDER BY b.name LIMIT $2 OFFSET $3",
            )
            .bind(directory_id)
            .bind(per_page)
            .bind(offset)
            .fetch_all(&s.db)
            .await?;
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM claimed_businesses cb JOIN businesses b ON b.id = cb.business_id \
                 WHERE b.directory_id = $1",
            )
            .bind(directory_id)
            .fetch_one(&s.db)
            .await?;
            let out = rows
                .iter()
                .map(|r| {
                    json!({
                        "claim_id": r.try_get::<Uuid, _>("claim_id").ok(),
                        "business_id": r.try_get::<Uuid, _>("business_id").ok(),
                        "name": r.try_get::<String, _>("name").ok(),
                        "email": r.try_get::<Option<String>, _>("email").ok().flatten(),
                        "city": r.try_get::<Option<String>, _>("city").ok().flatten(),
                        "owner_email": r.try_get::<String, _>("owner_email").ok(),
                        "owner_name": r.try_get::<Option<String>, _>("owner_name").ok().flatten(),
                        "verified_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("verified_at").ok().flatten(),
                        "is_active": r.try_get::<bool, _>("is_active").ok(),
                    })
                })
                .collect();
            (out, total)
        }
        LIST_SPONSORS => {
            let active_only = params
                .get("active")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let rows = sqlx::query(
                "SELECT sl.id AS listing_id, b.id AS business_id, b.name, b.email, b.city, \
                        sl.is_active, sl.slot_position, sl.start_date, sl.end_date, sl.crm_pushed_at \
                 FROM sponsored_listings sl JOIN businesses b ON b.id = sl.business_id \
                 WHERE sl.directory_id = $1 AND (NOT $2 OR sl.is_active) \
                 ORDER BY sl.is_active DESC NULLS LAST, sl.slot_position, b.name \
                 LIMIT $3 OFFSET $4",
            )
            .bind(directory_id)
            .bind(active_only)
            .bind(per_page)
            .bind(offset)
            .fetch_all(&s.db)
            .await?;
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sponsored_listings sl \
                 WHERE sl.directory_id = $1 AND (NOT $2 OR sl.is_active)",
            )
            .bind(directory_id)
            .bind(active_only)
            .fetch_one(&s.db)
            .await?;
            let out = rows
                .iter()
                .map(|r| {
                    json!({
                        "listing_id": r.try_get::<Uuid, _>("listing_id").ok(),
                        "business_id": r.try_get::<Uuid, _>("business_id").ok(),
                        "name": r.try_get::<String, _>("name").ok(),
                        "email": r.try_get::<Option<String>, _>("email").ok().flatten(),
                        "city": r.try_get::<Option<String>, _>("city").ok().flatten(),
                        "is_active": r.try_get::<Option<bool>, _>("is_active").ok().flatten(),
                        "slot_position": r.try_get::<Option<i32>, _>("slot_position").ok().flatten(),
                        "start_date": r.try_get::<Option<chrono::NaiveDate>, _>("start_date").ok().flatten(),
                        "end_date": r.try_get::<Option<chrono::NaiveDate>, _>("end_date").ok().flatten(),
                        "crm_pushed_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("crm_pushed_at").ok().flatten(),
                    })
                })
                .collect();
            (out, total)
        }
        _ => unreachable!("parse_list returned an unknown list"),
    };

    Ok(Json(json!({
        "directory_id": directory_id,
        "slug": slug,
        "list": list,
        "members": members,
        "total": total,
        "page": page,
        "per_page": per_page,
    })))
}

/// GET /api/v1/admin/audience/:slug/sponsors — the Sponsors members of one city.
///
/// A separate handler because `/admin/audience/:slug/sponsors` is ALSO the POST target of the
/// "move to Sponsors" transition: the static segment wins over `:list`, so without this a GET there
/// came back 405 instead of the list. Both methods now live on the same path.
pub async fn list_sponsor_members(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    list_members(
        State(s),
        headers,
        Path((slug, LIST_SPONSORS.to_string())),
        Query(params),
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct MoveToSponsorsRequest {
    pub business_id: Uuid,
    pub slot_position: Option<i32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub price_paid: Option<f64>,
    pub currency: Option<String>,
    pub badge_text: Option<String>,
}

/// POST /api/v1/admin/audience/:slug/sponsors — the explicit "move to Sponsors" transition.
/// Creates the sponsored listing if there is none, otherwise (re)activates the newest one, and fires
/// the CoreSwift Sponsors-list push. This is the admin-operable half of the sponsor nurture loop.
pub async fn move_to_sponsors(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(body): Json<MoveToSponsorsRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let directory_id = assert_directory_admin_by_slug(&s.db, &claims, &slug).await?;

    // The business must belong to THIS city — never sponsor another city's business by id.
    let in_city = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM businesses WHERE id = $1 AND directory_id = $2)",
    )
    .bind(body.business_id)
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;
    if !in_city {
        return Err(AppError::NotFound("business not found in this city".into()));
    }

    let start = parse_date(body.start_date.as_deref())?;
    let end = parse_date(body.end_date.as_deref())?;
    let price = body
        .price_paid
        .map(|v| rust_decimal::Decimal::try_from(v).unwrap_or_default());
    let slot = body.slot_position.unwrap_or(1);

    // No UNIQUE(directory_id, business_id) on the table: find the newest row for the pair and
    // reactivate it, else insert. Both statements are complete literals.
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM sponsored_listings WHERE directory_id = $1 AND business_id = $2 \
         ORDER BY is_active DESC NULLS LAST, created_at DESC LIMIT 1",
    )
    .bind(directory_id)
    .bind(body.business_id)
    .fetch_optional(&s.db)
    .await?;

    const REACTIVATE: &str = "UPDATE sponsored_listings SET \
            is_active = true, slot_position = $1, \
            start_date = COALESCE($2::date, start_date), \
            end_date = COALESCE($3::date, end_date), \
            price_paid = COALESCE($4::numeric, price_paid), \
            currency = COALESCE($5::text, currency), \
            badge_text = COALESCE($6::text, badge_text), \
            crm_pushed_at = NULL, updated_at = NOW() \
         WHERE id = $7 \
         RETURNING id, business_id";

    const INSERT: &str = "INSERT INTO sponsored_listings \
            (directory_id, business_id, slot_position, start_date, end_date, is_active, \
             price_paid, currency, badge_text, metadata) \
         VALUES ($1, $2, $3, COALESCE($4::date, CURRENT_DATE), \
                 COALESCE($5::date, CURRENT_DATE + 30), true, $6, $7, $8, '{}'::jsonb) \
         RETURNING id, business_id";

    let row = match existing {
        Some(listing_id) => {
            sqlx::query(REACTIVATE)
                .bind(slot)
                .bind(start)
                .bind(end)
                .bind(price)
                .bind(body.currency.as_deref())
                .bind(body.badge_text.as_deref())
                .bind(listing_id)
                .fetch_one(&s.db)
                .await?
        }
        None => {
            sqlx::query(INSERT)
                .bind(directory_id)
                .bind(body.business_id)
                .bind(slot)
                .bind(start)
                .bind(end)
                .bind(price)
                .bind(body.currency.as_deref())
                .bind(body.badge_text.as_deref())
                .fetch_one(&s.db)
                .await?
        }
    };

    let listing_id: Uuid = row.try_get("id")?;
    let business_id: Uuid = row.try_get("business_id")?;

    // Fire-and-forget: the CRM push must not gate the admin action (same shape as loyalty enroll).
    spawn_sponsor_push(s.db.clone(), listing_id, business_id);

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "listing_id": listing_id,
            "business_id": business_id,
            "reactivated": existing.is_some(),
            "push": "queued",
            "message": "Moved to Sponsors — the CoreSwift Sponsors list push is queued.",
        })),
    ))
}

/// DELETE /api/v1/admin/audience/:slug/sponsors/:listing_id — remove from Sponsors.
/// Deactivates rather than deletes: the sponsorship history and its crm_pushed_at stay auditable.
pub async fn remove_sponsor(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((slug, listing_id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let directory_id = assert_directory_admin_by_slug(&s.db, &claims, &slug).await?;

    let res = sqlx::query(
        "UPDATE sponsored_listings SET is_active = false, updated_at = NOW() \
         WHERE id = $1 AND directory_id = $2",
    )
    .bind(listing_id)
    .bind(directory_id)
    .execute(&s.db)
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("sponsored listing not found".into()));
    }
    Ok(Json(
        json!({ "listing_id": listing_id, "is_active": false }),
    ))
}

fn parse_date(raw: Option<&str>) -> Result<Option<chrono::NaiveDate>, AppError> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| AppError::Validation(format!("invalid date '{v}' (expected YYYY-MM-DD)"))),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// C. Campaign sequences — MD owns the definition and the audience
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSequenceRequest {
    pub name: String,
    pub description: Option<String>,
    pub directory_id: Option<Uuid>,
    pub network_id: Option<Uuid>,
    pub target_list: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSequenceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target_list: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct StepRequest {
    pub step_order: Option<i32>,
    pub delay_days: Option<i32>,
    pub subject: Option<String>,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub template_id: Option<Uuid>,
    pub image_url: Option<String>,
}

/// A sequence's scope guard: a network-owned sequence needs the network guard, a city-owned one the
/// city guard. Nothing lives above a network.
async fn assert_sequence_scope(
    db: &PgPool,
    claims: &Claims,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<(), AppError> {
    match (directory_id, network_id) {
        (Some(d), _) => assert_directory_admin(db, claims, d).await,
        (None, Some(n)) => assert_network_admin(db, claims, n).await,
        (None, None) => Err(AppError::Validation(
            "a sequence belongs to a city (directory_id) or a network (network_id)".into(),
        )),
    }
}

fn validate_target_list(target: Option<&str>) -> Result<String, AppError> {
    let t = target
        .unwrap_or(LIST_SUBSCRIBERS)
        .trim()
        .to_ascii_lowercase();
    match t.as_str() {
        LIST_SUBSCRIBERS | "claimed" | LIST_SPONSORS => Ok(t),
        other => Err(AppError::Validation(format!(
            "target_list must be subscribers, claimed or sponsors (got '{other}')"
        ))),
    }
}

async fn load_steps(
    db: &PgPool,
    sequence_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Value>>, AppError> {
    let mut out: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if sequence_ids.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(
        "SELECT id, sequence_id, step_order, delay_days, subject, body_html, body_text, \
                template_id, image_url, updated_at \
         FROM campaign_sequence_steps WHERE sequence_id = ANY($1) \
         ORDER BY step_order, id",
    )
    .bind(sequence_ids)
    .fetch_all(db)
    .await?;
    for r in &rows {
        let sid: Uuid = r.try_get("sequence_id")?;
        out.entry(sid).or_default().push(step_json(r)?);
    }
    Ok(out)
}

fn step_json(r: &sqlx::postgres::PgRow) -> Result<Value, AppError> {
    Ok(json!({
        "id": r.try_get::<Uuid, _>("id")?,
        "sequence_id": r.try_get::<Uuid, _>("sequence_id")?,
        "step_order": r.try_get::<Option<i32>, _>("step_order")?.unwrap_or(1),
        "delay_days": r.try_get::<Option<i32>, _>("delay_days")?.unwrap_or(0),
        "subject": r.try_get::<Option<String>, _>("subject")?.unwrap_or_default(),
        "body_html": r.try_get::<Option<String>, _>("body_html")?,
        "body_text": r.try_get::<Option<String>, _>("body_text")?,
        "template_id": r.try_get::<Option<Uuid>, _>("template_id")?,
        "image_url": r.try_get::<Option<String>, _>("image_url")?,
    }))
}

/// Compile-time column list for `campaign_sequences`. `concat!` needs LITERALS, not consts, so the
/// same bytes are published as a macro (gate rule 5d: the statement is assembled at compile time).
/// MUST stay byte-identical to the SELECT/RETURNING lists below.
macro_rules! seq_cols {
    () => {
        "id, name, description, directory_id, network_id, target_list, is_active, created_at, updated_at"
    };
}

/// GET /api/v1/admin/campaign-sequences?directory_id=&network_id=
/// With a city: that city's sequences AND its network's. With a network: the network's plus every
/// city in it (one surface). With neither (operator): everything.
pub async fn list_sequences(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let directory_id = parse_uuid_param(&params, "directory_id")?;
    let network_id = parse_uuid_param(&params, "network_id")?;

    const SEQ_BY_DIRECTORY: &str = "SELECT id, name, description, directory_id, network_id, target_list, \
            is_active, created_at, updated_at FROM campaign_sequences \
         WHERE directory_id = $1 OR network_id = (SELECT network_id FROM directories WHERE id = $1) \
         ORDER BY name";
    const SEQ_BY_NETWORK: &str = "SELECT id, name, description, directory_id, network_id, target_list, \
            is_active, created_at, updated_at FROM campaign_sequences \
         WHERE network_id = $1 OR directory_id IN (SELECT id FROM directories WHERE network_id = $1) \
         ORDER BY directory_id NULLS FIRST, name";
    const SEQ_ALL: &str = "SELECT id, name, description, directory_id, network_id, target_list, \
            is_active, created_at, updated_at FROM campaign_sequences ORDER BY name";

    let rows = if let Some(did) = directory_id {
        assert_directory_admin(&s.db, &claims, did).await?;
        sqlx::query(SEQ_BY_DIRECTORY)
            .bind(did)
            .fetch_all(&s.db)
            .await?
    } else if let Some(nid) = network_id {
        assert_network_admin(&s.db, &claims, nid).await?;
        sqlx::query(SEQ_BY_NETWORK)
            .bind(nid)
            .fetch_all(&s.db)
            .await?
    } else {
        if !is_platform_operator(&claims) {
            return Err(AppError::Forbidden(
                "pass directory_id or network_id".into(),
            ));
        }
        sqlx::query(SEQ_ALL).fetch_all(&s.db).await?
    };

    let ids: Vec<Uuid> = rows.iter().filter_map(|r| r.try_get("id").ok()).collect();
    let mut steps = load_steps(&s.db, &ids).await?;

    let sequences: Vec<Value> = rows
        .iter()
        .map(|r| {
            let id: Uuid = r.try_get("id")?;
            Ok(json!({
                "id": id,
                "name": r.try_get::<String, _>("name")?,
                "description": r.try_get::<Option<String>, _>("description")?,
                "directory_id": r.try_get::<Option<Uuid>, _>("directory_id")?,
                "network_id": r.try_get::<Option<Uuid>, _>("network_id")?,
                "target_list": r.try_get::<String, _>("target_list")?,
                "is_active": r.try_get::<bool, _>("is_active")?,
                "created_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at")?,
                "updated_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("updated_at")?,
                "steps": steps.remove(&id).unwrap_or_default(),
            }))
        })
        .collect::<Result<Vec<Value>, AppError>>()?;

    Ok(Json(json!({
        "sequences": sequences,
        "sending": "CoreSwift",
        "note": "Multi-Directory defines a sequence and its audience; CoreSwift performs the send.",
    })))
}

pub async fn create_sequence(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateSequenceRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("name is required".into()));
    }
    assert_sequence_scope(&s.db, &claims, body.directory_id, body.network_id).await?;
    let target_list = validate_target_list(body.target_list.as_deref())?;

    let row = sqlx::query(concat!(
        "INSERT INTO campaign_sequences (name, description, directory_id, network_id, target_list, is_active) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING ",
        seq_cols!(),
        ""
    ))
    .bind(&name)
    .bind(body.description.as_deref())
    .bind(body.directory_id)
    .bind(body.network_id)
    .bind(&target_list)
    .bind(body.is_active.unwrap_or(true))
    .fetch_one(&s.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": row.try_get::<Uuid, _>("id")?,
            "name": row.try_get::<String, _>("name")?,
            "target_list": row.try_get::<String, _>("target_list")?,
            "directory_id": row.try_get::<Option<Uuid>, _>("directory_id")?,
            "network_id": row.try_get::<Option<Uuid>, _>("network_id")?,
            "is_active": row.try_get::<bool, _>("is_active")?,
            "steps": [],
        })),
    ))
}

/// Load one sequence and assert the caller may administer its scope.
async fn sequence_guard(
    s: &AppState,
    claims: &Claims,
    id: Uuid,
) -> Result<sqlx::postgres::PgRow, AppError> {
    let row = sqlx::query(
        "SELECT id, name, description, directory_id, network_id, target_list, is_active, \
                created_at, updated_at FROM campaign_sequences WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("sequence not found".into()))?;

    let directory_id: Option<Uuid> = row.try_get("directory_id")?;
    let network_id: Option<Uuid> = row.try_get("network_id")?;
    assert_sequence_scope(&s.db, claims, directory_id, network_id).await?;
    Ok(row)
}

pub async fn get_sequence(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let row = sequence_guard(&s, &claims, id).await?;
    let mut steps = load_steps(&s.db, &[id]).await?;
    Ok(Json(json!({
        "id": id,
        "name": row.try_get::<String, _>("name")?,
        "description": row.try_get::<Option<String>, _>("description")?,
        "directory_id": row.try_get::<Option<Uuid>, _>("directory_id")?,
        "network_id": row.try_get::<Option<Uuid>, _>("network_id")?,
        "target_list": row.try_get::<String, _>("target_list")?,
        "is_active": row.try_get::<bool, _>("is_active")?,
        "steps": steps.remove(&id).unwrap_or_default(),
    })))
}

pub async fn update_sequence(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSequenceRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let row = sequence_guard(&s, &claims, id).await?;

    let target = match body.target_list.as_deref() {
        Some(t) => Some(validate_target_list(Some(t))?),
        None => None,
    };

    let updated = sqlx::query(concat!(
        "UPDATE campaign_sequences SET \
            name = COALESCE($1, name), \
            description = COALESCE($2, description), \
            target_list = COALESCE($3, target_list), \
            is_active = COALESCE($4, is_active), \
            updated_at = NOW() \
         WHERE id = $5 RETURNING ",
        seq_cols!(),
        ""
    ))
    .bind(body.name.as_deref())
    .bind(body.description.as_deref())
    .bind(target)
    .bind(body.is_active)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "id": updated.try_get::<Uuid, _>("id")?,
        "name": updated.try_get::<String, _>("name")?,
        "description": updated.try_get::<Option<String>, _>("description")?,
        "directory_id": updated.try_get::<Option<Uuid>, _>("directory_id")?,
        "network_id": updated.try_get::<Option<Uuid>, _>("network_id")?,
        "target_list": updated.try_get::<String, _>("target_list")?,
        "is_active": updated.try_get::<bool, _>("is_active")?,
        "previous_name": row.try_get::<String, _>("name")?,
    })))
}

pub async fn delete_sequence(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    sequence_guard(&s, &claims, id).await?;
    let res = sqlx::query("DELETE FROM campaign_sequences WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("sequence not found".into()));
    }
    Ok(Json(json!({ "deleted": true, "id": id })))
}

pub async fn create_step(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<StepRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    sequence_guard(&s, &claims, id).await?;

    // Default the order to "after the last step" so the panel never has to compute it.
    let next_order: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(step_order), 0) + 1 FROM campaign_sequence_steps WHERE sequence_id = $1",
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(1);

    let row = sqlx::query(
        "INSERT INTO campaign_sequence_steps \
            (sequence_id, step_order, delay_days, subject, body_html, body_text, template_id, image_url) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         RETURNING id, sequence_id, step_order, delay_days, subject, body_html, body_text, template_id, image_url",
    )
    .bind(id)
    .bind(body.step_order.unwrap_or(next_order))
    .bind(body.delay_days.unwrap_or(0))
    .bind(body.subject.clone().unwrap_or_default())
    .bind(body.body_html.as_deref())
    .bind(body.body_text.as_deref())
    .bind(body.template_id)
    .bind(body.image_url.as_deref())
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(step_json(&row)?)))
}

pub async fn update_step(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((id, step_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<StepRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    sequence_guard(&s, &claims, id).await?;

    let row = sqlx::query(
        "UPDATE campaign_sequence_steps SET \
            step_order = COALESCE($1, step_order), \
            delay_days = COALESCE($2, delay_days), \
            subject = COALESCE($3, subject), \
            body_html = COALESCE($4, body_html), \
            body_text = COALESCE($5, body_text), \
            template_id = COALESCE($6, template_id), \
            image_url = COALESCE($7, image_url), \
            updated_at = NOW() \
         WHERE id = $8 AND sequence_id = $9 \
         RETURNING id, sequence_id, step_order, delay_days, subject, body_html, body_text, template_id, image_url",
    )
    .bind(body.step_order)
    .bind(body.delay_days)
    .bind(body.subject.as_deref())
    .bind(body.body_html.as_deref())
    .bind(body.body_text.as_deref())
    .bind(body.template_id)
    .bind(body.image_url.as_deref())
    .bind(step_id)
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("step not found".into()))?;

    Ok(Json(step_json(&row)?))
}

pub async fn delete_step(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((id, step_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    sequence_guard(&s, &claims, id).await?;

    let res = sqlx::query("DELETE FROM campaign_sequence_steps WHERE id = $1 AND sequence_id = $2")
        .bind(step_id)
        .bind(id)
        .execute(&s.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("step not found".into()));
    }
    Ok(Json(json!({ "deleted": true, "id": step_id })))
}

// ─────────────────────────────────────────────────────────────────────────────
// D. Generic image upload + the public route that serves it
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MediaUploadRequest {
    pub directory_id: Uuid,
    pub filename: Option<String>,
    /// Raw base64, or a full `data:image/png;base64,...` URL — both are accepted.
    pub content_base64: String,
}

fn safe_ext(declared: Option<&str>, data_url_mime: Option<&str>) -> Result<String, AppError> {
    let from_name = declared
        .map(|f| f.rsplit('.').next().unwrap_or("").to_ascii_lowercase())
        .filter(|e| !e.is_empty());
    let from_mime = data_url_mime.and_then(|m| m.rsplit('/').next()).map(|m| {
        let m = m.to_ascii_lowercase();
        if m == "jpeg" {
            "jpg".to_string()
        } else {
            m
        }
    });
    let ext = from_name.or(from_mime).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "webp" | "gif" | "svg" | "ico" => Ok(ext),
        other => Err(AppError::Validation(format!(
            "unsupported image type '{other}' (png, jpg, webp, gif, svg, ico)"
        ))),
    }
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// POST /api/v1/admin/media/upload  { directory_id, filename, content_base64 }
/// Returns a STABLE, publicly fetchable url usable from templates, blog posts and campaign emails.
pub async fn upload_media(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MediaUploadRequest>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    assert_directory_admin(&s.db, &claims, body.directory_id).await?;
    let uploaded_by = Uuid::parse_str(&claims.sub).ok();

    let raw = body.content_base64.trim();
    let (data_url_mime, payload) = match raw.strip_prefix("data:") {
        Some(rest) => match rest.split_once(',') {
            Some((meta, payload)) => (meta.split(';').next().map(|m| m.to_string()), payload),
            None => {
                return Err(AppError::Validation(
                    "malformed data URL (expected data:<mime>;base64,<payload>)".into(),
                ))
            }
        },
        None => (None, raw),
    };

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| AppError::Validation(format!("content_base64 is not valid base64: {e}")))?;

    if bytes.is_empty() {
        return Err(AppError::Validation("the uploaded image is empty".into()));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::Validation(format!(
            "image is {} bytes — the limit is {MAX_IMAGE_BYTES}",
            bytes.len()
        )));
    }

    let ext = safe_ext(body.filename.as_deref(), data_url_mime.as_deref())?;
    let mime = mime_for_ext(&ext);

    let root = uploads_root();
    let rel_dir = format!("media/{}", body.directory_id);
    let abs_dir = format!("{root}/{rel_dir}");
    tokio::fs::create_dir_all(&abs_dir)
        .await
        .map_err(|e| AppError::Internal(format!("could not create the upload directory: {e}")))?;

    let stored = format!("{}.{}", Uuid::new_v4(), ext);
    let abs_path = format!("{abs_dir}/{stored}");
    tokio::fs::write(&abs_path, &bytes)
        .await
        .map_err(|e| AppError::Internal(format!("could not write the image: {e}")))?;

    let url = format!("/uploads/{rel_dir}/{stored}");
    let display_name = body
        .filename
        .as_deref()
        .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(f).to_string())
        .unwrap_or_else(|| format!("image.{ext}"));

    let row = sqlx::query(
        "INSERT INTO media_assets (directory_id, filename, url, mime_type, byte_size, uploaded_by) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, created_at",
    )
    .bind(body.directory_id)
    .bind(&display_name)
    .bind(&url)
    .bind(mime)
    .bind(bytes.len() as i64)
    .bind(uploaded_by)
    .fetch_one(&s.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": row.try_get::<Uuid, _>("id")?,
            "url": url,
            "filename": display_name,
            "mime_type": mime,
            "byte_size": bytes.len(),
            "directory_id": body.directory_id,
            "created_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at")?,
        })),
    ))
}

/// GET /api/v1/admin/media?directory_id=<uuid> — the media library for one city.
pub async fn list_media(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;
    let directory_id: Uuid = parse_uuid_param(&params, "directory_id")?
        .ok_or_else(|| AppError::Validation("directory_id is required".into()))?;
    assert_directory_admin(&s.db, &claims, directory_id).await?;

    let rows = sqlx::query(
        "SELECT id, filename, url, mime_type, byte_size, created_at FROM media_assets \
         WHERE directory_id = $1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(directory_id)
    .fetch_all(&s.db)
    .await?;

    let assets: Vec<Value> = rows
        .iter()
        .map(|r| {
            Ok(json!({
                "id": r.try_get::<Uuid, _>("id")?,
                "filename": r.try_get::<String, _>("filename")?,
                "url": r.try_get::<String, _>("url")?,
                "mime_type": r.try_get::<Option<String>, _>("mime_type")?,
                "byte_size": r.try_get::<i64, _>("byte_size")?,
                "created_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("created_at")?,
            }))
        })
        .collect::<Result<Vec<Value>, AppError>>()?;

    Ok(Json(
        json!({ "directory_id": directory_id, "assets": assets }),
    ))
}

pub async fn delete_media(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let claims = claims_of(&s, &headers)?;

    let row = sqlx::query("SELECT directory_id, url FROM media_assets WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;

    let directory_id: Option<Uuid> = row.try_get("directory_id")?;
    match directory_id {
        Some(d) => assert_directory_admin(&s.db, &claims, d).await?,
        None => {
            if !is_platform_operator(&claims) {
                return Err(AppError::NotFound("asset not found".into()));
            }
        }
    }

    let url: String = row.try_get("url")?;
    sqlx::query("DELETE FROM media_assets WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    // Best-effort unlink: a missing file must not fail the delete.
    if let Some(rel) = url.strip_prefix("/uploads/") {
        let _ = tokio::fs::remove_file(format!("{}/{rel}", uploads_root())).await;
    }

    Ok(Json(json!({ "deleted": true, "id": id })))
}

/// GET /uploads/*path — serve an uploaded file out of the uploads root.
///
/// This is what makes the url an upload returns actually WORK. Before it existed, `/uploads/*` fell
/// through to the SPA fallback and every stored asset url was dead. Only paths that resolve INSIDE
/// the uploads root are served — `..` cannot escape it.
pub async fn serve_media(Path(path): Path<String>) -> Result<Response, AppError> {
    let not_found = || AppError::NotFound("file not found".into());

    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel.contains("..") || rel.contains('\0') {
        return Err(not_found());
    }

    let root = tokio::fs::canonicalize(uploads_root())
        .await
        .map_err(|_| not_found())?;
    let candidate = tokio::fs::canonicalize(root.join(rel))
        .await
        .map_err(|_| not_found())?;
    if !candidate.starts_with(&root) {
        return Err(not_found());
    }

    let bytes = tokio::fs::read(&candidate).await.map_err(|_| not_found())?;
    let ext = candidate
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_for_ext(&ext))
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("could not build the response: {e}")))?)
}
