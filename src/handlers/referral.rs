//! Referral programme (card B48) — per directory, admin-verified, native to Multi-Directory.
//!
//! A member's referral code is minted by `feed::generate_referral_code`. Every signup that
//! presents `?ref=CODE` writes a PENDING referral row (see `portal::visitor_register`); an
//! operator verifies it here (or rejects it). Verification awards the directory's own
//! currency to the referrer and marks the row paid.
//!
//! Nothing is hardcoded: the four direction amounts live on the directory itself
//! (`directories.zaarhub_config -> 'referral_rewards'`), defaulting to working ZaarHub
//! values and editable from the admin panel. Every endpoint is operator-guarded at the
//! router; this module never trusts the caller for authorisation.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

/// Working ZaarHub defaults. A directory overrides any of these from the admin panel.
pub fn default_rewards() -> Value {
    json!({
        "visitor_to_visitor": 50,
        "business_to_business": 200,
        "business_to_visitor": 50,
        "visitor_to_business": 100
    })
}

/// Resolve a directory's reward amounts: `zaarhub_config.referral_rewards` merged over the
/// defaults, so every direction always resolves and a partly-configured directory works.
pub async fn reward_amounts(db: &sqlx::PgPool, directory_id: Option<Uuid>) -> Value {
    let mut out = default_rewards();
    let Some(dir) = directory_id else {
        return out;
    };
    let row = sqlx::query("SELECT zaarhub_config FROM directories WHERE id = $1")
        .bind(dir)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    let Some(row) = row else {
        return out;
    };
    let cfg: Option<Value> = row.try_get("zaarhub_config").unwrap_or(None);
    if let Some(Value::Object(map)) = cfg {
        if let Some(Value::Object(custom)) = map.get("referral_rewards") {
            if let Some(dst) = out.as_object_mut() {
                for (k, v) in custom.iter() {
                    if v.is_i64() || v.is_u64() {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }
    out
}

fn direction_key(referrer_type: &str, referee_type: &str) -> String {
    format!("{}_to_{}", referrer_type, referee_type)
}

async fn account_directory(db: &sqlx::PgPool, account_id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar::<_, Option<Uuid>>("SELECT directory_id FROM visitor_accounts WHERE id = $1")
        .bind(account_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .flatten()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub directory_id: Option<Uuid>,
}

/// GET /api/v1/admin/referrals?status=&directory_id=
pub async fn list_referrals(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query(
        r#"SELECT r.id, r.referrer_type, r.referrer_id, r.referrer_email,
                  r.referee_type, r.referee_id, r.referee_email, r.referee_name,
                  r.referral_code, r.status, r.zaarcash_earned,
                  r.verified_at, r.created_at,
                  CASE WHEN r.referee_id IS NOT NULL THEN true ELSE false END AS used
           FROM referrals r
           LEFT JOIN visitor_accounts ra ON ra.id = r.referrer_id AND r.referrer_type = 'visitor'
           LEFT JOIN visitor_accounts da ON da.id = r.referee_id AND r.referee_type = 'visitor'
           WHERE ($1::text IS NULL OR r.status = $1)
             AND ($2::uuid IS NULL OR ra.directory_id = $2 OR da.directory_id = $2)
           ORDER BY r.created_at DESC
           LIMIT 300"#,
    )
    .bind(&q.status)
    .bind(q.directory_id)
    .fetch_all(&s.db)
    .await?;

    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<Uuid, _>("id").ok(),
                "referrer_type": r.try_get::<String, _>("referrer_type").unwrap_or_default(),
                "referrer_email": r.try_get::<Option<String>, _>("referrer_email").unwrap_or(None),
                "referee_type": r.try_get::<String, _>("referee_type").unwrap_or_default(),
                "referee_email": r.try_get::<Option<String>, _>("referee_email").unwrap_or(None),
                "referee_name": r.try_get::<Option<String>, _>("referee_name").unwrap_or(None),
                "referral_code": r.try_get::<String, _>("referral_code").unwrap_or_default(),
                "status": r.try_get::<String, _>("status").unwrap_or_default(),
                "zaarcash_earned": r.try_get::<i32, _>("zaarcash_earned").unwrap_or(0),
                "used": r.try_get::<bool, _>("used").unwrap_or(false),
                "created_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").ok(),
            })
        })
        .collect();

    Ok(Json(json!({ "referrals": list, "total": list.len() })))
}

fn referral_json(row: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": row.try_get::<Uuid, _>("id").ok(),
        "referral_code": row.try_get::<String, _>("referral_code").unwrap_or_default(),
        "referrer_type": row.try_get::<String, _>("referrer_type").unwrap_or_default(),
        "referee_type": row.try_get::<String, _>("referee_type").unwrap_or_default(),
        "referee_email": row.try_get::<Option<String>, _>("referee_email").unwrap_or(None),
        "status": row.try_get::<String, _>("status").unwrap_or_default(),
        "zaarcash_earned": row.try_get::<i32, _>("zaarcash_earned").unwrap_or(0),
    })
}

/// POST /api/v1/admin/referrals/:id/verify — pay the referrer and mark the row paid.
pub async fn verify_referral(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query("SELECT * FROM referrals WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Referral '{}' not found", id)))?;

    let status: String = row.try_get("status").unwrap_or_default();
    if status == "paid" {
        return Err(AppError::Validation(
            "This referral is already paid".to_string(),
        ));
    }
    let referrer_type: String = row
        .try_get("referrer_type")
        .unwrap_or_else(|_| "visitor".to_string());
    let referrer_id: Uuid = row.try_get("referrer_id")?;
    let referee_type: String = row
        .try_get("referee_type")
        .unwrap_or_else(|_| "visitor".to_string());
    let referee_id: Option<Uuid> = row.try_get("referee_id").unwrap_or(None);
    let Some(referee_id) = referee_id else {
        return Err(AppError::Validation(
            "This code has not been used by anyone yet — there is no referee to verify".to_string(),
        ));
    };

    // The directory that owns the reward: the referee's account first, else the referrer's.
    let directory = match account_directory(&s.db, referee_id).await {
        Some(d) => Some(d),
        None => account_directory(&s.db, referrer_id).await,
    };

    let amounts = reward_amounts(&s.db, directory).await;
    let amount = amounts
        .get(direction_key(&referrer_type, &referee_type))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(0) as i32;

    // Award the referrer's currency natively (graceful when no programme is configured).
    if referrer_type == "visitor" {
        if let Some(dir) = directory {
            if let Err(e) = crate::handlers::loyalty_native::credit_visitor_units(
                &s.db,
                &dir,
                &referrer_id,
                amount,
                "referral",
                "Referral reward",
            )
            .await
            {
                eprintln!("[referral] credit failed for referrer {referrer_id}: {e}");
            }
        }
    }

    let updated = sqlx::query(
        "UPDATE referrals SET status = 'paid', zaarcash_earned = $1, verified_at = now(), \
         updated_at = now() WHERE id = $2 RETURNING *",
    )
    .bind(amount)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!({
        "verified": true,
        "reward": amount,
        "directory_id": directory,
        "referral": referral_json(&updated),
    })))
}

/// POST /api/v1/admin/referrals/:id/reject — close a referral without paying.
pub async fn reject_referral(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let updated = sqlx::query(
        "UPDATE referrals SET status = 'expired', updated_at = now() \
         WHERE id = $1 AND status <> 'paid' RETURNING *",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::Validation("Referral not found, or it is already paid".to_string()))?;

    Ok(Json(
        json!({ "rejected": true, "referral": referral_json(&updated) }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SettingsQuery {
    pub directory_id: Uuid,
}

/// GET /api/v1/admin/referral-settings?directory_id=
pub async fn get_referral_settings(
    State(s): State<AppState>,
    Query(q): Query<SettingsQuery>,
) -> ApiResult<Json<Value>> {
    let rewards = reward_amounts(&s.db, Some(q.directory_id)).await;
    Ok(Json(json!({
        "directory_id": q.directory_id,
        "defaults": default_rewards(),
        "rewards": rewards,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SaveRewardsRequest {
    pub directory_id: Uuid,
    pub visitor_to_visitor: Option<i64>,
    pub business_to_business: Option<i64>,
    pub business_to_visitor: Option<i64>,
    pub visitor_to_business: Option<i64>,
}

/// PUT /api/v1/admin/referral-settings — per-directory reward amounts.
pub async fn put_referral_settings(
    State(s): State<AppState>,
    Json(b): Json<SaveRewardsRequest>,
) -> ApiResult<Json<Value>> {
    let mut rewards = reward_amounts(&s.db, Some(b.directory_id)).await;
    if let Some(obj) = rewards.as_object_mut() {
        let mut set = |k: &str, v: Option<i64>| {
            if let Some(n) = v {
                obj.insert(k.to_string(), json!(n.max(0)));
            }
        };
        set("visitor_to_visitor", b.visitor_to_visitor);
        set("business_to_business", b.business_to_business);
        set("business_to_visitor", b.business_to_visitor);
        set("visitor_to_business", b.visitor_to_business);
    }

    sqlx::query(
        "UPDATE directories SET zaarhub_config = \
         COALESCE(zaarhub_config, '{}'::jsonb) || \
         jsonb_build_object('referral_rewards', $2::jsonb) WHERE id = $1",
    )
    .bind(b.directory_id)
    .bind(rewards.to_string())
    .execute(&s.db)
    .await?;

    Ok(Json(
        json!({ "saved": true, "directory_id": b.directory_id, "rewards": rewards }),
    ))
}
