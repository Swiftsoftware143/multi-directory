//! Native Clearinghouse engine for Multi-Directory — network-wide loyalty economics.
//!
//! Brings the IncentiveSwift clearinghouse natively into Multi-Directory, re-keyed
//! to NETWORK scope: ZaarCash is a single universal wallet across every city directory
//! in the network (earn in Palm Bay, redeem in St. Pete).
//!
//! Economics (clearinghouse / buy-in + reimbursement):
//!   - 1 point = $0.01 to the consumer.
//!   - Issuing business is billed $0.01 per point issued (buy-in).
//!   - Redeeming business is reimbursed $0.008 per point redeemed.
//!   - Platform keeps the $0.002 slippage (spread) for fees + breakage.
//!   - Guardrails: per-category redemption caps (e.g. 20% at grocery), rolling expiry.

use crate::error::AppError;
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sqlx::PgPool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Network resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the network_id a directory belongs to. Every directory is mapped onto
/// a network; loyalty is network-wide. Returns None only if the directory has no
/// network (falls back to directory-scoped behavior).
async fn directory_network(db: &PgPool, directory_id: Uuid) -> Result<Option<Uuid>, AppError> {
    let net: Option<Uuid> = sqlx::query_scalar(
        "SELECT network_id FROM directories WHERE id = $1",
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error resolving network: {e}");
        AppError::Database(e)
    })?
    .flatten();
    Ok(net)
}

/// Find the network-wide loyalty program for a network. If none exists, create one
/// lazily so a network is always demoable end-to-end.
async fn find_or_create_network_program(
    db: &PgPool,
    network_id: Uuid,
) -> Result<Uuid, AppError> {
    // A program is network-wide when network_id IS NOT NULL.
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM loyalty_programs WHERE network_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(network_id)
    .fetch_optional(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error finding network program: {e}");
        AppError::Database(e)
    })?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let net_name: String = sqlx::query_scalar("SELECT name FROM networks WHERE id = $1")
        .bind(network_id)
        .fetch_one(db)
        .await
        .unwrap_or_else(|_| "Network".to_string());

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_programs (id, name, network_id, recognition_method, currency_name, currency_icon, currency_color, points_per_checkin, max_checkins_per_day, points_expire_days, tiers_enabled, is_active)
         VALUES ($1, $2, $3, 'qr_scan', 'ZaarCash', '💎', '#0ea5e9', 10, 10, 365, true, true)",
    )
    .bind(id)
    .bind(format!("{net_name} Loyalty"))
    .bind(network_id)
    .execute(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error creating network program: {e}");
        AppError::Database(e)
    })?;

    Ok(id)
}

// ─────────────────────────────────────────────────────────────────────────────
// Network-wide member wallet
// ─────────────────────────────────────────────────────────────────────────────

/// Find or create the network-scoped member for a visitor. A visitor has exactly
/// one wallet in a network program, shared across all cities.
async fn find_or_create_network_member(
    db: &PgPool,
    program_id: Uuid,
    visitor_account_id: Uuid,
    network_id: Uuid,
) -> Result<Uuid, AppError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM loyalty_members WHERE program_id = $1 AND visitor_account_id = $2 AND network_id = $3",
    )
    .bind(program_id)
    .bind(visitor_account_id)
    .bind(network_id)
    .fetch_optional(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error finding member: {e}");
        AppError::Database(e)
    })?;
    if let Some(id) = existing {
        // ensure network_id is backfilled so the wallet is network-wide
        sqlx::query("UPDATE loyalty_members SET network_id = $1 WHERE id = $2 AND network_id IS NULL")
            .bind(network_id)
            .bind(id)
            .execute(db)
            .await
            .ok();
        return Ok(id);
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_members (id, program_id, visitor_account_id, network_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(program_id)
    .bind(visitor_account_id)
    .bind(network_id)
    .execute(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error creating member: {e}");
        AppError::Database(e)
    })?;
    Ok(id)
}

// ─────────────────────────────────────────────────────────────────────────────
// Treasury helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn ensure_treasury(db: &PgPool, network_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO point_treasury (network_id) VALUES ($1)
         ON CONFLICT (network_id) DO NOTHING",
    )
    .bind(network_id)
    .execute(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error ensuring treasury: {e}");
        AppError::Database(e)
    })?;
    Ok(())
}

async fn update_business_ledger(
    db: &PgPool,
    network_id: Uuid,
    business_id: Uuid,
    business_name: &str,
    points_issued: i32,
    points_redeemed: i32,
) -> Result<(), AppError> {
    // Billed = points issued x $0.01; Reimbursed = points redeemed x $0.008
    let billed = Decimal::new((points_issued * 1) as i64, 2);        // $x.xx
    let reimbursed = Decimal::new((points_redeemed * 8) as i64, 3); // $x.xxx

    sqlx::query(
        r#"INSERT INTO business_point_ledger (network_id, business_id, business_name, month_key,
                points_issued_this_month, points_redeemed_this_month,
                total_billed_this_month, total_reimbursed_this_month, net_position)
           VALUES ($1,$2,$3, TO_CHAR(NOW(),'YYYY-MM'), $4,$5,$6,$7, $8)
           ON CONFLICT (network_id, business_id, month_key) DO UPDATE SET
                points_issued_this_month = business_point_ledger.points_issued_this_month + $4,
                points_redeemed_this_month = business_point_ledger.points_redeemed_this_month + $5,
                total_billed_this_month = business_point_ledger.total_billed_this_month + $6,
                total_reimbursed_this_month = business_point_ledger.total_reimbursed_this_month + $7,
                net_position = (business_point_ledger.total_reimbursed_this_month + $7)
                             - (business_point_ledger.total_billed_this_month + $6),
                updated_at = NOW()"#,
    )
    .bind(network_id)
    .bind(business_id)
    .bind(business_name)
    .bind(points_issued as i64)
    .bind(points_redeemed as i64)
    .bind(billed)
    .bind(reimbursed)
    .bind(Decimal::new((points_redeemed * 8 - points_issued) as i64, 3)) // fresh net_position
    .execute(db)
    .await
    .map_err(|e| {
        eprintln!("[clearinghouse] error updating ledger: {e}");
        AppError::Database(e)
    })?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Network-wide scan (issue or redeem)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CssScanRequest {
    pub visitor_account_id: Uuid,
    pub business_id: Uuid,
    pub business_name: Option<String>,
    pub business_category: Option<String>,
    pub scan_type: String,             // purchase | redeem | checkin | visit
    pub transaction_amount: Option<Decimal>,
    pub points: Option<i32>,           // for manual redeem; required for redeem
    pub transaction_id: Option<String>,
}

/// POST /api/v1/networks/:slug/clear/scan
pub async fn clearhouse_scan(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CssScanRequest>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let network_id = directory_network(&state.db, directory_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Directory is not part of a network".into()))?;
    ensure_treasury(&state.db, network_id).await?;

    let program_id = find_or_create_network_program(&state.db, network_id).await?;
    let member_id = find_or_create_network_member(&state.db, program_id, req.visitor_account_id, network_id).await?;

    let business_name = req.business_name.clone().unwrap_or_else(|| "Business".to_string());

    match req.scan_type.as_str() {
        "redeem" => {
            let points = req.points.ok_or_else(|| {
                AppError::BadRequest("points required for redemption".into())
            })?;
            redeem(&state, network_id, program_id, member_id, &req, business_name, points).await
        }
        _ => {
            // purchase / checkin / visit => issuance (bill issuing business)
            issue(&state, network_id, program_id, member_id, &req, business_name).await
        }
    }
}

async fn issue(
    state: &AppState,
    network_id: Uuid,
    program_id: Uuid,
    member_id: Uuid,
    req: &CssScanRequest,
    business_name: String,
) -> Result<Json<Value>, AppError> {
    // Points = 1 per dollar spent (default), truncated down.
    let amount = req.transaction_amount.unwrap_or(Decimal::ZERO);
    let points = amount.trunc().to_i64().unwrap_or(0).max(0) as i32;

    let sqlx = &state.db;
    // Update member balance + lifetime
    sqlx::query(
        "UPDATE loyalty_members SET points_balance = points_balance + $1,
                lifetime_points = lifetime_points + $1, last_activity_date = NOW() WHERE id = $2",
    )
    .bind(points)
    .bind(member_id)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    let total_billed_cents = points * 1; // $0.01/pt

    // Issuance log
    sqlx::query(
        "INSERT INTO point_issuance_log (network_id, issuing_business_id, business_name, member_id, program_id,
                points_issued, bill_rate_cents, total_billed_cents, transaction_amount, transaction_id, issuance_type)
         VALUES ($1,$2,$3,$4,$5,$6,1,$7,$8,$9,$10)",
    )
    .bind(network_id)
    .bind(req.business_id)
    .bind(&business_name)
    .bind(member_id)
    .bind(program_id)
    .bind(points)
    .bind(total_billed_cents)
    .bind(req.transaction_amount)
    .bind(&req.transaction_id)
    .bind(&req.scan_type)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Treasury
    sqlx::query(
        "UPDATE point_treasury SET total_points_issued = total_points_issued + $1,
                total_revenue_collected = total_revenue_collected + $2,
                outstanding_liability = outstanding_liability + $2, updated_at = NOW() WHERE network_id = $3",
    )
    .bind(points as i64)
    .bind(Decimal::new(total_billed_cents as i64, 2))
    .bind(network_id)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    update_business_ledger(sqlx, network_id, req.business_id, &business_name, points, 0).await?;

    // Record scan
    let scan_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO loyalty_scans (id, member_id, program_id, business_id, business_name, scan_type,
                points_awarded, points_balance, transaction_amount, business_category, clearinghouse_processed)
           VALUES ($1,$2,$3, (SELECT id FROM businesses WHERE id = $4), $5,$6,$7, COALESCE((SELECT points_balance FROM loyalty_members WHERE id=$2),0), $8,$9, true)"#,
    )
    .bind(scan_id)
    .bind(member_id)
    .bind(program_id)
    .bind(req.business_id)
    .bind(&business_name)
    .bind(&req.scan_type)
    .bind(points)
    .bind(req.transaction_amount)
    .bind(&req.business_category)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Activity
    sqlx::query(
        "INSERT INTO loyalty_activity (member_id, activity_type, description, points_earned)
         VALUES ($1, 'purchase', $2, $3)",
    )
    .bind(member_id)
    .bind(format!("Earned {points} {} at {business_name}", "ZaarCash"))
    .bind(points)
    .execute(sqlx)
    .await
    .ok();

    Ok(Json(json!({
        "status": "issued",
        "member_id": member_id,
        "program_id": program_id,
        "points_awarded": points,
        "billed_cents": total_billed_cents,
        "business": business_name,
        "currency": "ZaarCash"
    })))
}

async fn redeem(
    state: &AppState,
    network_id: Uuid,
    program_id: Uuid,
    member_id: Uuid,
    req: &CssScanRequest,
    business_name: String,
    points: i32,
) -> Result<Json<Value>, AppError> {
    let sqlx = &state.db;

    // Check balance
    let balance: i32 = sqlx::query_scalar("SELECT points_balance FROM loyalty_members WHERE id = $1")
        .bind(member_id)
        .fetch_one(sqlx)
        .await
        .map_err(|e| AppError::Database(e))?;
    if balance < points {
        return Err(AppError::BadRequest(
            format!("Insufficient points: have {balance}, need {points}"),
        ));
    }

    // Category cap
    let mut max_pct = 100;
    if let Some(cat) = &req.business_category {
        let cap: Option<i32> = sqlx::query_scalar(
            "SELECT max_redeem_percent FROM category_redeem_caps WHERE network_id = $1 AND category_name = $2",
        )
        .bind(network_id)
        .bind(cat)
        .fetch_optional(sqlx)
        .await
        .map_err(|e| AppError::Database(e))?;
        max_pct = cap.unwrap_or(100);
    }

    // Enforce cap vs transaction amount. 1 point = 1 cent ($0.01).
    // points_value_cents must be <= max_pct% of the invoice amount in cents.
    if let Some(amount) = req.transaction_amount {
        let amount_cents = (amount * Decimal::ONE_HUNDRED).floor();       // invoice in cents
        let points_value_cents = Decimal::from(points);               // 1 pt = 1 cent
        let max_allowed_cents = (amount_cents * Decimal::from(max_pct)) / Decimal::ONE_HUNDRED;
        if points_value_cents > max_allowed_cents && max_pct < 100 {
            return Err(AppError::BadRequest(format!(
                "Redemption capped at {max_pct}% of invoice for this category"
            )));
        }
    }

    // Deduct
    sqlx::query(
        "UPDATE loyalty_members SET points_balance = points_balance - $1, last_activity_date = NOW() WHERE id = $2",
    )
    .bind(points)
    .bind(member_id)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    let reimbursement_cents = points * 8 / 10; // 0.8c/pt (÷10 for 0.8)
    let reimbursement = Decimal::new(reimbursement_cents as i64, 2);

    // Redemption log
    sqlx::query(
        "INSERT INTO point_redemption_log (network_id, redeeming_business_id, business_name, member_id, program_id,
                points_redeemed, reimbursement_rate_cents, total_reimbursement_cents, transaction_amount, max_redeem_percent, transaction_id)
         VALUES ($1,$2,$3,$4,$5,$6,8,$7,$8,$9,$10)",
    )
    .bind(network_id)
    .bind(req.business_id)
    .bind(&business_name)
    .bind(member_id)
    .bind(program_id)
    .bind(points)
    .bind(reimbursement_cents)
    .bind(req.transaction_amount)
    .bind(max_pct)
    .bind(&req.transaction_id)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Treasury
    sqlx::query(
        "UPDATE point_treasury SET total_points_redeemed = total_points_redeemed + $1,
                total_reimbursements_paid = total_reimbursements_paid + $2,
                outstanding_liability = GREATEST(0, outstanding_liability - $3), updated_at = NOW() WHERE network_id = $4",
    )
    .bind(points as i64)
    .bind(reimbursement)
    .bind(Decimal::new(points as i64, 2))
    .bind(network_id)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    update_business_ledger(sqlx, network_id, req.business_id, &business_name, 0, points).await?;

    // Scan
    let scan_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO loyalty_scans (id, member_id, program_id, business_id, business_name, scan_type,
                points_awarded, points_balance, transaction_amount, business_category, clearinghouse_processed)
           VALUES ($1,$2,$3, (SELECT id FROM businesses WHERE id = $4), $5,'redemption',-$6, COALESCE((SELECT points_balance FROM loyalty_members WHERE id=$2),0), $7,$8, true)"#,
    )
    .bind(scan_id)
    .bind(member_id)
    .bind(program_id)
    .bind(req.business_id)
    .bind(&business_name)
    .bind(points)
    .bind(req.transaction_amount)
    .bind(&req.business_category)
    .execute(sqlx)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Activity
    sqlx::query(
        "INSERT INTO loyalty_activity (member_id, activity_type, description, points_earned)
         VALUES ($1, 'redemption', $2, -$3)",
    )
    .bind(member_id)
    .bind(format!("Redeemed {points} {} at {business_name}", "ZaarCash"))
    .bind(points)
    .execute(sqlx)
    .await
    .ok();

    Ok(Json(json!({
        "status": "redeemed",
        "member_id": member_id,
        "points_redeemed": points,
        "reimbursed_cents": reimbursement_cents,
        "category_cap": max_pct,
        "business": business_name,
        "currency": "ZaarCash"
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// Treasury + config endpoints
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/networks/:slug/clear/treasury
pub async fn treasury_summary(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let network_id = directory_network(&state.db, directory_id).await?;
    let Some(network_id) = network_id else {
        return Ok(Json(json!({ "error": "Directory not part of a network" })));
    };
    ensure_treasury(&state.db, network_id).await?;

    let row = sqlx::query_as::<_, (i64, i64, Decimal, Decimal, Decimal, Decimal)>(
        "SELECT COALESCE(total_points_issued,0), COALESCE(total_points_redeemed,0),
                COALESCE(total_revenue_collected,0), COALESCE(total_reimbursements_paid,0),
                COALESCE(outstanding_liability,0), COALESCE(minimum_float,0)
         FROM point_treasury WHERE network_id = $1",
    )
    .bind(network_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(Json(json!({
        "network_id": network_id,
        "total_points_issued": row.0,
        "total_points_redeemed": row.1,
        "revenue_collected": format!("{:.2}", row.2),
        "reimbursements_paid": format!("{:.2}", row.3),
        "outstanding_liability": format!("{:.2}", row.4),
        "minimum_float": format!("{:.2}", row.5),
        "issuance_rate": "0.01",
        "redemption_rate": "0.008",
        "platform_spread_percent": "20.00"
    })))
}

/// GET /api/v1/networks/:slug/clear/ledgers
pub async fn business_ledgers(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let network_id = directory_network(&state.db, directory_id).await?;
    let Some(network_id) = network_id else {
        return Ok(Json(json!({ "ledgers": [] })));
    };

    let rows: Vec<(Uuid, Option<String>, i64, i64, Decimal, Decimal, Decimal, String)> =
        sqlx::query_as(
            "SELECT business_id, business_name, points_issued_this_month, points_redeemed_this_month,
                    total_billed_this_month, total_reimbursed_this_month, net_position, month_key
             FROM business_point_ledger WHERE network_id = $1 AND month_key = TO_CHAR(NOW(),'YYYY-MM')
             ORDER BY total_billed_this_month DESC",
        )
        .bind(network_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    let items: Vec<Value> = rows
        .into_iter()
        .map(|(bid, name, pi, pr, tb, tr, np, mk)| {
            json!({
                "business_id": bid,
                "business_name": name,
                "points_issued": pi,
                "points_redeemed": pr,
                "billed": format!("{:.2}", tb),
                "reimbursed": format!("{:.2}", tr),
                "net_position": format!("{:.2}", np),
                "month": mk
            })
        })
        .collect();

    Ok(Json(json!({ "ledgers": items, "count": items.len() })))
}

/// GET /api/v1/networks/:slug/clear/caps
pub async fn category_caps(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let network_id = directory_network(&state.db, directory_id).await?;
    let Some(network_id) = network_id else {
        return Ok(Json(json!({ "caps": [] })));
    };

    let rows: Vec<(String, i32, Option<String>)> = sqlx::query_as(
        "SELECT category_name, max_redeem_percent, description FROM category_redeem_caps WHERE network_id = $1 ORDER BY category_name",
    )
    .bind(network_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let items: Vec<Value> = rows
        .into_iter()
        .map(|(c, p, d)| json!({ "category": c, "max_redeem_percent": p, "description": d }))
        .collect();

    Ok(Json(json!({ "caps": items })))
}

/// PUT /api/v1/networks/:slug/clear/caps  (body: { category, max_redeem_percent, description })
#[derive(Debug, Deserialize)]
pub struct UpsertCap {
    pub category: String,
    pub max_redeem_percent: i32,
    pub description: Option<String>,
}

pub async fn upsert_category_cap(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(req): Json<UpsertCap>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let network_id = directory_network(&state.db, directory_id).await?;
    let Some(network_id) = network_id else {
        return Err(AppError::BadRequest("Directory not part of a network".into()));
    };

    if req.max_redeem_percent < 0 || req.max_redeem_percent > 100 {
        return Err(AppError::BadRequest("Cap must be 0-100".into()));
    }

    sqlx::query(
        "INSERT INTO category_redeem_caps (network_id, category_name, max_redeem_percent, description)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (network_id, category_name) DO UPDATE SET max_redeem_percent = EXCLUDED.max_redeem_percent, description = EXCLUDED.description",
    )
    .bind(network_id)
    .bind(&req.category)
    .bind(req.max_redeem_percent)
    .bind(&req.description)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(Json(json!({ "success": true, "category": req.category, "max_redeem_percent": req.max_redeem_percent })))
}

/// POST /api/v1/networks/:slug/clear/expire   (rolling 12-month expiry)
pub async fn expire_points(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let network_id = directory_network(&state.db, directory_id).await?;
    let Some(network_id) = network_id else {
        return Ok(Json(json!({ "error": "Directory not part of a network" })));
    };

    let cutoff = chrono::Utc::now() - chrono::Duration::days(365);

    // Sum points older than cutoff from issuance logs
    let expired: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT member_id, SUM(points_issued)
         FROM point_issuance_log WHERE network_id = $1 AND created_at < $2 AND points_issued > 0
         GROUP BY member_id",
    )
    .bind(network_id)
    .bind(cutoff)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut total_expired: i64 = 0;
    let mut affected = 0;
    for (member_id, points) in expired {
        sqlx::query(
            "UPDATE loyalty_members SET points_balance = GREATEST(0, points_balance - $1) WHERE id = $2",
        )
        .bind(points as i32)
        .bind(member_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;
        sqlx::query(
            "UPDATE point_treasury SET outstanding_liability = GREATEST(0, outstanding_liability - $1), updated_at = NOW() WHERE network_id = $2",
        )
        .bind(Decimal::new(points as i64, 2))
        .bind(network_id)
        .execute(&state.db)
        .await
        .ok();
        total_expired += points;
        affected += 1;
    }

    Ok(Json(json!({
        "success": true,
        "total_points_expired": total_expired,
        "members_affected": affected,
        "cutoff": cutoff.to_rfc3339()
    })))
}

/// GET /api/v1/networks/:slug/clear/logs  (consolidated issuance + redemption log)
pub async fn clearing_logs(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let directory_id: Uuid = sqlx::query_scalar("SELECT id FROM directories WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;
    let network_id = directory_network(&state.db, directory_id).await?;
    let Some(network_id) = network_id else {
        return Ok(Json(json!({ "items": [], "count": 0 })));
    };
    let limit: i64 = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50);

    let issued: Vec<(Uuid, i32, i32, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT member_id, points_issued, total_billed_cents, business_name, created_at FROM point_issuance_log WHERE network_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(network_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let redeemed: Vec<(Uuid, i32, i32, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT member_id, points_redeemed, total_reimbursement_cents, business_name, created_at FROM point_redemption_log WHERE network_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(network_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut items: Vec<Value> = Vec::new();
    for (mid, pts, cents, name, ts) in issued {
        items.push(json!({ "type": "issue", "member_id": mid, "points": pts, "value": format!("{:.2}", cents), "business": name, "time": ts }));
    }
    for (mid, pts, cents, name, ts) in redeemed {
        items.push(json!({ "type": "redeem", "member_id": mid, "points": pts, "value": format!("{:.2}", cents), "business": name, "time": ts }));
    }
    items.sort_by(|a, b| b["time"].as_str().cmp(&a["time"].as_str()));

    Ok(Json(json!({ "items": items, "count": items.len() })))
}
