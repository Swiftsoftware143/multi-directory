//! Native Loyalty engine for Multi-Directory.
//!
//! Cloned from IncentiveSwift's loyalty engine, re-keyed to Multi-Directory entities:
//!   - tenant:   account_id  -> directory_id   (directories.id)
//!   - member:   contact_id  -> visitor_account_id (visitor_accounts.id)
//!   - business: business_id -> business_id    (businesses.id)
//!   - (no campaign_id / entry_id — directories have no campaigns)
//!
//! Every directory owns its loyalty programs; each directory admin creates and
//! configures their own programs. Members are consumer visitor accounts.

use crate::error::AppError;
use crate::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// 100 units of a programme currency = US$1. A platform-wide constant (deliberately NOT a column):
/// the admin panel renders every rate in dollars with it, and deals.rs settles bills with it.
pub const UNITS_PER_DOLLAR: f64 = 100.0;

// ─────────────────────────────────────────────────────────────────────────────
// Program config
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LoyaltyProgram {
    pub id: Uuid,
    /// NULL for a network-wide programme (the normal case: ZaarHub loyalty covers every city).
    pub directory_id: Option<Uuid>,
    /// Set when the programme is network-wide (ZaarHub is ONE directory across many cities).
    pub network_id: Option<Uuid>,
    pub name: String,
    pub recognition_method: String,
    pub points_per_checkin: i32,
    pub max_checkins_per_day: i32,
    pub point_decay_days: Option<i32>,
    pub points_expire_days: i32,
    pub currency_name: String,
    pub currency_icon: String,
    pub currency_color: String,
    pub points_per_visit: i32,
    pub points_per_redemption: i32,
    /// Currency units credited per $1 of earnable spend. 0 = earning disabled. Default 1.
    pub earn_rate: f64,
    /// Max % of a bill a member may settle with the currency (0-100). Default 10.
    pub redemption_cap_pct: i32,
    /// Balance required before a member may redeem. 100 units = $1, so 100 = $1. Default 100.
    pub min_redeem_balance: i32,
    /// Free / fully-discounted items earn nothing. Default on.
    pub exclude_free_items: bool,
    pub tiers_enabled: bool,
    pub milestones_enabled: bool,
    pub streak_enabled: bool,
    pub streak_bonus: i32,
    pub streak_days: i32,
    pub referral_bonus: i32,
    pub birthday_bonus: i32,
    pub social_share_points: i32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ProgramInput {
    pub name: String,
    pub recognition_method: Option<String>,
    pub points_per_checkin: Option<i32>,
    pub max_checkins_per_day: Option<i32>,
    pub point_decay_days: Option<i32>,
    pub points_expire_days: Option<i32>,
    pub currency_name: Option<String>,
    pub currency_icon: Option<String>,
    pub currency_color: Option<String>,
    pub points_per_visit: Option<i32>,
    /// Points credited when a member redeems a deal. 0 = disabled.
    pub points_per_redemption: Option<i32>,
    /// Currency units credited per $1 of earnable spend. 0 = disabled. Default 1.
    pub earn_rate: Option<f64>,
    /// Max % of a bill payable in the currency (clamped 0-100). Default 10.
    pub redemption_cap_pct: Option<i32>,
    /// Balance required before a member may redeem. 100 units = $1. Default 100.
    pub min_redeem_balance: Option<i32>,
    /// Free / fully-discounted items earn nothing. Default true.
    pub exclude_free_items: Option<bool>,
    pub tiers_enabled: Option<bool>,
    pub milestones_enabled: Option<bool>,
    pub streak_enabled: Option<bool>,
    pub streak_bonus: Option<i32>,
    pub streak_days: Option<i32>,
    pub referral_bonus: Option<i32>,
    pub birthday_bonus: Option<i32>,
    pub social_share_points: Option<i32>,
    pub is_active: Option<bool>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Members
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LoyaltyMember {
    pub id: Uuid,
    pub program_id: Uuid,
    pub visitor_account_id: Uuid,
    pub points_balance: i32,
    pub lifetime_points: i32,
    pub tier_id: Option<Uuid>,
    pub current_streak: i32,
    pub longest_streak: i32,
    pub last_activity_date: Option<chrono::DateTime<chrono::Utc>>,
    pub birthday: Option<chrono::NaiveDate>,
    pub referral_code: Option<String>,
    pub total_referrals: i32,
    pub qr_code: Option<String>,
    pub member_since: chrono::DateTime<chrono::Utc>,
    pub last_checkin_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Data-layer helpers (directory-scoped)
// ─────────────────────────────────────────────────────────────────────────────

pub async fn get_program(pool: &PgPool, program_id: &Uuid) -> Result<LoyaltyProgram, AppError> {
    let p = sqlx::query_as::<_, LoyaltyProgram>(
        r#"SELECT id, directory_id, network_id, name, recognition_method, points_per_checkin,
                  max_checkins_per_day, point_decay_days, points_expire_days,
                  currency_name, currency_icon, currency_color, points_per_visit, points_per_redemption,
                  earn_rate, redemption_cap_pct, min_redeem_balance, exclude_free_items,
                  tiers_enabled, milestones_enabled, streak_enabled, streak_bonus,
                  streak_days, referral_bonus, birthday_bonus, social_share_points,
                  is_active, created_at, updated_at
           FROM loyalty_programs WHERE id = $1 AND is_active = true"#,
    )
    .bind(program_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Loyalty program not found or not active".into()))?;

    Ok(p)
}

pub async fn resolve_directory_id(pool: &PgPool, slug: &str) -> Result<Uuid, AppError> {
    let id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM directories WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Directory '{slug}' not found")))?;
    Ok(id)
}

/// The programme that governs a directory. ZaarHub is ONE directory across ten city directories
/// with ONE network-wide programme, so the scope is the directory's *network*: nothing about
/// loyalty may depend on a city-scoped programme existing.
pub async fn programme_for_directory(
    pool: &PgPool,
    directory_id: &Uuid,
) -> Result<Option<LoyaltyProgram>, AppError> {
    let program = sqlx::query_as::<_, LoyaltyProgram>(
        r#"SELECT id, directory_id, network_id, name, recognition_method, points_per_checkin,
                  max_checkins_per_day, point_decay_days, points_expire_days,
                  currency_name, currency_icon, currency_color, points_per_visit, points_per_redemption,
                  earn_rate, redemption_cap_pct, min_redeem_balance, exclude_free_items,
                  tiers_enabled, milestones_enabled, streak_enabled, streak_bonus,
                  streak_days, referral_bonus, birthday_bonus, social_share_points,
                  is_active, created_at, updated_at
           FROM (SELECT *, (network_id IS NOT NULL) AS _network_first
                 FROM loyalty_programs
                 WHERE is_active
                   AND ((network_id IS NOT NULL
                         AND network_id = (SELECT network_id FROM directories WHERE id = $1))
                        OR (network_id IS NULL AND directory_id = $1))) picked
           ORDER BY _network_first DESC, created_at
           LIMIT 1"#,
    )
    .bind(directory_id)
    .fetch_optional(pool)
    .await?;

    Ok(program)
}

/// Native, network-scoped loyalty enrolment. Called when a visitor signs up: the visitor joins
/// the single network-wide programme (no per-city programme, no external service). Best-effort by
/// design — enrolment must never fail a signup.
pub async fn enroll_visitor_in_network_loyalty(pool: &PgPool, visitor_account_id: &Uuid) {
    let directory_id: Option<Uuid> =
        sqlx::query_scalar("SELECT directory_id FROM visitor_accounts WHERE id = $1")
            .bind(visitor_account_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    // 1) the programme for the visitor's city's network, 2) otherwise the network programme itself
    let program_id: Option<Uuid> = match directory_id {
        Some(dir) => match programme_for_directory(pool, &dir).await {
            Ok(Some(p)) => Some(p.id),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("[loyalty] programme lookup failed on signup: {e}");
                None
            }
        },
        None => None,
    };
    let program_id = match program_id {
        Some(id) => Some(id),
        None => sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM loyalty_programs WHERE is_active AND network_id IS NOT NULL
             ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .unwrap_or(None),
    };

    let Some(program_id) = program_id else {
        tracing::info!(
            "[loyalty] no active programme — visitor {visitor_account_id} not enrolled (admin creates one in the panel)"
        );
        return;
    };

    match find_or_create_member(pool, &program_id, visitor_account_id).await {
        Ok(member_id) => tracing::info!(
            "[loyalty] visitor {visitor_account_id} enrolled in network programme {program_id} as member {member_id}"
        ),
        Err(e) => tracing::warn!(
            "[loyalty] native enrolment failed for visitor {visitor_account_id}: {e}"
        ),
    }
}

/// Find or create a member for (program, visitor account). Network-scoped programmes carry the
/// network on the membership row, which is what the database's partial unique index enforces.
pub async fn find_or_create_member(
    pool: &PgPool,
    program_id: &Uuid,
    visitor_account_id: &Uuid,
) -> Result<Uuid, AppError> {
    if let Some(id) = find_member(pool, program_id, visitor_account_id).await? {
        return Ok(id);
    }

    sqlx::query(
        "INSERT INTO loyalty_members (id, program_id, visitor_account_id, points_balance, lifetime_points, network_id)
         SELECT $1, p.id, $3, 0, 0, p.network_id FROM loyalty_programs p WHERE p.id = $2
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(program_id)
    .bind(visitor_account_id)
    .execute(pool)
    .await?;

    find_member(pool, program_id, visitor_account_id)
        .await?
        .ok_or_else(|| AppError::Internal("could not create loyalty membership".into()))
}

/// The visitor's membership of a programme — directly, or through the network the programme
/// belongs to (one membership per visitor per network).
async fn find_member(
    pool: &PgPool,
    program_id: &Uuid,
    visitor_account_id: &Uuid,
) -> Result<Option<Uuid>, AppError> {
    let id = sqlx::query_scalar(
        "SELECT id FROM loyalty_members
          WHERE visitor_account_id = $2
            AND (program_id = $1
                 OR (network_id IS NOT NULL
                     AND network_id = (SELECT network_id FROM loyalty_programs WHERE id = $1)))
          ORDER BY (program_id = $1) DESC
          LIMIT 1",
    )
    .bind(program_id)
    .bind(visitor_account_id)
    .fetch_optional(pool)
    .await?;

    Ok(id)
}

/// Record a check-in and update balances (+ activity ledger).
pub async fn record_checkin(
    pool: &PgPool,
    program_id: &Uuid,
    member_id: &Uuid,
    points: i32,
    method: &str,
) -> Result<(), AppError> {
    let checkin_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_checkins (id, member_id, points_awarded, method) VALUES ($1, $2, $3, $4)",
    )
    .bind(checkin_id)
    .bind(member_id)
    .bind(points)
    .bind(method)
    .execute(pool)
    .await?;

    sqlx::query(
        "UPDATE loyalty_members SET points_balance = points_balance + $1, lifetime_points = lifetime_points + $1, last_checkin_at = now() WHERE id = $2",
    )
    .bind(points)
    .bind(member_id)
    .execute(pool)
    .await?;

    // activity ledger
    let act_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_activity (id, member_id, activity_type, description, points_earned) VALUES ($1, $2, 'checkin', 'Check-in', $3)",
    )
    .bind(act_id)
    .bind(member_id)
    .bind(points)
    .execute(pool)
    .await?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers — directory-scoped, public-read / admin-write
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/v1/directories/:slug/loyalty/programs — list the programmes that govern a directory
/// The slug selects the *network*: ZaarHub's single programme is returned for every one of its
/// city directories, and a programme that is city-scoped only applies when the directory has no
/// network.
pub async fn list_programs(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, AppError> {
    let directory_id = resolve_directory_id(&state.db, &slug).await?;

    let programs: Vec<LoyaltyProgram> = sqlx::query_as::<_, LoyaltyProgram>(
        r#"SELECT id, directory_id, network_id, name, recognition_method, points_per_checkin,
                  max_checkins_per_day, point_decay_days, points_expire_days,
                  currency_name, currency_icon, currency_color, points_per_visit, points_per_redemption,
                  earn_rate, redemption_cap_pct, min_redeem_balance, exclude_free_items,
                  tiers_enabled, milestones_enabled, streak_enabled, streak_bonus,
                  streak_days, referral_bonus, birthday_bonus, social_share_points,
                  is_active, created_at, updated_at
           FROM loyalty_programs
           WHERE (network_id IS NOT NULL
                  AND network_id = (SELECT network_id FROM directories WHERE id = $1))
              OR (network_id IS NULL AND directory_id = $1)
           ORDER BY (directory_id IS NULL) DESC, created_at"#,
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({ "programs": programs })))
}

/// POST /api/v1/directories/:slug/loyalty/programs — create the programme (admin)
/// Network-wide by design: when the directory belongs to a network the programme is created for
/// the network (directory_id NULL) and an existing one is returned untouched instead of creating
/// a duplicate. This is the rule that keeps ZaarHub at exactly one programme.
pub async fn create_program(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<ProgramInput>,
) -> Result<Json<Value>, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::Validation("Program name is required".into()));
    }
    let directory_id = resolve_directory_id(&state.db, &slug).await?;
    let network_id: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_one(&state.db)
            .await?;

    if let Some(existing) = programme_for_directory(&state.db, &directory_id).await? {
        return Ok(Json(json!({ "program": existing, "already_exists": true })));
    }

    let id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO loyalty_programs (id, directory_id, name, recognition_method, points_per_checkin, max_checkins_per_day, point_decay_days, points_expire_days, currency_name, currency_icon, currency_color, points_per_visit, tiers_enabled, milestones_enabled, streak_enabled, streak_bonus, streak_days, referral_bonus, birthday_bonus, social_share_points, is_active, points_per_redemption, network_id, earn_rate, redemption_cap_pct, min_redeem_balance, exclude_free_items)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27)",
    )
    .bind(id)
    .bind(if network_id.is_some() { None } else { Some(directory_id) })
    .bind(&body.name)
    .bind(body.recognition_method.as_deref().unwrap_or("both"))
    .bind(body.points_per_checkin.unwrap_or(10))
    .bind(body.max_checkins_per_day.unwrap_or(1))
    .bind(body.point_decay_days)
    .bind(body.points_expire_days.unwrap_or(365))
    .bind(body.currency_name.as_deref().unwrap_or("Points"))
    .bind(body.currency_icon.as_deref().unwrap_or("⭐"))
    .bind(body.currency_color.as_deref().unwrap_or("#0d9488"))
    .bind(body.points_per_visit.unwrap_or(5))
    .bind(body.tiers_enabled.unwrap_or(false))
    .bind(body.milestones_enabled.unwrap_or(false))
    .bind(body.streak_enabled.unwrap_or(false))
    .bind(body.streak_bonus.unwrap_or(0))
    .bind(body.streak_days.unwrap_or(7))
    .bind(body.referral_bonus.unwrap_or(0))
    .bind(body.birthday_bonus.unwrap_or(0))
    .bind(body.social_share_points.unwrap_or(0))
    .bind(body.is_active.unwrap_or(true))
    .bind(body.points_per_redemption.unwrap_or(0))
    .bind(network_id)
    .bind(body.earn_rate.map(|v| v.max(0.0)).unwrap_or(1.0))
    .bind(body.redemption_cap_pct.map(|v| v.clamp(0, 100)).unwrap_or(10))
    .bind(
        body.min_redeem_balance
            .map(|v| v.max(0))
            .unwrap_or(100),
    )
    .bind(body.exclude_free_items.unwrap_or(true))
    .execute(&state.db)
    .await?;

    let program = get_program(&state.db, &id).await?;
    Ok(Json(json!({ "program": program })))
}

/// Authorise a programme id against a directory slug. A programme is in scope when it is scoped
/// to the directory itself or to the directory's network (the ZaarHub case: one network-wide
/// programme serving all ten cities). Returns the resolved directory id.
async fn assert_program_in_scope(
    pool: &PgPool,
    slug: &str,
    program_id: &Uuid,
) -> Result<Uuid, AppError> {
    let directory_id = resolve_directory_id(pool, slug).await?;
    let program = get_program(pool, program_id).await?;
    let directory_network: Option<Uuid> =
        sqlx::query_scalar("SELECT network_id FROM directories WHERE id = $1")
            .bind(directory_id)
            .fetch_one(pool)
            .await?;

    let in_scope = match (program.directory_id, program.network_id) {
        (_, Some(program_network)) => Some(program_network) == directory_network,
        (Some(program_directory), None) => program_directory == directory_id,
        (None, None) => false,
    };

    if !in_scope {
        return Err(AppError::NotFound(
            "Program not found for this directory or its network".into(),
        ));
    }

    Ok(directory_id)
}

/// GET /api/v1/directories/:slug/loyalty/programs/:program_id
pub async fn get_program_handler(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    assert_program_in_scope(&state.db, &slug, &program_id).await?;
    let program = get_program(&state.db, &program_id).await?;
    Ok(Json(json!({ "program": program })))
}

/// PUT /api/v1/directories/:slug/loyalty/programs/:program_id — update program (admin)
pub async fn update_program(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<ProgramInput>,
) -> Result<Json<Value>, AppError> {
    assert_program_in_scope(&state.db, &slug, &program_id).await?;

    sqlx::query(
        "UPDATE loyalty_programs SET
            name = COALESCE($2, name),
            recognition_method = COALESCE($3, recognition_method),
            points_per_checkin = COALESCE($4, points_per_checkin),
            max_checkins_per_day = COALESCE($5, max_checkins_per_day),
            points_expire_days = COALESCE($6, points_expire_days),
            currency_name = COALESCE($7, currency_name),
            currency_icon = COALESCE($8, currency_icon),
            currency_color = COALESCE($9, currency_color),
            tiers_enabled = COALESCE($10, tiers_enabled),
            milestones_enabled = COALESCE($11, milestones_enabled),
            streak_enabled = COALESCE($12, streak_enabled),
            streak_bonus = COALESCE($13, streak_bonus),
            streak_days = COALESCE($14, streak_days),
            referral_bonus = COALESCE($15, referral_bonus),
            birthday_bonus = COALESCE($16, birthday_bonus),
            social_share_points = COALESCE($17, social_share_points),
            is_active = COALESCE($18, is_active),
            points_per_redemption = COALESCE($19, points_per_redemption),
            earn_rate = COALESCE($20, earn_rate),
            redemption_cap_pct = COALESCE($21, redemption_cap_pct),
            min_redeem_balance = COALESCE($22, min_redeem_balance),
            exclude_free_items = COALESCE($23, exclude_free_items),
            updated_at = now()
         WHERE id = $1",
    )
    .bind(program_id)
    .bind(&body.name)
    .bind(&body.recognition_method)
    .bind(body.points_per_checkin)
    .bind(body.max_checkins_per_day)
    .bind(body.points_expire_days)
    .bind(&body.currency_name)
    .bind(&body.currency_icon)
    .bind(&body.currency_color)
    .bind(body.tiers_enabled)
    .bind(body.milestones_enabled)
    .bind(body.streak_enabled)
    .bind(body.streak_bonus)
    .bind(body.streak_days)
    .bind(body.referral_bonus)
    .bind(body.birthday_bonus)
    .bind(body.social_share_points)
    .bind(body.is_active)
    .bind(body.points_per_redemption)
    .bind(body.earn_rate.map(|v| v.max(0.0)))
    .bind(body.redemption_cap_pct.map(|v| v.clamp(0, 100)))
    .bind(body.min_redeem_balance.map(|v| v.max(0)))
    .bind(body.exclude_free_items)
    .execute(&state.db)
    .await?;

    let program = get_program(&state.db, &program_id).await?;
    Ok(Json(json!({ "program": program })))
}

/// DELETE /api/v1/directories/:slug/loyalty/programs/:program_id — delete (admin)
pub async fn delete_program(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    assert_program_in_scope(&state.db, &slug, &program_id).await?;
    let res = sqlx::query("DELETE FROM loyalty_programs WHERE id = $1")
        .bind(program_id)
        .execute(&state.db)
        .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("Program not found".into()));
    }
    Ok(Json(json!({ "deleted": true })))
}

/// POST /api/v1/directories/:slug/loyalty/programs/:program_id/enroll — enroll a visitor
/// Body: { visitor_account_id }
#[derive(Debug, Deserialize)]
pub struct EnrollInput {
    pub visitor_account_id: Uuid,
}

pub async fn enroll_member(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<EnrollInput>,
) -> Result<Json<Value>, AppError> {
    let directory_id = assert_program_in_scope(&state.db, &slug, &program_id).await?;

    let member_id = find_or_create_member(&state.db, &program_id, &body.visitor_account_id).await?;

    // Drill down into CoreSwift CRM (fire-and-forget)
    let db = state.db.clone();
    let dir = directory_id;
    let mid = member_id;
    tokio::spawn(async move {
        if let Err(e) = crate::coreswift::push_loyalty_member(&db, dir, mid).await {
            tracing::warn!("[loyalty] CoreSwift drill-down failed on enroll: {e}");
        }
    });

    Ok(Json(json!({ "member_id": member_id })))
}

/// POST /api/v1/directories/:slug/loyalty/programs/:program_id/checkin
/// Body: { visitor_account_id, method }
#[derive(Debug, Deserialize)]
pub struct CheckinInput {
    pub visitor_account_id: Uuid,
    pub method: Option<String>,
}

pub async fn checkin(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<CheckinInput>,
) -> Result<Json<Value>, AppError> {
    let directory_id = assert_program_in_scope(&state.db, &slug, &program_id).await?;
    let program = get_program(&state.db, &program_id).await?;

    let member_id = find_or_create_member(&state.db, &program_id, &body.visitor_account_id).await?;

    // daily cap
    let today_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM loyalty_checkins WHERE member_id = $1 AND checked_in_at::date = CURRENT_DATE",
    )
    .bind(member_id)
    .fetch_one(&state.db)
    .await?;

    if today_count >= program.max_checkins_per_day as i64 {
        return Err(AppError::Validation("Daily check-in limit reached".into()));
    }

    let method = body.method.as_deref().unwrap_or("manual_lookup");
    record_checkin(
        &state.db,
        &program_id,
        &member_id,
        program.points_per_checkin,
        method,
    )
    .await?;

    // Drill down into CoreSwift CRM (fire-and-forget)
    let db = state.db.clone();
    let dir = directory_id;
    let mid = member_id;
    tokio::spawn(async move {
        if let Err(e) = crate::coreswift::push_loyalty_member(&db, dir, mid).await {
            tracing::warn!("[loyalty] CoreSwift drill-down failed on check-in: {e}");
        }
    });

    // return updated member
    let member: LoyaltyMember = sqlx::query_as::<_, LoyaltyMember>(
        r#"SELECT id, program_id, visitor_account_id, points_balance, lifetime_points, tier_id,
                  current_streak, longest_streak, last_activity_date, birthday, referral_code,
                  total_referrals, qr_code, member_since, last_checkin_at
           FROM loyalty_members WHERE id = $1"#,
    )
    .bind(member_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(
        json!({ "member": member, "points_awarded": program.points_per_checkin }),
    ))
}

/// GET /api/v1/directories/:slug/loyalty/members/:visitor_account_id — member summary
pub async fn get_member(
    State(state): State<AppState>,
    Path((slug, visitor_account_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let directory_id = resolve_directory_id(&state.db, &slug).await?;

    // Network-aware: ZaarHub membership is network-wide, so the visitor's membership of the
    // directory's network counts just as much as a city-scoped one.
    let member: Option<LoyaltyMember> = sqlx::query_as::<_, LoyaltyMember>(
        r#"SELECT m.id, m.program_id, m.visitor_account_id, m.points_balance, m.lifetime_points, m.tier_id,
                  m.current_streak, m.longest_streak, m.last_activity_date, m.birthday, m.referral_code,
                  m.total_referrals, m.qr_code, m.member_since, m.last_checkin_at
           FROM loyalty_members m
           JOIN loyalty_programs p ON p.id = m.program_id
           WHERE m.visitor_account_id = $1
             AND (m.network_id = (SELECT network_id FROM directories WHERE id = $2)
                  OR p.directory_id = $2)
           ORDER BY m.member_since DESC LIMIT 1"#,
    )
    .bind(visitor_account_id)
    .bind(directory_id)
    .fetch_optional(&state.db)
    .await?;

    match member {
        Some(m) => Ok(Json(json!({ "member": m }))),
        None => Ok(Json(json!({ "member": null }))),
    }
}
// ─────────────────────────────────────────────────────────────────────────────
// Tiers, rewards, milestones (native — directory-scoped)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LoyaltyTier {
    pub id: Uuid,
    pub loyalty_program_id: Uuid,
    pub name: String,
    pub min_points: i64,
    pub color: String,
    pub perks: Option<Value>,
    pub multiplier: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LoyaltyRewardTier {
    pub id: Uuid,
    pub program_id: Uuid,
    pub name: String,
    pub points_required: i32,
    pub requires_approval: bool,
    #[sqlx(rename = "reward_tag")]
    pub reward_tag: String,
    pub marketing_boost: Option<Value>,
    pub sort_order: i32,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LoyaltyRewardEarned {
    pub id: Uuid,
    pub member_id: Uuid,
    pub tier_id: Option<Uuid>,
    pub status: String,
    pub earned_at: chrono::DateTime<chrono::Utc>,
    pub approved_by: Option<Uuid>,
    pub fulfilled_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LoyaltyMilestone {
    pub id: Uuid,
    pub loyalty_program_id: Uuid,
    pub name: String,
    pub trigger_type: String,
    pub trigger_value: i64,
    pub bonus_points: i64,
    pub bonus_reward_id: Option<Uuid>,
    pub once_per_member: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct TierInput {
    pub name: String,
    pub min_points: Option<i64>,
    pub color: Option<String>,
    pub perks: Option<Value>,
    pub multiplier: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct RewardTierInput {
    pub name: String,
    pub points_required: i32,
    pub requires_approval: Option<bool>,
    pub reward_tag: String,
    pub marketing_boost: Option<Value>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RewardEarnInput {
    pub member_id: Uuid,
    pub tier_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct RewardApproveInput {
    pub approved: bool,
}

#[derive(Debug, Deserialize)]
pub struct MilestoneInput {
    pub name: String,
    pub trigger_type: String,
    pub trigger_value: i64,
    pub bonus_points: i64,
    pub bonus_reward_id: Option<Uuid>,
    pub once_per_member: Option<bool>,
}

async fn owned_program_id(pool: &PgPool, slug: &str, program_id: &Uuid) -> Result<Uuid, AppError> {
    assert_program_in_scope(pool, slug, program_id).await
}

// ── Tiers ──

pub async fn list_tiers(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    let tiers: Vec<LoyaltyTier> = sqlx::query_as(
        "SELECT id, loyalty_program_id, name, min_points, color, perks, multiplier::float8 AS multiplier, created_at
         FROM loyalty_tiers WHERE loyalty_program_id = $1 ORDER BY min_points ASC",
    )
    .bind(program_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "tiers": tiers })))
}

pub async fn create_tier(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<TierInput>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    if body.name.trim().is_empty() {
        return Err(AppError::Validation("Tier name is required".into()));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_tiers (id, loyalty_program_id, name, min_points, color, perks, multiplier)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(program_id)
    .bind(&body.name)
    .bind(body.min_points.unwrap_or(0))
    .bind(body.color.as_deref().unwrap_or("#6B7280"))
    .bind(body.perks.unwrap_or_else(|| json!([])))
    .bind(body.multiplier.unwrap_or(1.0))
    .execute(&state.db)
    .await?;

    let tier: LoyaltyTier = sqlx::query_as(
        "SELECT id, loyalty_program_id, name, min_points, color, perks, multiplier::float8 AS multiplier, created_at
         FROM loyalty_tiers WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({ "tier": tier })))
}

pub async fn delete_tier(
    State(state): State<AppState>,
    Path((slug, program_id, tier_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    let res = sqlx::query("DELETE FROM loyalty_tiers WHERE id = $1 AND loyalty_program_id = $2")
        .bind(tier_id)
        .bind(program_id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "deleted": res.rows_affected() > 0 })))
}

// ── Rewards ──

pub async fn list_rewards(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    let rewards: Vec<LoyaltyRewardTier> = sqlx::query_as(
        "SELECT id, program_id, name, points_required, requires_approval, reward_tag, marketing_boost, sort_order
         FROM loyalty_reward_tiers WHERE program_id = $1 ORDER BY sort_order ASC, points_required ASC",
    )
    .bind(program_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "rewards": rewards })))
}

pub async fn create_reward(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<RewardTierInput>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    if body.name.trim().is_empty() || body.reward_tag.trim().is_empty() {
        return Err(AppError::Validation(
            "Reward name and tag are required".into(),
        ));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_reward_tiers (id, program_id, name, points_required, requires_approval, reward_tag, marketing_boost, sort_order)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(program_id)
    .bind(&body.name)
    .bind(body.points_required)
    .bind(body.requires_approval.unwrap_or(false))
    .bind(&body.reward_tag)
    .bind(body.marketing_boost)
    .bind(body.sort_order.unwrap_or(0))
    .execute(&state.db)
    .await?;

    let reward: LoyaltyRewardTier = sqlx::query_as(
        "SELECT id, program_id, name, points_required, requires_approval, reward_tag, marketing_boost, sort_order
         FROM loyalty_reward_tiers WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({ "reward": reward })))
}

pub async fn delete_reward(
    State(state): State<AppState>,
    Path((slug, program_id, reward_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    let res = sqlx::query("DELETE FROM loyalty_reward_tiers WHERE id = $1 AND program_id = $2")
        .bind(reward_id)
        .bind(program_id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "deleted": res.rows_affected() > 0 })))
}

/// Member claims a reward (spends points). Creates an earned record; auto-approves if not requires_approval.
pub async fn earn_reward(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<RewardEarnInput>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;

    let reward: LoyaltyRewardTier = sqlx::query_as(
        "SELECT id, program_id, name, points_required, requires_approval, reward_tag, marketing_boost, sort_order
         FROM loyalty_reward_tiers WHERE id = $1 AND program_id = $2",
    )
    .bind(body.tier_id)
    .bind(program_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Reward not found in this program".into()))?;

    // Verify member belongs to this program + has enough points
    let member_bal: Option<(Uuid, i32)> = sqlx::query_as(
        "SELECT id, points_balance FROM loyalty_members WHERE id = $1 AND program_id = $2",
    )
    .bind(body.member_id)
    .bind(program_id)
    .fetch_optional(&state.db)
    .await?;
    let (member_id, balance) =
        member_bal.ok_or_else(|| AppError::NotFound("Member not found in this program".into()))?;

    if balance < reward.points_required {
        return Err(AppError::Validation("Insufficient points".into()));
    }

    // Deduct points + record earned
    sqlx::query("UPDATE loyalty_members SET points_balance = points_balance - $1 WHERE id = $2")
        .bind(reward.points_required)
        .bind(member_id)
        .execute(&state.db)
        .await?;

    let earned_id = Uuid::new_v4();
    let status = if reward.requires_approval {
        "pending"
    } else {
        "approved"
    };
    sqlx::query(
        "INSERT INTO loyalty_rewards_earned (id, member_id, tier_id, status) VALUES ($1,$2,$3,$4)",
    )
    .bind(earned_id)
    .bind(member_id)
    .bind(reward.id)
    .bind(status)
    .execute(&state.db)
    .await?;

    Ok(Json(
        json!({ "earned": { "id": earned_id, "reward": reward.name, "status": status, "points_spent": reward.points_required } }),
    ))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EarnedRewardView {
    pub id: Uuid,
    pub member_id: Uuid,
    pub tier_id: Option<Uuid>,
    pub status: String,
    pub earned_at: chrono::DateTime<chrono::Utc>,
    pub reward_name: Option<String>,
}

pub async fn list_earned(
    State(state): State<AppState>,
    Path((slug, member_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    resolve_directory_id(&state.db, &slug).await?;
    let earned: Vec<EarnedRewardView> = sqlx::query_as(
        "SELECT e.id, e.member_id, e.tier_id, e.status, e.earned_at, r.name AS reward_name
         FROM loyalty_rewards_earned e
         LEFT JOIN loyalty_reward_tiers r ON r.id = e.tier_id
         WHERE e.member_id = $1 ORDER BY e.earned_at DESC",
    )
    .bind(member_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "earned": earned })))
}

pub async fn approve_reward(
    State(state): State<AppState>,
    Path((slug, earned_id)): Path<(String, Uuid)>,
    Json(body): Json<RewardApproveInput>,
) -> Result<Json<Value>, AppError> {
    resolve_directory_id(&state.db, &slug).await?;
    let status = if body.approved {
        "approved"
    } else {
        "rejected"
    };
    sqlx::query(
        "UPDATE loyalty_rewards_earned SET status = $1, approved_by = NULL, fulfilled_at = CASE WHEN $1 = 'approved' THEN now() ELSE fulfilled_at END WHERE id = $2",
    )
    .bind(status)
    .bind(earned_id)
    .execute(&state.db)
    .await?;
    Ok(Json(json!({ "status": status })))
}

// ── Milestones ──

pub async fn list_milestones(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    let milestones: Vec<LoyaltyMilestone> = sqlx::query_as(
        "SELECT id, loyalty_program_id, name, trigger_type, trigger_value, bonus_points, bonus_reward_id, once_per_member, created_at
         FROM loyalty_milestones WHERE loyalty_program_id = $1 ORDER BY trigger_value ASC",
    )
    .bind(program_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "milestones": milestones })))
}

pub async fn create_milestone(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<MilestoneInput>,
) -> Result<Json<Value>, AppError> {
    owned_program_id(&state.db, &slug, &program_id).await?;
    if body.name.trim().is_empty() || body.trigger_type.trim().is_empty() {
        return Err(AppError::Validation(
            "Milestone name and trigger_type are required".into(),
        ));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_milestones (id, loyalty_program_id, name, trigger_type, trigger_value, bonus_points, bonus_reward_id, once_per_member)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(program_id)
    .bind(&body.name)
    .bind(&body.trigger_type)
    .bind(body.trigger_value)
    .bind(body.bonus_points)
    .bind(body.bonus_reward_id)
    .bind(body.once_per_member.unwrap_or(true))
    .execute(&state.db)
    .await?;

    let milestone: LoyaltyMilestone = sqlx::query_as(
        "SELECT id, loyalty_program_id, name, trigger_type, trigger_value, bonus_points, bonus_reward_id, once_per_member, created_at
         FROM loyalty_milestones WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(json!({ "milestone": milestone })))
}
