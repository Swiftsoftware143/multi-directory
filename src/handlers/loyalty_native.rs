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

// ─────────────────────────────────────────────────────────────────────────────
// Program config
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LoyaltyProgram {
    pub id: Uuid,
    pub directory_id: Uuid,
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
        r#"SELECT id, directory_id, name, recognition_method, points_per_checkin,
                  max_checkins_per_day, point_decay_days, points_expire_days,
                  currency_name, currency_icon, currency_color, points_per_visit,
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

/// Find or create a member for (program, visitor account).
pub async fn find_or_create_member(
    pool: &PgPool,
    program_id: &Uuid,
    visitor_account_id: &Uuid,
) -> Result<Uuid, AppError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM loyalty_members WHERE program_id = $1 AND visitor_account_id = $2",
    )
    .bind(program_id)
    .bind(visitor_account_id)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO loyalty_members (id, program_id, visitor_account_id, points_balance, lifetime_points)
         VALUES ($1, $2, $3, 0, 0)",
    )
    .bind(id)
    .bind(program_id)
    .bind(visitor_account_id)
    .execute(pool)
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

/// GET /api/v1/directories/:slug/loyalty/programs — list programs for a directory
pub async fn list_programs(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, AppError> {
    let directory_id = resolve_directory_id(&state.db, &slug).await?;

    let programs: Vec<LoyaltyProgram> = sqlx::query_as::<_, LoyaltyProgram>(
        r#"SELECT id, directory_id, name, recognition_method, points_per_checkin,
                  max_checkins_per_day, point_decay_days, points_expire_days,
                  currency_name, currency_icon, currency_color, points_per_visit,
                  tiers_enabled, milestones_enabled, streak_enabled, streak_bonus,
                  streak_days, referral_bonus, birthday_bonus, social_share_points,
                  is_active, created_at, updated_at
           FROM loyalty_programs WHERE directory_id = $1 ORDER BY created_at"#,
    )
    .bind(directory_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({ "programs": programs })))
}

/// POST /api/v1/directories/:slug/loyalty/programs — create a program (admin)
pub async fn create_program(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<ProgramInput>,
) -> Result<Json<Value>, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::Validation("Program name is required".into()));
    }
    let directory_id = resolve_directory_id(&state.db, &slug).await?;
    let id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO loyalty_programs (id, directory_id, name, recognition_method, points_per_checkin, max_checkins_per_day, point_decay_days, points_expire_days, currency_name, currency_icon, currency_color, points_per_visit, tiers_enabled, milestones_enabled, streak_enabled, streak_bonus, streak_days, referral_bonus, birthday_bonus, social_share_points, is_active)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)",
    )
    .bind(id)
    .bind(directory_id)
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
    .execute(&state.db)
    .await?;

    let program = get_program(&state.db, &id).await?;
    Ok(Json(json!({ "program": program })))
}

/// GET /api/v1/directories/:slug/loyalty/programs/:program_id
pub async fn get_program_handler(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, AppError> {
    let directory_id = resolve_directory_id(&state.db, &slug).await?;
    let program = get_program(&state.db, &program_id).await?;
    if program.directory_id != directory_id {
        return Err(AppError::NotFound(
            "Program not found in this directory".into(),
        ));
    }
    Ok(Json(json!({ "program": program })))
}

/// PUT /api/v1/directories/:slug/loyalty/programs/:program_id — update program (admin)
pub async fn update_program(
    State(state): State<AppState>,
    Path((slug, program_id)): Path<(String, Uuid)>,
    Json(body): Json<ProgramInput>,
) -> Result<Json<Value>, AppError> {
    let directory_id = resolve_directory_id(&state.db, &slug).await?;

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
            updated_at = now()
         WHERE id = $1 AND directory_id = $19",
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
    .bind(directory_id)
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
    let directory_id = resolve_directory_id(&state.db, &slug).await?;
    let res = sqlx::query("DELETE FROM loyalty_programs WHERE id = $1 AND directory_id = $2")
        .bind(program_id)
        .bind(directory_id)
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
    let directory_id = resolve_directory_id(&state.db, &slug).await?;
    let program = get_program(&state.db, &program_id).await?;
    if program.directory_id != directory_id {
        return Err(AppError::NotFound(
            "Program not found in this directory".into(),
        ));
    }

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
    let directory_id = resolve_directory_id(&state.db, &slug).await?;
    let program = get_program(&state.db, &program_id).await?;
    if program.directory_id != directory_id {
        return Err(AppError::NotFound(
            "Program not found in this directory".into(),
        ));
    }

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

    let member: Option<LoyaltyMember> = sqlx::query_as::<_, LoyaltyMember>(
        r#"SELECT m.id, m.program_id, m.visitor_account_id, m.points_balance, m.lifetime_points, m.tier_id,
                  m.current_streak, m.longest_streak, m.last_activity_date, m.birthday, m.referral_code,
                  m.total_referrals, m.qr_code, m.member_since, m.last_checkin_at
           FROM loyalty_members m
           JOIN loyalty_programs p ON p.id = m.program_id
           WHERE m.visitor_account_id = $1 AND p.directory_id = $2
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

async fn owned_program_id(
    pool: &PgPool,
    slug: &str,
    program_id: &Uuid,
) -> Result<Uuid, AppError> {
    let directory_id = resolve_directory_id(pool, slug).await?;
    let program = get_program(pool, program_id).await?;
    if program.directory_id != directory_id {
        return Err(AppError::NotFound("Program not found in this directory".into()));
    }
    Ok(directory_id)
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
        return Err(AppError::Validation("Reward name and tag are required".into()));
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
    let (member_id, balance) = member_bal
        .ok_or_else(|| AppError::NotFound("Member not found in this program".into()))?;

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
    let status = if reward.requires_approval { "pending" } else { "approved" };
    sqlx::query(
        "INSERT INTO loyalty_rewards_earned (id, member_id, tier_id, status) VALUES ($1,$2,$3,$4)",
    )
    .bind(earned_id)
    .bind(member_id)
    .bind(reward.id)
    .bind(status)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({ "earned": { "id": earned_id, "reward": reward.name, "status": status, "points_spent": reward.points_required } })))
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
    let status = if body.approved { "approved" } else { "rejected" };
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
        return Err(AppError::Validation("Milestone name and trigger_type are required".into()));
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
