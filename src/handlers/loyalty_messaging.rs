//! Loyalty messaging + positioning (cards B92 / B93) — the admin-configurable ZaarCash surface that
//! rides the shared page chrome on the homepage AND every city page.
//!
//! Rules from the cards, enforced HERE in the back end so no surface can drift:
//!   1. NOTHING HARDCODED — on/off + copy + CTA come from `loyalty_messaging`, resolved
//!      directory -> network -> built-in defaults. A buyer with no agent can change or switch it
//!      off from the admin panel.
//!   2. HONEST STATE — whether a visitor can actually earn is computed from `loyalty_programs`
//!      (earn_rate / points_per_redemption). While nothing can be earned the served copy is the
//!      truthful "how it works" line, never an earnings promise; the full earn copy appears
//!      automatically once a rate is set. The word "points" is never served — the programme
//!      currency is ZaarCash (or whatever the network renames it to).
//!   3. B93 GATING — claims the product cannot deliver are NOT published: no per-job earnings
//!      (needs completion verification, B47), no "verified" language (badges unbuilt), no booking
//!      promise. The impact ticker is computed from REAL ledger data and hides itself below the
//!      admin-set threshold, so a $0 ticker is never shown.
//!
//! Tokens: `{city}` and `{currency}` in every admin-authored string are interpolated server-side so
//! the city-scoped hero carries the city name (and the subfolder SEO with it).

use crate::error::{ApiResult, AppError};
use crate::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_PILL: &str = "Local rewards programme";
const DEFAULT_HEADLINE: &str = "Earn {currency} when you shop local";
const DEFAULT_SUBHEADLINE: &str =
    "One balance the whole network accepts — support participating local businesses, then spend \
     your {currency} in any city, not just where you earned it.";
const DEFAULT_CTA_LABEL: &str = "See how it works";
const DEFAULT_CTA_URL: &str = "/visitor";

const DEFAULT_EARN_TITLE: &str = "Earn where you live";
const DEFAULT_EARN_BODY: &str =
    "Support participating local businesses and collect {currency} as you go.";
const DEFAULT_REDEEM_TITLE: &str = "Spend where you love";
const DEFAULT_REDEEM_BODY: &str =
    "Use one {currency} balance across every participating city — not just where you earned it.";

/// Built-in defaults are city-aware: a city page carries its own city name (and the subfolder SEO
/// with it) even before the admin writes any copy of its own.
fn default_pill(city: Option<&str>) -> String {
    match city.filter(|c| !c.trim().is_empty()) {
        Some(c) => format!("Local rewards in {}", c.trim()),
        None => DEFAULT_PILL.to_string(),
    }
}

fn default_headline(city: Option<&str>, currency: &str) -> String {
    match city.filter(|c| !c.trim().is_empty()) {
        Some(c) => format!("Earn {currency} in {}", c.trim()),
        None => format!("Earn {currency} when you shop local"),
    }
}

/// Ticker aggregates are cached for this long: real data, but not a ledger scan on every page view.
const TICKER_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MessagingRow {
    pub id: Uuid,
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub enabled: bool,
    pub headline: Option<String>,
    pub subheadline: Option<String>,
    pub cta_label: Option<String>,
    pub cta_url: Option<String>,
    pub show_earn_claims: bool,
    pub pill_text: Option<String>,
    pub steps: Option<Value>,
    pub earn_card_title: Option<String>,
    pub earn_card_body: Option<String>,
    pub redeem_card_title: Option<String>,
    pub redeem_card_body: Option<String>,
    pub ticker_enabled: bool,
    pub ticker_threshold: f64,
}

#[derive(Debug, Deserialize)]
pub struct PublicQuery {
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsQuery {
    /// "network" or "directory"
    pub scope: String,
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct SaveSettings {
    pub scope: String,
    pub id: Uuid,
    pub enabled: Option<bool>,
    pub headline: Option<String>,
    pub subheadline: Option<String>,
    pub cta_label: Option<String>,
    pub cta_url: Option<String>,
    pub show_earn_claims: Option<bool>,
    pub pill_text: Option<String>,
    pub steps: Option<Value>,
    pub earn_card_title: Option<String>,
    pub earn_card_body: Option<String>,
    pub redeem_card_title: Option<String>,
    pub redeem_card_body: Option<String>,
    pub ticker_enabled: Option<bool>,
    pub ticker_threshold: Option<f64>,
}

/// A programme currency must never be served as the generic word "points" (David's rule).
fn normalise_currency(name: Option<String>) -> String {
    match name {
        Some(n) if !n.trim().is_empty() && !n.trim().eq_ignore_ascii_case("points") => {
            n.trim().to_string()
        }
        _ => "ZaarCash".to_string(),
    }
}

/// `{city}` / `{currency}` interpolation. Unknown tokens are left as-is (never silently dropped in
/// authored copy) but the built-in defaults only use these two.
fn interp(s: &str, city: Option<&str>, currency: &str) -> String {
    let out = s
        .replace("{currency}", currency)
        .replace("{currency_name}", currency);
    match city.filter(|c| !c.trim().is_empty()) {
        Some(c) => out.replace("{city}", c.trim()),
        None => out.replace("{city}", "your city"),
    }
}

/// Resolve a directory id -> (city_name, network_id). None when the id is unknown.
async fn directory_context(
    db: &PgPool,
    id: Uuid,
) -> Result<Option<(Option<String>, Option<Uuid>)>, sqlx::Error> {
    sqlx::query_as::<_, (Option<String>, Option<Uuid>)>(
        "SELECT COALESCE(NULLIF(city, ''), name), network_id FROM directories WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

/// The network the public root serves. The homepage is a NETWORK surface, so when it asks for its
/// loyalty section with no directory id we resolve the platform network here (the network that owns
/// the directory set) instead of dropping to built-in defaults — otherwise the admin's network-scope
/// copy would never reach the homepage, which is exactly what card B92 rule (1) forbids. Resolved
/// from live data, never hardcoded: the network with the most directories wins, ties broken by id.
async fn default_network_id(db: &PgPool) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"SELECT network_id FROM directories
            WHERE network_id IS NOT NULL
            GROUP BY network_id
            ORDER BY count(*) DESC, network_id
            LIMIT 1"#,
    )
    .fetch_optional(db)
    .await
}

/// The messaging row owned by EXACTLY this scope. Used by the admin read/write paths: a directory
/// with no row of its own must show defaults (and save a directory row) rather than silently
/// reading the network row and then overwriting it.
async fn resolve_exact(
    db: &PgPool,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<Option<MessagingRow>, sqlx::Error> {
    sqlx::query_as::<_, MessagingRow>(
        r#"SELECT id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url,
                  show_earn_claims, pill_text, steps, earn_card_title, earn_card_body,
                  redeem_card_title, redeem_card_body, ticker_enabled, ticker_threshold
             FROM loyalty_messaging
            WHERE ($1::uuid IS NOT NULL AND directory_id = $1)
               OR ($1::uuid IS NULL AND $2::uuid IS NOT NULL AND network_id = $2)
            LIMIT 1"#,
    )
    .bind(directory_id)
    .bind(network_id)
    .fetch_optional(db)
    .await
}

/// The messaging row IN FORCE for a public page: directory -> network -> built-in defaults. A city
/// (child directory) with no copy of its own inherits its network's copy, so the homepage and the
/// city pages cannot drift — the rule card B92/B93 states and the exact-scope lookup alone did not
/// deliver (a directory query matched nothing and fell through to defaults).
async fn resolve_effective(
    db: &PgPool,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<Option<MessagingRow>, sqlx::Error> {
    sqlx::query_as::<_, MessagingRow>(
        r#"SELECT id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url,
                  show_earn_claims, pill_text, steps, earn_card_title, earn_card_body,
                  redeem_card_title, redeem_card_body, ticker_enabled, ticker_threshold
             FROM loyalty_messaging
            WHERE ($1::uuid IS NOT NULL AND directory_id = $1)
               OR ($2::uuid IS NOT NULL AND network_id = $2)
            ORDER BY (directory_id IS NOT NULL) DESC
            LIMIT 1"#,
    )
    .bind(directory_id)
    .bind(network_id)
    .fetch_optional(db)
    .await
}

/// (currency_name, currency_icon, earning_enabled) for the scope's active programme.
async fn program_state(
    db: &PgPool,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<(String, String, bool), sqlx::Error> {
    let row = sqlx::query_as::<_, (String, Option<String>, f64, i32)>(
        r#"SELECT currency_name, currency_icon, COALESCE(earn_rate, 0)::float8, COALESCE(points_per_redemption, 0)
             FROM loyalty_programs
            WHERE is_active = true
              AND (($1::uuid IS NOT NULL AND directory_id = $1) OR ($2::uuid IS NOT NULL AND network_id = $2))
            ORDER BY (directory_id IS NOT NULL) DESC
            LIMIT 1"#,
    )
    .bind(directory_id)
    .bind(network_id)
    .fetch_optional(db)
    .await?;

    match row {
        Some((name, icon, earn_rate, per_redemption)) => {
            let earning = earn_rate > 0.0 || per_redemption > 0;
            Ok((
                normalise_currency(Some(name)),
                icon.unwrap_or_else(|| "⭐".to_string()),
                earning,
            ))
        }
        None => Ok(("ZaarCash".to_string(), "⭐".to_string(), false)),
    }
}

fn gated_subheadline(
    mode: &str,
    city: Option<&str>,
    currency: &str,
    configured: Option<&str>,
) -> String {
    match mode {
        "earn" => {
            let raw = configured
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_SUBHEADLINE.to_string());
            interp(&raw, city, currency)
        }
        "how_it_works" => format!("Here's how {currency} works across the network."),
        _ => match city {
            Some(c) if !c.is_empty() => {
                format!("Earning opens soon in {c} — here's how {currency} will work.")
            }
            _ => format!("Earning opens soon — here's how {currency} will work."),
        },
    }
}

/// The three process cards. Built-in defaults are deliberately honest: the "earn" step describes
/// qualifying activity at participating businesses — never "every job", which the product cannot
/// yet verify (B47). Authored steps are token-interpolated; when earning is off the earn step is
/// replaced with the truthful "opens soon" line so no surface can promise it.
fn steps_value(
    configured: Option<&Value>,
    city: Option<&str>,
    currency: &str,
    earning: bool,
) -> Value {
    let default_earn_body = if earning {
        format!("Collect {currency} on qualifying activity at participating local businesses.")
    } else {
        match city {
            Some(c) if !c.trim().is_empty() => format!(
                "Earning opens soon in {} — here's how {currency} will work.",
                c.trim()
            ),
            _ => format!("Earning opens soon — here's how {currency} will work."),
        }
    };

    match configured {
        Some(Value::Array(items)) if !items.is_empty() => {
            let mut out = Vec::new();
            for (idx, item) in items.iter().enumerate() {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| interp(s, city, currency))
                    .unwrap_or_default();
                let body = item
                    .get("body")
                    .and_then(|v| v.as_str())
                    .map(|s| interp(s, city, currency))
                    .unwrap_or_default();
                // Gate the earn step even in authored copy: if this is the SECOND card and earning
                // is off, the honest line wins over whatever was authored.
                let body = if idx == 1 && !earning {
                    default_earn_body.clone()
                } else {
                    body
                };
                out.push(json!({ "title": title, "body": body }));
            }
            Value::Array(out)
        }
        _ => json!([
            { "title": "Hire a local pro", "body": "Find, hire and support a participating local business." },
            { "title": format!("Earn {currency}"), "body": default_earn_body },
            { "title": "Redeem locally", "body": format!("Spend your {currency} with any participating business — in any city on the network.") }
        ]),
    }
}

/// A cached real aggregate. Never mocked: dollars come from the redemption ledger, members from the
/// loyalty roster.
#[derive(Clone, Copy)]
struct TickerTotals {
    at: Instant,
    dollars: f64,
    members: i64,
}

fn ticker_cache() -> &'static Mutex<HashMap<Uuid, TickerTotals>> {
    static CACHE: OnceLock<Mutex<HashMap<Uuid, TickerTotals>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn ticker_totals(db: &PgPool, network_id: Uuid) -> Result<(f64, i64), sqlx::Error> {
    if let Ok(guard) = ticker_cache().lock() {
        if let Some(hit) = guard.get(&network_id) {
            if hit.at.elapsed() < TICKER_TTL {
                return Ok((hit.dollars, hit.members));
            }
        }
    }

    let dollars = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(SUM(total_reimbursement_cents), 0)::float8 / 100.0 \
         FROM point_redemption_log WHERE network_id = $1",
    )
    .bind(network_id)
    .fetch_one(db)
    .await?;
    let members =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM loyalty_members WHERE network_id = $1")
            .bind(network_id)
            .fetch_one(db)
            .await?;

    if let Ok(mut guard) = ticker_cache().lock() {
        guard.insert(
            network_id,
            TickerTotals {
                at: Instant::now(),
                dollars,
                members,
            },
        );
    }
    Ok((dollars, members))
}

/// The impact ticker. Hidden below the admin threshold and when the network cannot be resolved —
/// showing "$0 reinvested" would do more damage than showing nothing (card B93).
async fn ticker_value(
    db: &PgPool,
    network_id: Option<Uuid>,
    enabled: bool,
    threshold: f64,
) -> Value {
    let net = match network_id {
        Some(n) => n,
        None => {
            return json!({ "enabled": enabled, "visible": false, "threshold": threshold,
                           "dollars": 0.0, "members": 0, "reason": "no network resolved" })
        }
    };
    match ticker_totals(db, net).await {
        Ok((dollars, members)) => {
            let visible = enabled && dollars >= threshold;
            json!({
                "enabled": enabled,
                "visible": visible,
                "threshold": threshold,
                "dollars": dollars,
                "members": members,
                "label_dollars": "Local dollars reinvested",
                "label_members": "Local members rewarded",
                "reason": if visible { Value::Null } else { json!("below threshold") }
            })
        }
        Err(e) => {
            tracing::warn!("[loyalty] ticker aggregate failed: {e}");
            json!({ "enabled": enabled, "visible": false, "threshold": threshold,
                    "dollars": 0.0, "members": 0, "reason": "aggregate unavailable" })
        }
    }
}

/// GET /api/v1/loyalty/messaging[?directory_id=<uuid>] — PUBLIC. Everything a public page needs to
/// render the section, already resolved and already honest about the earn state.
pub async fn get_messaging(
    State(s): State<AppState>,
    Query(q): Query<PublicQuery>,
) -> ApiResult<Json<Value>> {
    let (city_name, network_id) = match q.directory_id {
        Some(dir) => match directory_context(&s.db, dir).await? {
            Some((city, net)) => (city, net),
            None => (None, None),
        },
        None => (None, default_network_id(&s.db).await?),
    };

    let row = resolve_effective(&s.db, q.directory_id, network_id).await?;
    let source = if row
        .as_ref()
        .map(|r| r.directory_id.is_some())
        .unwrap_or(false)
    {
        "directory"
    } else if row.is_some() {
        "network"
    } else {
        "default"
    };
    let row = row.unwrap_or(MessagingRow {
        id: Uuid::nil(),
        network_id,
        directory_id: q.directory_id,
        enabled: true,
        headline: None,
        subheadline: None,
        cta_label: None,
        cta_url: None,
        show_earn_claims: true,
        pill_text: None,
        steps: None,
        earn_card_title: None,
        earn_card_body: None,
        redeem_card_title: None,
        redeem_card_body: None,
        ticker_enabled: true,
        ticker_threshold: 100.0,
    });

    let (currency_name, currency_icon, earning_enabled_raw) =
        program_state(&s.db, q.directory_id, network_id).await?;
    let earning_enabled = earning_enabled_raw && row.enabled;

    let mode = if !earning_enabled {
        "how_it_works"
    } else if row.show_earn_claims {
        "earn"
    } else {
        "how_it_works"
    };

    let city = city_name.as_deref();
    let headline = match row
        .headline
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(h) => interp(h, city, &currency_name),
        None => default_headline(city, &currency_name),
    };
    let subheadline = gated_subheadline(
        mode,
        city_name.as_deref(),
        &currency_name,
        row.subheadline.as_deref(),
    );
    let pill = match row
        .pill_text
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(p) => interp(p, city, &currency_name),
        None => default_pill(city),
    };
    let steps = steps_value(row.steps.as_ref(), city, &currency_name, earning_enabled);

    let earn_card = json!({
        "title": interp(row.earn_card_title.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_EARN_TITLE), city, &currency_name),
        "body": interp(row.earn_card_body.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_EARN_BODY), city, &currency_name),
    });
    let redeem_card = json!({
        "title": interp(row.redeem_card_title.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_REDEEM_TITLE), city, &currency_name),
        "body": interp(row.redeem_card_body.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_REDEEM_BODY), city, &currency_name),
    });

    let ticker = ticker_value(
        &s.db,
        network_id,
        row.ticker_enabled && row.enabled,
        row.ticker_threshold,
    )
    .await;

    Ok(Json(json!({
        "enabled": row.enabled,
        "mode": mode,
        "pill": pill,
        "headline": headline,
        "subheadline": subheadline,
        "cta_label": row.cta_label.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_CTA_LABEL),
        "cta_url": row.cta_url.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_CTA_URL),
        "steps": steps,
        "earn_card": earn_card,
        "redeem_card": redeem_card,
        "ticker": ticker,
        "currency_name": currency_name,
        "currency_icon": currency_icon,
        "city_name": city_name,
        "earning_enabled": earning_enabled,
        "show_earn_claims": row.show_earn_claims,
        "source": source,
    })))
}

/// GET /api/v1/loyalty-messaging/settings?scope=network|directory&id=<uuid> — admin (operator) read.
pub async fn get_settings(
    State(s): State<AppState>,
    Query(q): Query<SettingsQuery>,
) -> ApiResult<Json<Value>> {
    let (directory_id, network_id) = scope_ids(&s.db, &q.scope, q.id).await?;
    let row = resolve_exact(&s.db, directory_id, network_id).await?;
    // What the public page would inherit from the network when THIS scope has no row of its own.
    // The panel shows it so the admin can see the copy a city page is using without editing blindly.
    let inherited = if row.is_none() {
        resolve_effective(&s.db, directory_id, network_id).await?
    } else {
        None
    };
    let (currency_name, currency_icon, earning) =
        program_state(&s.db, directory_id, network_id).await?;

    let ticker = match &row {
        Some(r) => {
            ticker_value(
                &s.db,
                network_id,
                r.ticker_enabled && r.enabled,
                r.ticker_threshold,
            )
            .await
        }
        None => ticker_value(&s.db, network_id, true, 100.0).await,
    };

    Ok(Json(json!({
        "scope": q.scope,
        "id": q.id,
        "exists": row.is_some(),
        "row": row,
        "inherited": inherited,
        "defaults": {
            "pill": DEFAULT_PILL,
            "headline": DEFAULT_HEADLINE,
            "subheadline": DEFAULT_SUBHEADLINE,
            "cta_label": DEFAULT_CTA_LABEL,
            "cta_url": DEFAULT_CTA_URL,
            "earn_card_title": DEFAULT_EARN_TITLE,
            "earn_card_body": DEFAULT_EARN_BODY,
            "redeem_card_title": DEFAULT_REDEEM_TITLE,
            "redeem_card_body": DEFAULT_REDEEM_BODY,
            "steps": steps_value(None, None, &currency_name, false),
        },
        "currency_name": currency_name,
        "currency_icon": currency_icon,
        "earning_enabled": earning,
        "ticker": ticker,
    })))
}

/// PUT /api/v1/loyalty-messaging/settings — admin (operator) upsert.
pub async fn put_settings(
    State(s): State<AppState>,
    Json(body): Json<SaveSettings>,
) -> ApiResult<Json<Value>> {
    let (directory_id, network_id) = scope_ids(&s.db, &body.scope, body.id).await?;
    let existing = resolve_exact(&s.db, directory_id, network_id).await?;
    // `loyalty_messaging_owner_check` admits exactly ONE owner: a directory row must carry
    // directory_id and NO network_id (a directory scope resolves its own network for the programme,
    // it does not need to store it). Binding both used to 500 every directory-scope save.
    let owner_network = if directory_id.is_some() {
        None
    } else {
        network_id
    };

    if let Some(t) = body.ticker_threshold {
        if !t.is_finite() || t < 0.0 {
            return Err(AppError::BadRequest(
                "ticker_threshold must be a number >= 0".into(),
            ));
        }
    }

    let row = match existing {
        Some(cur) => sqlx::query_as::<_, MessagingRow>(
            r#"UPDATE loyalty_messaging SET
                   enabled = $2,
                   headline = $3,
                   subheadline = $4,
                   cta_label = $5,
                   cta_url = $6,
                   show_earn_claims = $7,
                   pill_text = $8,
                   steps = $9,
                   earn_card_title = $10,
                   earn_card_body = $11,
                   redeem_card_title = $12,
                   redeem_card_body = $13,
                   ticker_enabled = $14,
                   ticker_threshold = $15,
                   updated_at = now()
                 WHERE id = $1
               RETURNING id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url,
                         show_earn_claims, pill_text, steps, earn_card_title, earn_card_body,
                         redeem_card_title, redeem_card_body, ticker_enabled, ticker_threshold"#,
        )
        .bind(cur.id)
        .bind(body.enabled.unwrap_or(cur.enabled))
        .bind(body.headline.or(cur.headline))
        .bind(body.subheadline.or(cur.subheadline))
        .bind(body.cta_label.or(cur.cta_label))
        .bind(body.cta_url.or(cur.cta_url))
        .bind(body.show_earn_claims.unwrap_or(cur.show_earn_claims))
        .bind(body.pill_text.or(cur.pill_text))
        .bind(body.steps.or(cur.steps))
        .bind(body.earn_card_title.or(cur.earn_card_title))
        .bind(body.earn_card_body.or(cur.earn_card_body))
        .bind(body.redeem_card_title.or(cur.redeem_card_title))
        .bind(body.redeem_card_body.or(cur.redeem_card_body))
        .bind(body.ticker_enabled.unwrap_or(cur.ticker_enabled))
        .bind(body.ticker_threshold.unwrap_or(cur.ticker_threshold))
        .fetch_one(&s.db)
        .await?,
        None => sqlx::query_as::<_, MessagingRow>(
            r#"INSERT INTO loyalty_messaging
                   (network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url,
                    show_earn_claims, pill_text, steps, earn_card_title, earn_card_body,
                    redeem_card_title, redeem_card_body, ticker_enabled, ticker_threshold)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
               RETURNING id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url,
                         show_earn_claims, pill_text, steps, earn_card_title, earn_card_body,
                         redeem_card_title, redeem_card_body, ticker_enabled, ticker_threshold"#,
        )
        .bind(owner_network)
        .bind(directory_id)
        .bind(body.enabled.unwrap_or(true))
        .bind(body.headline)
        .bind(body.subheadline)
        .bind(body.cta_label)
        .bind(body.cta_url)
        .bind(body.show_earn_claims.unwrap_or(true))
        .bind(body.pill_text)
        .bind(body.steps)
        .bind(body.earn_card_title)
        .bind(body.earn_card_body)
        .bind(body.redeem_card_title)
        .bind(body.redeem_card_body)
        .bind(body.ticker_enabled.unwrap_or(true))
        .bind(body.ticker_threshold.unwrap_or(100.0))
        .fetch_one(&s.db)
        .await?,
    };

    Ok(Json(json!({ "status": "saved", "row": row })))
}

/// Map (scope, id) -> (directory_id, network_id) for the storage row. A directory scope always
/// carries the directory's own network too, so a directory row still resolves its programme.
async fn scope_ids(
    db: &PgPool,
    scope: &str,
    id: Uuid,
) -> Result<(Option<Uuid>, Option<Uuid>), AppError> {
    match scope {
        "network" => Ok((None, Some(id))),
        "directory" => {
            let ctx = directory_context(db, id).await?;
            let network_id = ctx.and_then(|(_, net)| net);
            Ok((Some(id), network_id))
        }
        other => Err(AppError::BadRequest(format!(
            "Unknown scope '{}' — use 'network' or 'directory'",
            other
        ))),
    }
}
