//! Loyalty messaging (card B92) — the admin-configurable ZaarCash section that rides the shared
//! page chrome on the homepage AND every city page.
//!
//! Two rules from the card are enforced HERE, in the back end, so no surface can drift:
//!   1. NOTHING HARDCODED — on/off + copy + CTA come from `loyalty_messaging`, resolved
//!      directory -> network -> built-in defaults. A buyer with no agent can change or switch it
//!      off from the admin panel.
//!   2. HONEST STATE — whether a visitor can actually earn is computed from `loyalty_programs`
//!      (earn_rate / points_per_redemption). While nothing can be earned the served copy is the
//!      truthful "how it works" line, never an earnings promise; the full earn copy appears
//!      automatically once a rate is set. The word "points" is never served — the programme
//!      currency is ZaarCash (or whatever the network renames it to).

use crate::error::{ApiResult, AppError};
use crate::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const DEFAULT_HEADLINE: &str = "Earn ZaarCash when you shop local";
const DEFAULT_SUBHEADLINE: &str =
    "One balance the whole network accepts — support participating local businesses, then spend \
     your ZaarCash in any city, not just where you earned it.";
const DEFAULT_CTA_LABEL: &str = "See how it works";
const DEFAULT_CTA_URL: &str = "/visitor";

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

/// The messaging row in force for a scope: an explicit directory row wins over the network row.
async fn resolve_row(
    db: &PgPool,
    directory_id: Option<Uuid>,
    network_id: Option<Uuid>,
) -> Result<Option<MessagingRow>, sqlx::Error> {
    sqlx::query_as::<_, MessagingRow>(
        r#"SELECT id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url, show_earn_claims
             FROM loyalty_messaging
            WHERE ($1::uuid IS NOT NULL AND directory_id = $1)
               OR ($1::uuid IS NULL AND $2::uuid IS NOT NULL AND network_id = $2)
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
        "earn" => configured
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_SUBHEADLINE.to_string()),
        "how_it_works" => format!("Here's how {currency} works across the network."),
        _ => match city {
            Some(c) if !c.is_empty() => {
                format!("Earning opens soon in {c} — here's how {currency} will work.")
            }
            _ => format!("Earning opens soon — here's how {currency} will work."),
        },
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

    let row = resolve_row(&s.db, q.directory_id, network_id).await?;
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
    });

    let (currency_name, currency_icon, earning_enabled) =
        program_state(&s.db, q.directory_id, network_id).await?;
    let earning_enabled = earning_enabled && row.enabled;

    let mode = if !earning_enabled {
        "how_it_works"
    } else if row.show_earn_claims {
        "earn"
    } else {
        "how_it_works"
    };

    let headline = row
        .headline
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_HEADLINE)
        .to_string();
    let subheadline = gated_subheadline(
        mode,
        city_name.as_deref(),
        &currency_name,
        row.subheadline.as_deref(),
    );

    Ok(Json(json!({
        "enabled": row.enabled,
        "mode": mode,
        "headline": headline,
        "subheadline": subheadline,
        "cta_label": row.cta_label.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_CTA_LABEL),
        "cta_url": row.cta_url.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(DEFAULT_CTA_URL),
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
    let row = resolve_row(&s.db, directory_id, network_id).await?;
    let (currency_name, currency_icon, _earning) =
        program_state(&s.db, directory_id, network_id).await?;

    Ok(Json(json!({
        "scope": q.scope,
        "id": q.id,
        "exists": row.is_some(),
        "row": row,
        "defaults": {
            "headline": DEFAULT_HEADLINE,
            "subheadline": DEFAULT_SUBHEADLINE,
            "cta_label": DEFAULT_CTA_LABEL,
            "cta_url": DEFAULT_CTA_URL,
        },
        "currency_name": currency_name,
        "currency_icon": currency_icon,
    })))
}

/// PUT /api/v1/loyalty-messaging/settings — admin (operator) upsert.
pub async fn put_settings(
    State(s): State<AppState>,
    Json(body): Json<SaveSettings>,
) -> ApiResult<Json<Value>> {
    let (directory_id, network_id) = scope_ids(&s.db, &body.scope, body.id).await?;
    let existing = resolve_row(&s.db, directory_id, network_id).await?;

    let row = match existing {
        Some(cur) => sqlx::query_as::<_, MessagingRow>(
            r#"UPDATE loyalty_messaging SET
                   enabled = $2,
                   headline = $3,
                   subheadline = $4,
                   cta_label = $5,
                   cta_url = $6,
                   show_earn_claims = $7,
                   updated_at = now()
                 WHERE id = $1
               RETURNING id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url, show_earn_claims"#,
        )
        .bind(cur.id)
        .bind(body.enabled.unwrap_or(cur.enabled))
        .bind(body.headline.or(cur.headline))
        .bind(body.subheadline.or(cur.subheadline))
        .bind(body.cta_label.or(cur.cta_label))
        .bind(body.cta_url.or(cur.cta_url))
        .bind(body.show_earn_claims.unwrap_or(cur.show_earn_claims))
        .fetch_one(&s.db)
        .await?,
        None => sqlx::query_as::<_, MessagingRow>(
            r#"INSERT INTO loyalty_messaging
                   (network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url, show_earn_claims)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, network_id, directory_id, enabled, headline, subheadline, cta_label, cta_url, show_earn_claims"#,
        )
        .bind(network_id)
        .bind(directory_id)
        .bind(body.enabled.unwrap_or(true))
        .bind(body.headline)
        .bind(body.subheadline)
        .bind(body.cta_label)
        .bind(body.cta_url)
        .bind(body.show_earn_claims.unwrap_or(true))
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
