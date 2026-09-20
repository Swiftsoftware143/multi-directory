//! Clearinghouse settlement runs (T3) — close the money loop.
//!
//! The loyalty ledger already worked (`point_issuance_log` / `point_redemption_log` /
//! `business_point_ledger` hold live data); what never existed was the monthly
//! settlement that turns that ledger into invoices, payouts and per-business
//! statements.
//!
//! David's economics: the issuing business pays **$0.01 per point issued**, the
//! redeeming business is reimbursed **$0.008 per point redeemed**, and the **$0.002
//! spread** is platform revenue. None of those rates are constants here — they are
//! read from `point_treasury` (admin-editable via the Settlement Settings form) at
//! run time and copied onto the run, so a historical run keeps the rates it used.
//!
//! Invariants this module protects:
//!   * **Idempotent by period** — UNIQUE (network_id, period_start, period_end).
//!     Re-running a period returns the existing run; it can never double-bill.
//!   * **Never a fake success** — with no payment provider configured the run is
//!     marked `pending_provider` and the ledger stays correct. Nothing is reported
//!     as paid that was not paid.
//!   * **Never a panic** — an unconfigured provider, a missing treasury row or a
//!     provider API error all degrade to a recorded status plus a message.

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::provider_keys_handler;
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
    Extension, Json,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// Providers we can attempt a real payout with. Anything else configured in
/// `provider_keys` is left `pending_provider` with an honest message rather than
/// being silently treated as success.
const KNOWN_PAYMENT_PROVIDERS: [&str; 4] = ["stripe", "paypal", "square", "wise"];

// ─────────────────────────────────────────────────────────────────────────────
// Resolution helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Directory slug → network id (same convention as the existing /networks/:slug/clear/* routes).
async fn resolve_network(db: &PgPool, slug: &str) -> Result<Uuid, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(slug)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Directory '{slug}' not found")))?;

    let network_id: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_optional(db)
            .await?
            .flatten();

    network_id.ok_or_else(|| {
        AppError::BadRequest(format!(
            "Directory '{slug}' is not part of a network — settlement is network-wide"
        ))
    })
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SettlementSettings {
    pub network_id: Uuid,
    /// Dollars per point (0.01 = one cent per point issued).
    pub issuance_rate: Decimal,
    /// Dollars per point (0.008 = 0.8 cents per point redeemed).
    pub redemption_rate: Decimal,
    pub platform_spread_percent: Decimal,
    pub minimum_float: Decimal,
    pub default_expiry_days: i32,
    pub cycle_day: i32,
    pub currency: String,
    pub minimum_payout_cents: i32,
    pub settlement_enabled: bool,
    pub payment_provider: Option<String>,
}

/// Read the settings, creating the treasury row with sane defaults if the network
/// has none yet. A missing row must not 500 the endpoint.
async fn get_settings(db: &PgPool, network_id: Uuid) -> Result<SettlementSettings, AppError> {
    let sql = "SELECT network_id, issuance_rate, redemption_rate, platform_spread_percent, \
                      minimum_float, default_expiry_days, cycle_day, currency, minimum_payout_cents, \
                      settlement_enabled, payment_provider \
               FROM point_treasury WHERE network_id = $1";

    let existing: Option<SettlementSettings> = sqlx::query_as::<_, SettlementSettings>(sql)
        .bind(network_id)
        .fetch_optional(db)
        .await?;

    if let Some(row) = existing {
        return Ok(row);
    }

    sqlx::query("INSERT INTO point_treasury (network_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(network_id)
        .execute(db)
        .await?;

    sqlx::query_as::<_, SettlementSettings>(sql)
        .bind(network_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| {
            AppError::Internal("Could not read or create the network treasury row".into())
        })
}

// ─────────────────────────────────────────────────────────────────────────────
// Settings endpoints
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/networks/:slug/settlement/settings
pub async fn settlement_settings(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let network_id = resolve_network(&s.db, &slug).await?;
    let settings = get_settings(&s.db, network_id).await?;
    let (provider, configured) = detect_provider(&s.db, &settings).await;

    Ok(Json(json!({
        "settings": settings,
        "provider": {
            "effective": provider,
            "configured": configured,
            "known_providers": KNOWN_PAYMENT_PROVIDERS,
            "mode": if configured { "live" } else { "pending_provider" },
        },
        "effective_rates": {
            "issue_per_point": format!("{:.4}", settings.issuance_rate),
            "redeem_per_point": format!("{:.4}", settings.redemption_rate),
            "spread_per_point": format!("{:.4}", spread_per_point(&settings)),
        },
    })))
}

fn spread_per_point(settings: &SettlementSettings) -> Decimal {
    settings.issuance_rate - settings.redemption_rate
}

#[derive(Debug, Deserialize)]
pub struct SettlementSettingsUpdate {
    /// Dollars per point issued (e.g. 0.01).
    pub issuance_rate: Option<Decimal>,
    /// Dollars per point redeemed (e.g. 0.008).
    pub redemption_rate: Option<Decimal>,
    pub cycle_day: Option<i32>,
    pub currency: Option<String>,
    pub minimum_payout_cents: Option<i32>,
    pub settlement_enabled: Option<bool>,
    /// Blank string clears it back to auto-detect.
    pub payment_provider: Option<String>,
    pub default_expiry_days: Option<i32>,
}

/// PUT /api/v1/networks/:slug/settlement/settings
/// Admin-only: these are the rates the money is actually moved at.
pub async fn update_settlement_settings(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<SettlementSettingsUpdate>,
) -> ApiResult<impl IntoResponse> {
    if claims.role != "admin" && claims.role != "super_admin" {
        return Err(AppError::Forbidden(
            "Admin role required to change settlement settings".into(),
        ));
    }

    let network_id = resolve_network(&s.db, &slug).await?;
    let current = get_settings(&s.db, network_id).await?;

    // Validate rather than silently coercing: a nonsense rate must not become money.
    let issue = req.issuance_rate.unwrap_or(current.issuance_rate);
    let redeem = req.redemption_rate.unwrap_or(current.redemption_rate);
    if issue < Decimal::ZERO || issue > Decimal::from(1000) {
        return Err(AppError::Validation(
            "issuance_rate must be between 0 and 1000 dollars per point".into(),
        ));
    }
    if redeem < Decimal::ZERO || redeem > issue {
        return Err(AppError::Validation(
            "redemption_rate must be between 0 and the issuance rate (the spread cannot be negative)".into(),
        ));
    }

    let cycle_day = req.cycle_day.unwrap_or(current.cycle_day).clamp(1, 28);
    let currency = req
        .currency
        .map(|c| c.trim().to_uppercase())
        .filter(|c| !c.is_empty() && c.len() <= 8)
        .unwrap_or(current.currency);
    let min_payout = req
        .minimum_payout_cents
        .unwrap_or(current.minimum_payout_cents)
        .max(0);
    let enabled = req.settlement_enabled.unwrap_or(current.settlement_enabled);
    let expiry = req
        .default_expiry_days
        .unwrap_or(current.default_expiry_days)
        .max(1);
    let provider = match req.payment_provider {
        Some(p) if p.trim().is_empty() => None,
        Some(p) => Some(p.trim().to_lowercase()),
        None => current.payment_provider.clone(),
    };

    sqlx::query(
        "UPDATE point_treasury SET issuance_rate = $2, redemption_rate = $3, cycle_day = $4, \
            currency = $5, minimum_payout_cents = $6, settlement_enabled = $7, \
            payment_provider = $8, default_expiry_days = $9, updated_at = NOW() \
         WHERE network_id = $1",
    )
    .bind(network_id)
    .bind(issue)
    .bind(redeem)
    .bind(cycle_day)
    .bind(&currency)
    .bind(min_payout)
    .bind(enabled)
    .bind(&provider)
    .bind(expiry)
    .execute(&s.db)
    .await?;

    let settings = get_settings(&s.db, network_id).await?;
    let (effective, configured) = detect_provider(&s.db, &settings).await;

    Ok(Json(json!({
        "ok": true,
        "settings": settings,
        "provider": { "effective": effective, "configured": configured },
    })))
}

/// Which provider will actually be used, and whether a key exists for it.
/// Returns (name, key_present).
async fn detect_provider(db: &PgPool, settings: &SettlementSettings) -> (Option<String>, bool) {
    if let Some(named) = settings.payment_provider.as_ref().filter(|p| !p.is_empty()) {
        let key = provider_keys_handler::resolve_provider_key(db, named).await;
        let has = key.map(|k| !k.trim().is_empty()).unwrap_or(false);
        return (Some(named.clone()), has);
    }
    for candidate in KNOWN_PAYMENT_PROVIDERS {
        if let Some(key) = provider_keys_handler::resolve_provider_key(db, candidate).await {
            if !key.trim().is_empty() {
                return (Some(candidate.to_string()), true);
            }
        }
    }
    (None, false)
}

// ─────────────────────────────────────────────────────────────────────────────
// Aggregation from the live ledger
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BusinessPoints {
    pub business_id: Option<Uuid>,
    pub business_name: String,
    pub points: i64,
}

async fn issued_by_business(
    db: &PgPool,
    network_id: Uuid,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Result<Vec<BusinessPoints>, AppError> {
    let rows = sqlx::query_as::<_, BusinessPoints>(
        "SELECT l.issuing_business_id AS business_id, \
                COALESCE(MAX(l.business_name), MAX(b.name), 'Unknown business') AS business_name, \
                COALESCE(SUM(l.points_issued), 0)::bigint AS points \
         FROM point_issuance_log l \
         LEFT JOIN businesses b ON b.id = l.issuing_business_id \
         WHERE l.network_id = $1 \
           AND l.created_at >= $2::date \
           AND l.created_at < ($3::date + INTERVAL '1 day') \
         GROUP BY l.issuing_business_id \
         ORDER BY points DESC",
    )
    .bind(network_id)
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

async fn redeemed_by_business(
    db: &PgPool,
    network_id: Uuid,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Result<Vec<BusinessPoints>, AppError> {
    let rows = sqlx::query_as::<_, BusinessPoints>(
        "SELECT l.redeeming_business_id AS business_id, \
                COALESCE(MAX(l.business_name), MAX(b.name), 'Unknown business') AS business_name, \
                COALESCE(SUM(l.points_redeemed), 0)::bigint AS points \
         FROM point_redemption_log l \
         LEFT JOIN businesses b ON b.id = l.redeeming_business_id \
         WHERE l.network_id = $1 \
           AND l.created_at >= $2::date \
           AND l.created_at < ($3::date + INTERVAL '1 day') \
         GROUP BY l.redeeming_business_id \
         ORDER BY points DESC",
    )
    .bind(network_id)
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// points × dollars-per-point × 100, rounded to whole cents.
fn amount_cents(points: i64, rate: Decimal) -> Decimal {
    (Decimal::from(points) * rate * Decimal::from(100)).round_dp(2)
}

#[derive(Debug, Serialize)]
pub struct InvoicePreview {
    pub business_id: Option<Uuid>,
    pub business_name: String,
    pub points_issued: i64,
    pub amount_cents: String,
}

#[derive(Debug, Serialize)]
pub struct PayoutPreview {
    pub business_id: Option<Uuid>,
    pub business_name: String,
    pub points_redeemed: i64,
    pub amount_cents: String,
    pub below_minimum: bool,
}

async fn build_preview(
    db: &PgPool,
    network_id: Uuid,
    settings: &SettlementSettings,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Result<
    (
        Vec<InvoicePreview>,
        Vec<PayoutPreview>,
        i64,
        i64,
        Decimal,
        Decimal,
    ),
    AppError,
> {
    let issued = issued_by_business(db, network_id, start, end).await?;
    let redeemed = redeemed_by_business(db, network_id, start, end).await?;

    let invoices: Vec<InvoicePreview> = issued
        .iter()
        .map(|r| InvoicePreview {
            business_id: r.business_id,
            business_name: r.business_name.clone(),
            points_issued: r.points,
            amount_cents: format!("{:.2}", amount_cents(r.points, settings.issuance_rate)),
        })
        .collect();

    let payouts: Vec<PayoutPreview> = redeemed
        .iter()
        .map(|r| {
            let cents = amount_cents(r.points, settings.redemption_rate);
            PayoutPreview {
                business_id: r.business_id,
                business_name: r.business_name.clone(),
                points_redeemed: r.points,
                amount_cents: format!("{:.2}", cents),
                below_minimum: cents < Decimal::from(settings.minimum_payout_cents),
            }
        })
        .collect();

    let total_issued: i64 = issued.iter().map(|r| r.points).sum();
    let total_redeemed: i64 = redeemed.iter().map(|r| r.points).sum();
    let total_invoiced = amount_cents(total_issued, settings.issuance_rate);
    let total_payout = amount_cents(total_redeemed, settings.redemption_rate);

    Ok((
        invoices,
        payouts,
        total_issued,
        total_redeemed,
        total_invoiced,
        total_payout,
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Period handling
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PeriodQuery {
    /// Inclusive ISO dates (YYYY-MM-DD). Omit both for the previous calendar month.
    pub period_start: Option<chrono::NaiveDate>,
    pub period_end: Option<chrono::NaiveDate>,
    pub notes: Option<String>,
}

impl PeriodQuery {
    /// Resolve the period. Explicit dates win; otherwise the previous calendar
    /// month relative to today (a settlement run bills a *closed* period).
    fn resolve(
        &self,
        settings: &SettlementSettings,
    ) -> Result<(chrono::NaiveDate, chrono::NaiveDate), AppError> {
        match (self.period_start, self.period_end) {
            (Some(a), Some(b)) => {
                if b < a {
                    return Err(AppError::Validation(
                        "period_end is before period_start".into(),
                    ));
                }
                Ok((a, b))
            }
            (None, None) => Ok(previous_month(
                chrono::Utc::now().date_naive(),
                settings.cycle_day,
            )),
            _ => Err(AppError::Validation(
                "provide both period_start and period_end, or neither (previous month)".into(),
            )),
        }
    }
}

/// The most recent closed cycle. `cycle_day` is the day of the month the cycle
/// closes on, so the window is the previous calendar month by default.
fn previous_month(
    today: chrono::NaiveDate,
    cycle_day: i32,
) -> (chrono::NaiveDate, chrono::NaiveDate) {
    use chrono::Datelike;
    let _ = cycle_day; // window is calendar-month based; cycle_day only labels the run
    let (y, m) = (today.year(), today.month());
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let start = chrono::NaiveDate::from_ymd_opt(py, pm, 1).unwrap_or(today);
    let end = start + chrono::Duration::days(30);
    let end = chrono::NaiveDate::from_ymd_opt(
        if pm == 12 { py + 1 } else { py },
        if pm == 12 { 1 } else { pm + 1 },
        1,
    )
    .map(|d| d - chrono::Duration::days(1))
    .unwrap_or(end);
    (start, end)
}

fn period_key(start: chrono::NaiveDate) -> String {
    start.format("%Y-%m").to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Preview / run
// ─────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/networks/:slug/settlement/preview — dry run, writes nothing.
pub async fn settlement_preview(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<PeriodQuery>,
) -> ApiResult<impl IntoResponse> {
    let network_id = resolve_network(&s.db, &slug).await?;
    let settings = get_settings(&s.db, network_id).await?;
    let (start, end) = req.resolve(&settings)?;
    let (invoices, payouts, pts_issued, pts_redeemed, invoiced, payout_total) =
        build_preview(&s.db, network_id, &settings, start, end).await?;

    let net = invoiced - payout_total;

    Ok(Json(json!({
        "dry_run": true,
        "network_id": network_id,
        "period": { "start": start.to_string(), "end": end.to_string(), "key": period_key(start) },
        "rates": {
            "issue_per_point": format!("{:.4}", settings.issuance_rate),
            "redeem_per_point": format!("{:.4}", settings.redemption_rate),
            "currency": settings.currency,
            "minimum_payout_cents": settings.minimum_payout_cents,
        },
        "totals": {
            "points_issued": pts_issued,
            "points_redeemed": pts_redeemed,
            "invoiced_cents": format!("{:.2}", invoiced),
            "payout_cents": format!("{:.2}", payout_total),
            "platform_spread_cents": format!("{:.2}", net),
        },
        "invoices": invoices,
        "payouts": payouts,
    })))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RunRow {
    pub id: Uuid,
    pub network_id: Uuid,
    pub period_key: String,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub status: String,
    pub currency: String,
    pub rate_issue_per_point: Decimal,
    pub rate_redeem_per_point: Decimal,
    pub min_payout_cents: i32,
    pub cycle_day: i32,
    pub total_points_issued: i64,
    pub total_points_redeemed: i64,
    pub total_invoiced_cents: Decimal,
    pub total_payout_cents: Decimal,
    pub platform_spread_cents: Decimal,
    pub provider: Option<String>,
    pub provider_ref: Option<String>,
    pub provider_message: Option<String>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 'manual' | 'scheduler' — how the run actually fired.
    pub triggered_by: String,
    pub statements_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    pub statements_sent: i32,
    pub statements_skipped: i32,
    pub statements_message: Option<String>,
}

const RUN_COLS: &str = "id, network_id, period_key, period_start, period_end, status, currency, \
    rate_issue_per_point, rate_redeem_per_point, min_payout_cents, cycle_day, \
    total_points_issued, total_points_redeemed, total_invoiced_cents, total_payout_cents, \
    platform_spread_cents, provider, provider_ref, provider_message, notes, created_at, completed_at, \
    triggered_by, statements_sent_at, statements_sent, statements_skipped, statements_message";

/// The outcome of one settlement attempt — the single result type shared by the
/// manual endpoint and the scheduler, so the two can never report differently.
#[derive(Debug, Serialize)]
pub struct SettlementRunOutcome {
    pub ok: bool,
    pub already_existed: bool,
    pub status: String,
    pub message: Option<String>,
    pub run: Option<RunRow>,
    pub invoices_created: usize,
    pub payouts_created: usize,
    pub payouts_paid: usize,
    pub payouts_failed: usize,
    pub provider: Option<String>,
    pub provider_configured: bool,
}

/// POST /api/v1/networks/:slug/settlement/run
///
/// Idempotent per period: a second run for the same window returns the first run
/// untouched (`already_existed: true`) so nobody is billed twice. This endpoint and
/// the settlement scheduler both go through [`execute_settlement`], so a scheduled
/// run and a manual run can never diverge.
pub async fn settlement_run(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<PeriodQuery>,
) -> ApiResult<impl IntoResponse> {
    if claims.role != "admin" && claims.role != "super_admin" {
        return Err(AppError::Forbidden(
            "Admin role required to run settlement".into(),
        ));
    }

    let network_id = resolve_network(&s.db, &slug).await?;
    let settings = get_settings(&s.db, network_id).await?;
    let (start, end) = req.resolve(&settings)?;
    let created_by = Uuid::parse_str(&claims.sub).ok();

    let outcome = execute_settlement(
        &s.db,
        network_id,
        start,
        end,
        req.notes.clone(),
        created_by,
        "manual",
    )
    .await?;

    Ok(Json(settlement_outcome_json(&outcome)))
}

/// Render an outcome the way the endpoint has always reported it (plus the fields
/// the scheduler adds), so the existing panel keeps working unchanged.
fn settlement_outcome_json(o: &SettlementRunOutcome) -> Value {
    json!({
        "ok": o.ok,
        "already_existed": o.already_existed,
        "status": o.status,
        "message": o.message,
        "run": o.run,
        "invoices_created": o.invoices_created,
        "payouts_created": o.payouts_created,
        "payouts_paid": o.payouts_paid,
        "payouts_failed": o.payouts_failed,
        "provider": o.provider,
        "provider_configured": o.provider_configured,
    })
}

/// **The** settlement run — ONE code path with two callers: the admin endpoint above
/// and the hourly scheduler. Creates the period's invoices and payouts from the live
/// ledger at the rates frozen onto the run.
///
/// `triggered_by` is `"manual"` or `"scheduler"` and is stored on the run, so the
/// panel can show how a run actually fired. Idempotency is enforced by the database
/// (`settlement_runs_network_period_key`), not by a check in this function: a
/// restart, a double wake-up or a manual re-run all return the existing run.
pub async fn execute_settlement(
    db: &PgPool,
    network_id: Uuid,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    notes: Option<String>,
    created_by: Option<Uuid>,
    triggered_by: &str,
) -> Result<SettlementRunOutcome, AppError> {
    let settings = get_settings(db, network_id).await?;

    if !settings.settlement_enabled {
        return Ok(SettlementRunOutcome {
            ok: false,
            already_existed: false,
            status: "disabled".into(),
            message: Some(
                "Settlement is disabled for this network — enable it in the Clearinghouse settings first."
                    .into(),
            ),
            run: None,
            invoices_created: 0,
            payouts_created: 0,
            payouts_paid: 0,
            payouts_failed: 0,
            provider: None,
            provider_configured: false,
        });
    }

    // Claim the period. DO NOTHING on conflict => re-runs cannot double-bill.
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO settlement_runs \
            (network_id, period_start, period_end, period_key, status, currency, \
             rate_issue_per_point, rate_redeem_per_point, min_payout_cents, cycle_day, \
             created_by, notes, triggered_by) \
         VALUES ($1, $2, $3, $4, 'processing', $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (network_id, period_start, period_end) DO NOTHING \
         RETURNING id",
    )
    .bind(network_id)
    .bind(start)
    .bind(end)
    .bind(period_key(start))
    .bind(&settings.currency)
    .bind(settings.issuance_rate)
    .bind(settings.redemption_rate)
    .bind(settings.minimum_payout_cents)
    .bind(settings.cycle_day)
    .bind(created_by)
    .bind(&notes)
    .bind(triggered_by)
    .fetch_optional(db)
    .await?;

    let Some(run_id) = inserted else {
        // Period already settled — return the existing run, do not touch the ledger.
        let existing = sqlx::query_as::<_, RunRow>(&format!(
            "SELECT {RUN_COLS} FROM settlement_runs WHERE network_id = $1 AND period_start = $2 AND period_end = $3"
        ))
        .bind(network_id)
        .bind(start)
        .bind(end)
        .fetch_optional(db)
        .await?;

        return Ok(SettlementRunOutcome {
            ok: true,
            already_existed: true,
            status: existing
                .as_ref()
                .map(|r| r.status.clone())
                .unwrap_or_else(|| "unknown".into()),
            message: Some(
                "This period was already settled — returning the existing run. No new invoices or payouts were created."
                    .into(),
            ),
            run: existing,
            invoices_created: 0,
            payouts_created: 0,
            payouts_paid: 0,
            payouts_failed: 0,
            provider: None,
            provider_configured: false,
        });
    };

    let (invoices, payouts, pts_issued, pts_redeemed, invoiced, payout_total) =
        build_preview(db, network_id, &settings, start, end).await?;

    // Invoices: what each issuing business owes for the period.
    for inv in &invoices {
        let cents = amount_cents(inv.points_issued, settings.issuance_rate);
        sqlx::query(
            "INSERT INTO settlement_invoices \
                (run_id, business_id, business_name, points_issued, rate_per_point, amount_cents, currency, status, due_date) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', ($8::date + INTERVAL '30 days')::date) \
             ON CONFLICT (run_id, business_id) DO NOTHING",
        )
        .bind(run_id)
        .bind(inv.business_id)
        .bind(&inv.business_name)
        .bind(inv.points_issued)
        .bind(settings.issuance_rate)
        .bind(cents)
        .bind(&settings.currency)
        .bind(end)
        .execute(db)
        .await?;
    }

    // Payouts: what each redeeming business is reimbursed. Below the configured
    // minimum the row is still written (the ledger stays correct) but is not paid.
    let (provider, configured) = detect_provider(db, &settings).await;
    let provider_key = match (&provider, configured) {
        (Some(p), true) => provider_keys_handler::resolve_provider_key(db, p).await,
        _ => None,
    };
    let destination = platform_payout_destination(db, provider.as_deref()).await;

    let mut paid = 0usize;
    let mut failed = 0usize;

    for po in &payouts {
        let cents = amount_cents(po.points_redeemed, settings.redemption_rate);

        let (status, ref_id, message): (String, Option<String>, Option<String>) = if cents
            < Decimal::from(settings.minimum_payout_cents)
        {
            (
                "below_minimum".to_string(),
                None,
                Some(format!(
                    "{} is below the configured minimum payout of {} cents",
                    cents, settings.minimum_payout_cents
                )),
            )
        } else if let (Some(p), Some(key)) = (provider.as_ref(), provider_key.as_ref()) {
            if p == "stripe" {
                match stripe_transfer(
                    key,
                    cents,
                    &settings.currency,
                    destination.as_deref(),
                    run_id,
                )
                .await
                {
                    Ok(id) => {
                        paid += 1;
                        ("paid".to_string(), Some(id), None)
                    }
                    Err(e) => {
                        failed += 1;
                        ("failed".to_string(), None, Some(e))
                    }
                }
            } else {
                (
                    "pending_provider".to_string(),
                    None,
                    Some(format!(
                        "no payout adapter implemented for '{p}' — the ledger row is recorded, nothing was sent"
                    )),
                )
            }
        } else {
            (
                "pending_provider".to_string(),
                None,
                Some("no payment provider configured for this network".to_string()),
            )
        };

        sqlx::query(
            "INSERT INTO settlement_payouts \
                (run_id, business_id, business_name, points_redeemed, rate_per_point, amount_cents, \
                 currency, status, provider, provider_ref, provider_message, paid_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, \
                     CASE WHEN $8 = 'paid' THEN NOW() ELSE NULL END) \
             ON CONFLICT (run_id, business_id) DO NOTHING",
        )
        .bind(run_id)
        .bind(po.business_id)
        .bind(&po.business_name)
        .bind(po.points_redeemed)
        .bind(settings.redemption_rate)
        .bind(cents)
        .bind(&settings.currency)
        .bind(&status)
        .bind(&provider)
        .bind(&ref_id)
        .bind(&message)
        .execute(db)
        .await?;
    }

    // The run's own status tells the truth about what happened to the money.
    let run_status = if provider_key.is_none() {
        "pending_provider"
    } else if failed > 0 && paid == 0 {
        "failed"
    } else if paid > 0 && failed == 0 {
        "completed"
    } else if paid > 0 {
        "completed"
    } else {
        "pending_provider"
    };

    let run_message = if provider_key.is_none() {
        Some(format!(
            "no payment provider configured — {} payout(s) recorded as pending_provider, ledger is correct",
            payouts.len()
        ))
    } else if failed > 0 {
        Some(format!(
            "{paid} payout(s) sent, {failed} failed — see per-business messages"
        ))
    } else {
        None
    };

    sqlx::query(
        "UPDATE settlement_runs SET status = $2, total_points_issued = $3, total_points_redeemed = $4, \
            total_invoiced_cents = $5, total_payout_cents = $6, platform_spread_cents = $7, \
            provider = $8, provider_message = $9, completed_at = NOW() \
         WHERE id = $1",
    )
    .bind(run_id)
    .bind(run_status)
    .bind(pts_issued)
    .bind(pts_redeemed)
    .bind(invoiced)
    .bind(payout_total)
    .bind(invoiced - payout_total)
    .bind(&provider)
    .bind(&run_message)
    .execute(db)
    .await?;

    let run = sqlx::query_as::<_, RunRow>(&format!(
        "SELECT {RUN_COLS} FROM settlement_runs WHERE id = $1"
    ))
    .bind(run_id)
    .fetch_optional(db)
    .await?;

    Ok(SettlementRunOutcome {
        ok: true,
        already_existed: false,
        status: run_status.to_string(),
        message: run_message,
        run,
        invoices_created: invoices.len(),
        payouts_created: payouts.len(),
        payouts_paid: paid,
        payouts_failed: failed,
        provider,
        provider_configured: configured,
    })
}

/// A platform-level payout destination, if the admin stored one on the provider key
/// metadata. Absent is fine — the Stripe attempt then reports that honestly instead
/// of inventing a destination.
async fn platform_payout_destination(db: &PgPool, provider: Option<&str>) -> Option<String> {
    let p = provider?;
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT metadata->>'payout_destination' FROM provider_keys \
         WHERE provider = $1 AND is_active = true AND metadata->>'payout_destination' IS NOT NULL \
         ORDER BY is_default DESC, updated_at DESC LIMIT 1",
    )
    .bind(p)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// A real Stripe transfer attempt. Returns the provider's own reference on success
/// and the provider's own error text on failure — never a fabricated success.
async fn stripe_transfer(
    api_key: &str,
    amount_cents: Decimal,
    currency: &str,
    destination: Option<&str>,
    run_id: Uuid,
) -> Result<String, String> {
    let Some(dest) = destination else {
        return Err(
            "no payout destination configured (set metadata.payout_destination on the Stripe key)"
                .into(),
        );
    };

    let amount = amount_cents
        .to_i64()
        .ok_or_else(|| "payout amount out of range".to_string())?;

    let client = reqwest::Client::new();
    let res = client
        .post("https://api.stripe.com/v1/transfers")
        .bearer_auth(api_key)
        .form(&[
            ("amount".to_string(), amount.to_string()),
            ("currency".to_string(), currency.to_lowercase()),
            ("destination".to_string(), dest.to_string()),
            (
                "description".to_string(),
                format!("clearinghouse settlement {run_id}"),
            ),
        ])
        .send()
        .await
        .map_err(|e| format!("stripe request failed: {e}"))?;

    let status = res.status();
    let body: Value = res.json().await.unwrap_or_else(|_| json!({}));

    if status.is_success() {
        Ok(body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("stripe-ok")
            .to_string())
    } else {
        Err(body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("stripe returned HTTP {status}")))
    }
}

/// GET /api/v1/networks/:slug/settlement/runs
pub async fn settlement_runs(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<impl IntoResponse> {
    let network_id = resolve_network(&s.db, &slug).await?;
    let limit = q.limit.unwrap_or(24).clamp(1, 200);

    let runs = sqlx::query_as::<_, RunRow>(&format!(
        "SELECT {RUN_COLS} FROM settlement_runs WHERE network_id = $1 ORDER BY period_start DESC LIMIT $2"
    ))
    .bind(network_id)
    .bind(limit)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(
        json!({ "network_id": network_id, "count": runs.len(), "runs": runs }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StatementRow {
    pub run_id: Uuid,
    pub period_key: String,
    pub business_id: Option<Uuid>,
    pub business_name: Option<String>,
    pub points_issued: i64,
    pub invoiced_cents: Decimal,
    pub points_redeemed: i64,
    pub reimbursed_cents: Decimal,
    pub net_position_cents: Decimal,
    pub invoice_status: Option<String>,
    pub payout_status: Option<String>,
}

async fn statement_rows(db: &PgPool, run_id: Uuid) -> Result<Vec<StatementRow>, AppError> {
    let rows = sqlx::query_as::<_, StatementRow>(
        "SELECT run_id, period_key, business_id, business_name, points_issued, invoiced_cents, \
                points_redeemed, reimbursed_cents, net_position_cents, invoice_status, payout_status \
         FROM settlement_statements WHERE run_id = $1 \
         ORDER BY net_position_cents DESC",
    )
    .bind(run_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// GET /api/v1/networks/:slug/settlement/runs/:run_id/statements
pub async fn settlement_statements(
    State(s): State<AppState>,
    Path((slug, run_id)): Path<(String, Uuid)>,
) -> ApiResult<impl IntoResponse> {
    let network_id = resolve_network(&s.db, &slug).await?;

    let run = sqlx::query_as::<_, RunRow>(&format!(
        "SELECT {RUN_COLS} FROM settlement_runs WHERE id = $1 AND network_id = $2"
    ))
    .bind(run_id)
    .bind(network_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Settlement run not found for this network".into()))?;

    let rows = statement_rows(&s.db, run_id).await?;

    Ok(Json(json!({
        "run": run,
        "count": rows.len(),
        "statements": rows,
    })))
}

/// GET /api/v1/networks/:slug/settlement/runs/:run_id/statements.csv
pub async fn settlement_statements_csv(
    State(s): State<AppState>,
    Path((slug, run_id)): Path<(String, Uuid)>,
) -> ApiResult<axum::response::Response> {
    let network_id = resolve_network(&s.db, &slug).await?;

    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM settlement_runs WHERE id = $1 AND network_id = $2")
            .bind(run_id)
            .bind(network_id)
            .fetch_optional(&s.db)
            .await?;
    if exists.is_none() {
        return Err(AppError::NotFound(
            "Settlement run not found for this network".into(),
        ));
    }

    let rows = statement_rows(&s.db, run_id).await?;
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "period",
        "business_id",
        "business_name",
        "points_issued",
        "invoiced_cents",
        "points_redeemed",
        "reimbursed_cents",
        "net_position_cents",
        "invoice_status",
        "payout_status",
    ])
    .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;

    for r in &rows {
        w.write_record(vec![
            r.period_key.clone(),
            r.business_id.map(|b| b.to_string()).unwrap_or_default(),
            r.business_name.clone().unwrap_or_default(),
            r.points_issued.to_string(),
            format!("{:.2}", r.invoiced_cents),
            r.points_redeemed.to_string(),
            format!("{:.2}", r.reimbursed_cents),
            format!("{:.2}", r.net_position_cents),
            r.invoice_status.clone().unwrap_or_default(),
            r.payout_status.clone().unwrap_or_default(),
        ])
        .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
    }

    let data = String::from_utf8(
        w.into_inner()
            .map_err(|e| AppError::Internal(format!("CSV flush error: {e}")))?,
    )
    .map_err(|e| AppError::Internal(format!("CSV encoding error: {e}")))?;

    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=settlement_{run_id}.csv"),
        )
        .body(axum::body::Body::from(data))
        .map_err(|e| AppError::Internal(format!("CSV response build error: {e}")))
}

// ─────────────────────────────────────────────────────────────────────────────
// Statements by email — through each directory's CONFIGURED provider
// ─────────────────────────────────────────────────────────────────────────────

/// Where the email service lives. That service is the component that owns
/// transports and credentials (`directory_email_settings` + `provider_keys`), so a
/// provider is never hardcoded here: we hand it a directory id and it resolves the
/// admin's own configuration at send time.
fn email_service_url() -> String {
    std::env::var("EMAIL_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3456".to_string())
}

#[derive(Debug, sqlx::FromRow)]
struct StatementRecipient {
    business_id: Option<Uuid>,
    business_name: Option<String>,
    email: Option<String>,
    points_issued: i64,
    points_redeemed: i64,
    invoiced_cents: Decimal,
    reimbursed_cents: Decimal,
    net_position_cents: Decimal,
}

#[derive(Debug, Serialize)]
pub struct StatementSendOutcome {
    pub run_id: Uuid,
    pub period_key: Option<String>,
    pub recipients: usize,
    pub sent: usize,
    pub skipped: usize,
    pub failed: usize,
    pub provider_configured: bool,
    pub message: String,
}

fn money(currency: &str, cents: &Decimal) -> String {
    format!("{currency} {:.2}", cents)
}

/// Build the statement a business actually receives: what it issued and owes, what
/// it redeemed and is owed, the net position, the platform spread on its issuance and
/// the resulting amount payable or receivable. Every figure is the run's own frozen
/// rate applied to the ledger — nothing is recomputed from today's settings.
fn statement_email(
    business_name: &str,
    period_key: &str,
    currency: &str,
    r: &StatementRecipient,
    rate_issue: Decimal,
    rate_redeem: Decimal,
) -> (String, String, String) {
    let invoiced = r.invoiced_cents;
    let reimbursed = r.reimbursed_cents;
    let net = r.net_position_cents;
    let spread = amount_cents(r.points_issued, rate_issue - rate_redeem);

    let (arrow, payable) = if net >= Decimal::ZERO {
        ("payable", money(currency, &net))
    } else {
        ("receivable", money(currency, &net.abs()))
    };

    let subject = format!("Your {period_key} ZaarCash statement — {business_name}");

    let text = format!(
        "ZaarCash clearinghouse statement\n\
         Business: {business_name}\n\
         Period: {period_key}\n\n\
         Points issued: {issued} — invoiced {inv}\n\
         Points redeemed: {redeemed} — reimbursed {reimb}\n\
         Net position: {net_m} ({arrow})\n\
         Platform spread on your issuance ({issue_rate}/{redeem_rate} per point): {spread}\n\n\
         Amount {arrow}: {payable}\n",
        issued = r.points_issued,
        inv = money(currency, &invoiced),
        redeemed = r.points_redeemed,
        reimb = money(currency, &reimbursed),
        net_m = money(currency, &net),
        spread = money(currency, &spread),
        issue_rate = format!("{rate_issue:.4}"),
        redeem_rate = format!("{rate_redeem:.4}"),
    );

    let html = format!(
        "<h2>ZaarCash clearinghouse statement</h2>\
         <p><strong>{business_name}</strong><br>Period <strong>{period_key}</strong></p>\
         <table cellpadding=\"6\" style=\"border-collapse:collapse\">\
           <tr><td>Points issued</td><td align=\"right\">{issued}</td><td align=\"right\">{inv}</td></tr>\
           <tr><td>Points redeemed</td><td align=\"right\">{redeemed}</td><td align=\"right\">{reimb}</td></tr>\
           <tr><td><strong>Net position</strong></td><td></td><td align=\"right\"><strong>{net_m}</strong></td></tr>\
           <tr><td>Platform spread on your issuance</td><td align=\"right\">{issue_rate}/{redeem_rate}</td><td align=\"right\">{spread}</td></tr>\
         </table>\
         <p>Amount <strong>{arrow}</strong>: <strong>{payable}</strong></p>\
         <p style=\"color:#666;font-size:12px\">Generated by the Multi-Directory clearinghouse. Rates are those frozen on the settlement run for this period.</p>",
        issued = r.points_issued,
        inv = money(currency, &invoiced),
        redeemed = r.points_redeemed,
        reimb = money(currency, &reimbursed),
        net_m = money(currency, &net),
        spread = money(currency, &spread),
        issue_rate = format!("{rate_issue:.4}"),
        redeem_rate = format!("{rate_redeem:.4}"),
    );

    (subject, html, text)
}

async fn statement_recipients(
    db: &PgPool,
    run_id: Uuid,
) -> Result<Vec<StatementRecipient>, AppError> {
    let rows = sqlx::query_as::<_, StatementRecipient>(
        "SELECT s.business_id, \
                COALESCE(s.business_name, 'Business') AS business_name, \
                b.email, \
                s.points_issued, s.points_redeemed, \
                s.invoiced_cents, s.reimbursed_cents, s.net_position_cents \
         FROM settlement_statements s \
         LEFT JOIN businesses b ON b.id = s.business_id \
         WHERE s.run_id = $1 \
         ORDER BY s.net_position_cents DESC",
    )
    .bind(run_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// A directory in this network whose email transport is actually usable. Each
/// directory configures its own provider in the panel; the network inherits the
/// first one that is configured, and when none is, statements are skipped (logged,
/// never faked).
async fn statement_sending_directory(
    db: &PgPool,
    network_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    let dir: Option<Uuid> = sqlx::query_scalar(
        "SELECT des.directory_id FROM directories d \
         JOIN directory_email_settings des ON des.directory_id = d.id \
         WHERE d.network_id = $1 \
           AND (des.transport <> 'smtp' OR COALESCE(des.smtp_host, '') <> '') \
         ORDER BY des.updated_at DESC NULLS LAST LIMIT 1",
    )
    .bind(network_id)
    .fetch_optional(db)
    .await?;
    Ok(dir)
}

/// Email every affected business its statement for a run, then record on the run
/// exactly what happened (sent / skipped plus the reason).
///
/// Graceful by design: a directory with no provider configured skips its recipients
/// and says so; one unreachable transport fails that recipient only. The run and the
/// ledger are untouched either way — nothing is ever reported as sent that was not.
pub async fn send_statements(
    db: &PgPool,
    network_id: Uuid,
    run_id: Uuid,
) -> Result<StatementSendOutcome, AppError> {
    let run: Option<(String, Decimal, Decimal, String)> = sqlx::query_as(
        "SELECT period_key, rate_issue_per_point, rate_redeem_per_point, currency \
         FROM settlement_runs WHERE id = $1 AND network_id = $2",
    )
    .bind(run_id)
    .bind(network_id)
    .fetch_optional(db)
    .await?;

    let (period_key, rate_issue, rate_redeem, currency) =
        run.ok_or_else(|| AppError::NotFound("Settlement run not found for this network".into()))?;

    let recipients = statement_recipients(db, run_id).await?;
    let total = recipients.len();

    let directory_id = statement_sending_directory(db, network_id).await?;

    let (sent, skipped, failed, message, configured) = match directory_id {
        None => {
            let message = format!(
                "no email provider configured for any directory in this network — {total} statement(s) not sent (save an Email/SMTP transport for one of this network's directories in the panel)"
            );
            eprintln!("[settlement] run {run_id}: {message}");
            (0usize, total, 0usize, message, false)
        }
        Some(directory_id) => {
            let client = reqwest::Client::new();
            let url = format!("{}/send-email", email_service_url());
            let mut sent = 0usize;
            let mut skipped = 0usize;
            let mut failed = 0usize;

            for r in &recipients {
                let name = r.business_name.clone().unwrap_or_else(|| "Business".into());
                let to = r.email.clone().unwrap_or_default();
                if to.trim().is_empty() {
                    skipped += 1;
                    continue;
                }
                let (subject, html, text) =
                    statement_email(&name, &period_key, &currency, r, rate_issue, rate_redeem);

                let resp = client
                    .post(&url)
                    .json(&json!({
                        "directory_id": directory_id,
                        "to": to.trim(),
                        "subject": subject,
                        "html": html,
                        "text": text,
                    }))
                    .send()
                    .await;

                match resp {
                    Ok(res) => {
                        let raw = res.text().await.unwrap_or_default();
                        let parsed: Value = serde_json::from_str(&raw)
                            .unwrap_or_else(|_| json!({ "status": "error", "detail": raw }));
                        match parsed
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("error")
                        {
                            "sent" => sent += 1,
                            "skipped" => {
                                skipped += 1;
                                eprintln!(
                                    "[settlement] statement for {name} skipped: {}",
                                    parsed
                                        .get("detail")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unconfigured")
                                );
                            }
                            _ => {
                                failed += 1;
                                eprintln!(
                                    "[settlement] statement for {name} failed: {}",
                                    parsed
                                        .get("detail")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                );
                            }
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("[settlement] statement for {name} failed: email service unreachable: {e}");
                    }
                }
            }

            let message = format!(
                "{sent} statement(s) sent, {skipped} skipped, {failed} failed of {total} affected businesses"
            );
            (sent, skipped, failed, message, true)
        }
    };

    // Record delivery honestly on the run — the panel reads these four columns.
    sqlx::query(
        "UPDATE settlement_runs SET statements_sent_at = NOW(), statements_sent = $2, \
            statements_skipped = $3, statements_message = $4 WHERE id = $1",
    )
    .bind(run_id)
    .bind(sent as i32)
    .bind(skipped as i32)
    .bind(&message)
    .execute(db)
    .await?;

    Ok(StatementSendOutcome {
        run_id,
        period_key: Some(period_key),
        recipients: total,
        sent,
        skipped,
        failed,
        provider_configured: configured,
        message,
    })
}

/// POST /api/v1/networks/:slug/settlement/send-statements {run_id?}
///
/// "Send statements now" in the Settlement card. With no `run_id` it sends for the
/// most recent run of the network.
#[derive(Debug, Deserialize, Default)]
pub struct SendStatementsRequest {
    pub run_id: Option<Uuid>,
}

pub async fn send_statements_now(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Extension(claims): Extension<Claims>,
    body: Option<Json<SendStatementsRequest>>,
) -> ApiResult<impl IntoResponse> {
    if claims.role != "admin" && claims.role != "super_admin" {
        return Err(AppError::Forbidden(
            "Admin role required to send settlement statements".into(),
        ));
    }

    let network_id = resolve_network(&s.db, &slug).await?;
    let requested = body.and_then(|Json(b)| b.run_id);

    let run_id: Option<Uuid> = match requested {
        Some(id) => Some(id),
        None => {
            sqlx::query_scalar(
                "SELECT id FROM settlement_runs WHERE network_id = $1 \
                 ORDER BY period_start DESC, created_at DESC LIMIT 1",
            )
            .bind(network_id)
            .fetch_optional(&s.db)
            .await?
        }
    };

    let Some(run_id) = run_id else {
        return Err(AppError::BadRequest(
            "no settlement run exists for this network yet — run settlement first".into(),
        ));
    };

    let outcome = send_statements(&s.db, network_id, run_id).await?;
    Ok(Json(json!({ "ok": true, "result": outcome })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Schedule: the money loop closes by itself
// ─────────────────────────────────────────────────────────────────────────────

/// The next date this network's cycle day comes around (today counts). `cycle_day`
/// is clamped to 1..28 like the settings form, so every month has that day.
fn next_cycle_date(cycle_day: i32, today: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let day = cycle_day.clamp(1, 28) as u32;
    let (y, m) = (today.year(), today.month());
    match chrono::NaiveDate::from_ymd_opt(y, m, day) {
        Some(d) if d >= today => d,
        _ => {
            let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            chrono::NaiveDate::from_ymd_opt(ny, nm, day).unwrap_or(today)
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct LastRunRow {
    id: Uuid,
    period_key: String,
    period_start: chrono::NaiveDate,
    period_end: chrono::NaiveDate,
    status: String,
    triggered_by: String,
    created_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    statements_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    statements_sent: i32,
    statements_skipped: i32,
    statements_message: Option<String>,
}

/// GET /api/v1/networks/:slug/settlement/schedule
///
/// Everything the Settlement card needs to show that the loop is closed: whether the
/// scheduler will bill this network, on which day, when it last ran, when it runs
/// next, and the state of the expiry policy. All read from the DB — no constants.
pub async fn settlement_schedule(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let network_id = resolve_network(&s.db, &slug).await?;
    let settings = get_settings(&s.db, network_id).await?;

    let last_run: Option<LastRunRow> = sqlx::query_as::<_, LastRunRow>(
        "SELECT id, period_key, period_start, period_end, status, triggered_by, created_at, \
                completed_at, statements_sent_at, statements_sent, statements_skipped, statements_message \
         FROM settlement_runs WHERE network_id = $1 \
         ORDER BY period_start DESC, created_at DESC LIMIT 1",
    )
    .bind(network_id)
    .fetch_optional(&s.db)
    .await?;

    let today = chrono::Utc::now().date_naive();

    // "Next scheduled run" must be the next *unsettled* cycle date: if today is the
    // cycle day but the period already settled (the common case right after a run),
    // showing today would be a lie.
    let mut next_run = next_cycle_date(settings.cycle_day, today);
    let mut next_period = previous_month(next_run, settings.cycle_day);
    for _ in 0..3 {
        let settled: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM settlement_runs \
             WHERE network_id = $1 AND period_start = $2 AND period_end = $3",
        )
        .bind(network_id)
        .bind(next_period.0)
        .bind(next_period.1)
        .fetch_optional(&s.db)
        .await?;
        if settled.is_none() {
            break;
        }
        next_run = next_cycle_date(settings.cycle_day, next_run + chrono::Duration::days(1));
        next_period = previous_month(next_run, settings.cycle_day);
    }
    let (next_start, next_end) = next_period;

    let expiry_last_run_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT expiry_last_run_at FROM point_treasury WHERE network_id = $1")
            .bind(network_id)
            .fetch_optional(&s.db)
            .await?
            .flatten();

    Ok(Json(json!({
        "network_id": network_id,
        "enabled": settings.settlement_enabled,
        "cycle_day": settings.cycle_day,
        "scheduler": {
            "kind": "hourly",
            "interval_seconds": SETTLEMENT_SCHEDULER_INTERVAL_SECS,
            "runs_when": "today's day-of-month equals cycle_day and the period is not already settled",
        },
        "last_run": last_run,
        "next_run_at": next_run.to_string(),
        "next_period": {
            "start": next_start.to_string(),
            "end": next_end.to_string(),
            "key": period_key(next_start),
        },
        "expiry": {
            "policy_days": settings.default_expiry_days,
            "last_run_at": expiry_last_run_at,
        },
        "currency": settings.currency,
    })))
}

/// How often the settlement scheduler wakes. Hourly: the work is at most once a
/// month per network, and the wake itself is one cheap indexed query.
pub const SETTLEMENT_SCHEDULER_INTERVAL_SECS: u64 = 3600;

/// The thing that was missing: a background loop that settles each network on its
/// configured `cycle_day` when `settlement_enabled` is true, then emails statements
/// and applies the configured point-expiry policy.
///
/// It is deliberately dumb about *when*: it re-evaluates hourly and each period can
/// only settle once (database-enforced), so a restart, a double wake-up or a manual
/// run beside it cannot double-bill. Networks that are disabled, or whose cycle day
/// is not today, are skipped silently.
pub fn start_settlement_scheduler(db: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
            SETTLEMENT_SCHEDULER_INTERVAL_SECS,
        ));
        loop {
            interval.tick().await;

            let today = chrono::Utc::now().date_naive();
            let today_dom = {
                use chrono::Datelike;
                today.day() as i32
            };

            let due = sqlx::query_as::<_, (Uuid, i32, i32)>(
                "SELECT network_id, cycle_day, default_expiry_days FROM point_treasury \
                 WHERE settlement_enabled = true AND cycle_day = $1",
            )
            .bind(today_dom)
            .fetch_all(&db)
            .await;

            let due = match due {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(
                        "[settlement] scheduler could not read point_treasury: {}",
                        e
                    );
                    continue;
                }
            };

            for (network_id, cycle_day, expiry_days) in due {
                let (start, end) = previous_month(today, cycle_day);

                match execute_settlement(&db, network_id, start, end, None, None, "scheduler").await
                {
                    Ok(o) if o.already_existed => {
                        tracing::info!(
                            "[settlement] {} already settled for network {} (no-op)",
                            o.run
                                .as_ref()
                                .map(|r| r.period_key.clone())
                                .unwrap_or_default(),
                            network_id
                        );
                    }
                    Ok(o) => {
                        tracing::info!(
                            "[settlement] scheduled run {} for network {}: {} invoices, {} payouts ({})",
                            o.run.as_ref().map(|r| r.period_key.clone()).unwrap_or_default(),
                            network_id,
                            o.invoices_created,
                            o.payouts_created,
                            o.status
                        );
                        if let Some(run) = o.run.as_ref() {
                            match send_statements(&db, network_id, run.id).await {
                                Ok(s) => tracing::info!(
                                    "[settlement] statements for run {}: {}",
                                    run.id,
                                    s.message
                                ),
                                Err(e) => {
                                    tracing::warn!("[settlement] statement send failed: {}", e)
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[settlement] scheduled run for network {} failed: {}",
                            network_id,
                            e
                        );
                    }
                }

                // The expiry policy rides the same cycle: it is idempotent, so an
                // extra pass on this day removes nothing.
                match crate::handlers::clearinghouse::expire_points_for_network(
                    &db,
                    network_id,
                    expiry_days,
                )
                .await
                {
                    Ok(x) => tracing::info!(
                        "[settlement] expiry pass for network {} (policy {}d): {} points expired across {} member(s)",
                        network_id,
                        x.expiry_days,
                        x.total_points_expired,
                        x.members_affected
                    ),
                    Err(e) => tracing::warn!("[settlement] expiry pass failed: {}", e),
                }
            }
        }
    });
}
