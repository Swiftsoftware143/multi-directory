//! Ownership transfers (T2) — move a business from one owner to another.
//!
//! Built on `business_transfers` / `business_transfer_fees` (migration 086) plus
//! `business_transfer_events` for the audit trail (migration 090).
//!
//! Nothing is hardwired:
//!   * `fee_cents`, `currency`, `fee_direction` and `host_stays` are values an
//!     admin (or the owner requesting the transfer) enters — never constants;
//!   * `host_stays = true` keeps the hosting directory (`businesses.directory_id`)
//!     exactly as it is; `host_stays = false` re-homes the listing to the
//!     admin-chosen `target_directory_id`;
//!   * a transfer addressed to an email with no account is an invitation — the
//!     account is bound at accept time, nothing is faked in between.
//!
//! ACCEPT is ONE transaction: lock the transfer, lock the business, reassign
//! `businesses.owner_id` (+ directory and category validity when re-homing),
//! reassign `claimed_businesses` (email + user_id, keeping its subscription),
//! record the fee in `business_transfer_fees`, flip the status and write the audit
//! event. Any failure rolls the whole thing back — ownership never half-moves.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::is_super_admin;
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Transfer {
    pub id: Uuid,
    pub business_id: Uuid,
    pub business_name: Option<String>,
    pub business_slug: Option<String>,
    pub directory_id: Option<Uuid>,
    pub from_tenant_id: Option<Uuid>,
    pub from_user_id: Option<Uuid>,
    pub from_email: Option<String>,
    pub to_tenant_id: Option<Uuid>,
    pub to_user_id: Option<Uuid>,
    pub to_email: Option<String>,
    pub fee_cents: i32,
    pub currency: String,
    pub fee_direction: String,
    pub host_stays: bool,
    pub target_directory_id: Option<Uuid>,
    pub status: String,
    pub notes: Option<String>,
    pub requested_by: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub decided_at: Option<DateTime<Utc>>,
}

const TRANSFER_COLUMNS: &str =
    "t.id, t.business_id, b.name AS business_name, b.slug AS business_slug, \
     b.directory_id, t.from_tenant_id, t.from_user_id, t.from_email, t.to_tenant_id, t.to_user_id, \
     t.to_email, t.fee_cents, t.currency, t.fee_direction, t.host_stays, t.target_directory_id, \
     t.status, t.notes, t.requested_by, t.created_at, t.updated_at, t.decided_at";

fn is_admin(claims: &Claims) -> bool {
    // Round 13 IDOR audit: `admin` is the per-tenant role every business owner holds,
    // so treating it as "may act on anyone's transfer" let any tenant read, re-price,
    // accept, decline or cancel another tenant's transfer. Only the platform operator
    // (super_admin) acts across tenants; a party still passes on its own side.
    claims.role == "super_admin"
}

fn actor_id(claims: &Claims) -> ApiResult<Uuid> {
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

async fn user_email(db: &sqlx::PgPool, user_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

/// Lowercased email of a claim's user, if the user row still exists.
async fn actor_email(db: &sqlx::PgPool, claims: &Claims) -> Option<String> {
    match Uuid::parse_str(&claims.sub) {
        Ok(id) => user_email(db, id).await.map(|e| e.to_lowercase()),
        Err(_) => None,
    }
}

/// Who currently owns the business: the active claim row (email + user id) wins,
/// falling back to `businesses.owner_id` -> `users.email`.
async fn current_owner(db: &sqlx::PgPool, business_id: Uuid) -> (Option<Uuid>, Option<String>) {
    let claim = sqlx::query_as::<_, (Option<Uuid>, Option<String>)>(
        "SELECT user_id, owner_email FROM claimed_businesses \
         WHERE business_id = $1 AND is_active = true ORDER BY created_at ASC LIMIT 1",
    )
    .bind(business_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();

    if let Some((uid, email)) = claim {
        if uid.is_some() || email.is_some() {
            let resolved_email = match email {
                Some(ref e) if !e.trim().is_empty() => Some(e.to_lowercase()),
                _ => match uid {
                    Some(u) => user_email(db, u).await.map(|e| e.to_lowercase()),
                    None => None,
                },
            };
            return (uid, resolved_email);
        }
    }

    let owner = sqlx::query_scalar::<_, Uuid>("SELECT owner_id FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    match owner {
        Some(u) => (Some(u), user_email(db, u).await.map(|e| e.to_lowercase())),
        None => (None, None),
    }
}

async fn require_pending_transfer(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
) -> ApiResult<(
    String,
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<Uuid>,
    i32,
    String,
    String,
    bool,
    Option<Uuid>,
)> {
    sqlx::query_as::<_, (String, Uuid, Option<Uuid>, Option<Uuid>, Option<String>, Option<String>, Option<Uuid>, i32, String, String, bool, Option<Uuid>)>(
        "SELECT status, business_id, to_user_id, from_user_id, to_email, from_email, to_tenant_id, \
         fee_cents, currency, fee_direction, host_stays, target_directory_id \
         FROM business_transfers WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Transfer not found".to_string()))
}

async fn write_event(
    exec: &mut sqlx::PgConnection,
    transfer_id: Uuid,
    business_id: Option<Uuid>,
    claims: &Claims,
    event: &str,
    from_status: Option<&str>,
    to_status: Option<&str>,
    metadata: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO business_transfer_events \
         (transfer_id, business_id, actor_user_id, actor_role, event, from_status, to_status, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb)",
    )
    .bind(transfer_id)
    .bind(business_id)
    .bind(Uuid::parse_str(&claims.sub).ok())
    .bind(&claims.role)
    .bind(event)
    .bind(from_status)
    .bind(to_status)
    .bind(&metadata)
    .execute(&mut *exec)
    .await
    .map(|_| ())
}

fn sanitize_fee_direction(v: Option<String>, fallback: &str) -> String {
    match v.as_deref() {
        Some("incoming") | Some("outgoing") | Some("platform") => v.unwrap(),
        _ => fallback.to_string(),
    }
}

async fn fetch_transfer(db: &sqlx::PgPool, id: Uuid) -> ApiResult<Transfer> {
    let sql = format!(
        "SELECT {} FROM business_transfers t JOIN businesses b ON b.id = t.business_id WHERE t.id = $1",
        TRANSFER_COLUMNS
    );
    sqlx::query_as::<_, Transfer>(&sql)
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("Transfer not found".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// POST /api/v1/transfers — initiate (current owner or admin)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateTransferRequest {
    pub business_id: Uuid,
    pub to_email: String,
    pub fee_cents: Option<i32>,
    pub currency: Option<String>,
    pub fee_direction: Option<String>,
    pub host_stays: Option<bool>,
    pub target_directory_id: Option<Uuid>,
    pub notes: Option<String>,
}

pub async fn create_transfer(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateTransferRequest>,
) -> ApiResult<impl IntoResponse> {
    let actor = actor_id(&claims)?;
    let email = actor_email(&s.db, &claims).await;
    let admin = is_super_admin(&claims);

    let business = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, String)>(
        "SELECT owner_id, directory_id, name FROM businesses WHERE id = $1",
    )
    .bind(req.business_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;
    let (owner_id, directory_id, business_name) = business;

    let (from_user_id, from_email) = current_owner(&s.db, req.business_id).await;

    let owns = owner_id == Some(actor)
        || from_user_id == Some(actor)
        || match (&from_email, &email) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
    if !admin && !owns {
        return Err(AppError::Forbidden(
            "Only the current owner or an admin can start a transfer".to_string(),
        ));
    }

    let to_email = req.to_email.trim().to_lowercase();
    if to_email.is_empty() || !to_email.contains('@') {
        return Err(AppError::Validation(
            "A valid incoming-owner email is required".to_string(),
        ));
    }
    if let Some(ref f) = from_email {
        if *f == to_email {
            return Err(AppError::Validation(
                "The incoming owner cannot be the current owner".to_string(),
            ));
        }
    }

    let host_stays = req.host_stays.unwrap_or(true);
    if !host_stays && req.target_directory_id.is_none() {
        return Err(AppError::Validation(
            "host_stays=false requires a target directory to re-home the listing into".to_string(),
        ));
    }

    let fee_cents = req.fee_cents.unwrap_or(0).max(0);
    let currency = req
        .currency
        .clone()
        .unwrap_or_else(|| "USD".to_string())
        .to_uppercase();
    let fee_direction = sanitize_fee_direction(req.fee_direction.clone(), "incoming");

    // The incoming owner may already have an account; if not this stays an
    // invitation addressed to the email and is bound at accept time.
    let incoming = sqlx::query_as::<_, (Uuid, Option<Uuid>, String)>(
        "SELECT id, tenant_id, email FROM users WHERE lower(email) = lower($1) ORDER BY created_at ASC LIMIT 1",
    )
    .bind(&to_email)
    .fetch_optional(&s.db)
    .await?;

    let (to_user_id, to_tenant_id) = match incoming {
        Some((id, tid, _)) => (Some(id), tid),
        None => (None, None),
    };

    let id = Uuid::new_v4();
    let mut tx = s.db.begin().await?;

    sqlx::query(
        "INSERT INTO business_transfers \
         (id, business_id, from_tenant_id, from_user_id, from_email, to_tenant_id, to_user_id, to_email, \
          fee_cents, currency, fee_direction, host_stays, target_directory_id, status, notes, requested_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'pending', $14, $15)",
    )
    .bind(id)
    .bind(req.business_id)
    .bind(Uuid::parse_str(&claims.tid).ok())
    .bind(from_user_id)
    .bind(&from_email)
    .bind(to_tenant_id)
    .bind(to_user_id)
    .bind(&to_email)
    .bind(fee_cents)
    .bind(&currency)
    .bind(&fee_direction)
    .bind(host_stays)
    .bind(req.target_directory_id)
    .bind(req.notes.clone())
    .bind(actor)
    .execute(&mut *tx)
    .await?;

    write_event(
        &mut tx,
        id,
        Some(req.business_id),
        &claims,
        "created",
        None,
        Some("pending"),
        json!({
            "business_name": business_name,
            "from_email": from_email,
            "to_email": to_email,
            "invitation": to_user_id.is_none(),
            "fee_cents": fee_cents,
            "currency": currency,
            "fee_direction": fee_direction,
            "host_stays": host_stays,
            "target_directory_id": req.target_directory_id,
            "from_directory_id": directory_id,
        }),
    )
    .await?;

    tx.commit().await?;

    let row = fetch_transfer(&s.db, id).await?;
    Ok((StatusCode::CREATED, Json(json!(row))))
}

// ─────────────────────────────────────────────────────────────────────────────
// GET /api/v1/transfers — incoming / outgoing / history (admin: everything)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListTransfersQuery {
    pub scope: Option<String>,
    pub status: Option<String>,
    pub business_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub limit: Option<i64>,
}

pub async fn list_transfers(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(q): Query<ListTransfersQuery>,
) -> ApiResult<impl IntoResponse> {
    let actor = actor_id(&claims)?;
    let email = actor_email(&s.db, &claims).await.unwrap_or_default();
    let admin = is_super_admin(&claims);
    let scope = q.scope.clone().unwrap_or_else(|| "all".to_string());
    let limit = q.limit.unwrap_or(100).clamp(1, 500);

    let sql = format!(
        "SELECT {} FROM business_transfers t JOIN businesses b ON b.id = t.business_id \
         WHERE ($1::boolean \
                OR t.to_user_id = $2 OR lower(coalesce(t.to_email, '')) = $3 \
                OR t.from_user_id = $2 OR lower(coalesce(t.from_email, '')) = $3) \
           AND ($4::text IS NULL OR $4 = 'all' \
                OR ($4 = 'history' AND t.status <> 'pending') \
                OR ($1::boolean AND $4 IN ('incoming', 'outgoing')) \
                OR (NOT $1::boolean AND ( \
                      ($4 = 'incoming' AND (t.to_user_id = $2 OR lower(coalesce(t.to_email, '')) = $3)) \
                   OR ($4 = 'outgoing' AND (t.from_user_id = $2 OR lower(coalesce(t.from_email, '')) = $3))))) \
           AND ($5::text IS NULL OR t.status = $5) \
           AND ($6::uuid IS NULL OR t.business_id = $6) \
           AND ($7::uuid IS NULL OR b.directory_id = $7) \
         ORDER BY t.created_at DESC LIMIT $8",
        TRANSFER_COLUMNS
    );

    let rows = sqlx::query_as::<_, Transfer>(&sql)
        .bind(admin)
        .bind(actor)
        .bind(&email)
        .bind(&scope)
        .bind(q.status.clone())
        .bind(q.business_id)
        .bind(q.directory_id)
        .bind(limit)
        .fetch_all(&s.db)
        .await?;

    Ok(Json(json!({
        "transfers": rows,
        "scope": scope,
        "is_admin": admin,
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// GET /api/v1/transfers/options — businesses + directories for the pickers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OptionsQuery {
    pub directory_id: Option<Uuid>,
    pub q: Option<String>,
    pub limit: Option<i64>,
}

pub async fn transfer_options(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(qs): Query<OptionsQuery>,
) -> ApiResult<impl IntoResponse> {
    let actor = actor_id(&claims)?;
    let email = actor_email(&s.db, &claims).await.unwrap_or_default();
    let admin = is_super_admin(&claims);
    let limit = qs.limit.unwrap_or(200).clamp(1, 500);
    let search = qs.q.clone().map(|s| s.to_lowercase());

    let directories = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, slug FROM directories ORDER BY name ASC LIMIT 500",
    )
    .fetch_all(&s.db)
    .await?;

    // Admin sees every business; an owner sees only the businesses they own.
    let businesses = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<Uuid>, Option<Uuid>, Option<String>)>(
        "SELECT b.id, b.name, b.slug, b.directory_id, b.owner_id, \
                (SELECT cb.owner_email FROM claimed_businesses cb WHERE cb.business_id = b.id AND cb.is_active = true LIMIT 1) AS owner_email \
         FROM businesses b \
         WHERE ($1::boolean \
                OR b.owner_id = $2 \
                OR EXISTS (SELECT 1 FROM claimed_businesses cb2 WHERE cb2.business_id = b.id AND cb2.is_active = true \
                           AND (cb2.user_id = $2 OR lower(cb2.owner_email) = $3))) \
           AND ($4::uuid IS NULL OR b.directory_id = $4) \
           AND ($5::text IS NULL OR lower(b.name) LIKE '%' || $5 || '%') \
         ORDER BY b.name ASC LIMIT $6",
    )
    .bind(admin)
    .bind(actor)
    .bind(&email)
    .bind(qs.directory_id)
    .bind(&search)
    .bind(limit)
    .fetch_all(&s.db)
    .await?;

    let businesses: Vec<serde_json::Value> = businesses
        .into_iter()
        .map(|(id, name, slug, directory_id, owner_id, owner_email)| {
            json!({
                "id": id, "name": name, "slug": slug,
                "directory_id": directory_id, "owner_id": owner_id, "owner_email": owner_email,
            })
        })
        .collect();

    let directories: Vec<serde_json::Value> = directories
        .into_iter()
        .map(|(id, name, slug)| json!({ "id": id, "name": name, "slug": slug }))
        .collect();

    Ok(Json(
        json!({ "businesses": businesses, "directories": directories }),
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// GET /api/v1/transfers/:id — detail + audit trail + fee rows
// ─────────────────────────────────────────────────────────────────────────────

pub async fn get_transfer(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let transfer = fetch_transfer(&s.db, id).await?;
    let actor = actor_id(&claims)?;
    let email = actor_email(&s.db, &claims).await.unwrap_or_default();
    let mine = transfer.from_user_id == Some(actor)
        || transfer.to_user_id == Some(actor)
        || transfer.from_email.as_deref().map(|e| e.to_lowercase()) == Some(email.clone())
        || transfer.to_email.as_deref().map(|e| e.to_lowercase()) == Some(email.clone());
    if !is_super_admin(&claims) && !mine {
        return Err(AppError::Forbidden(
            "Not a party to this transfer".to_string(),
        ));
    }

    let events = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<String>, String, Option<String>, Option<String>, serde_json::Value, DateTime<Utc>)>(
        "SELECT id, actor_user_id, actor_role, event, from_status, to_status, metadata, created_at \
         FROM business_transfer_events WHERE transfer_id = $1 ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    let events: Vec<serde_json::Value> = events
        .into_iter()
        .map(
            |(
                id,
                actor_user_id,
                actor_role,
                event,
                from_status,
                to_status,
                metadata,
                created_at,
            )| {
                json!({
                    "id": id, "actor_user_id": actor_user_id, "actor_role": actor_role,
                    "event": event, "from_status": from_status, "to_status": to_status,
                    "metadata": metadata, "created_at": created_at,
                })
            },
        )
        .collect();

    let fees = sqlx::query_as::<_, (Uuid, Option<Uuid>, Option<Uuid>, i32, String, String, Option<DateTime<Utc>>, DateTime<Utc>)>(
        "SELECT id, payer_user_id, payee_user_id, amount_cents, currency, status, settled_at, created_at \
         FROM business_transfer_fees WHERE transfer_id = $1 ORDER BY created_at ASC",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    let fees: Vec<serde_json::Value> = fees
        .into_iter()
        .map(
            |(
                id,
                payer_user_id,
                payee_user_id,
                amount_cents,
                currency,
                status,
                settled_at,
                created_at,
            )| {
                json!({
                    "id": id, "payer_user_id": payer_user_id, "payee_user_id": payee_user_id,
                    "amount_cents": amount_cents, "currency": currency, "status": status,
                    "settled_at": settled_at, "created_at": created_at,
                })
            },
        )
        .collect();

    Ok(Json(
        json!({ "transfer": transfer, "events": events, "fees": fees }),
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// PUT /api/v1/transfers/:id — admin edits fee / host_stays / target dir (pending only)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateTransferRequest {
    pub fee_cents: Option<i32>,
    pub currency: Option<String>,
    pub fee_direction: Option<String>,
    pub host_stays: Option<bool>,
    pub target_directory_id: Option<Uuid>,
    pub notes: Option<String>,
}

pub async fn update_transfer(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTransferRequest>,
) -> ApiResult<impl IntoResponse> {
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden(
            "Admin role required to change transfer terms".to_string(),
        ));
    }

    let mut tx = s.db.begin().await?;
    let (
        status,
        business_id,
        _,
        _,
        _,
        _,
        _,
        fee_cents,
        currency,
        fee_direction,
        host_stays,
        target,
    ) = require_pending_transfer(&mut tx, id).await?;
    if status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Transfer is already {}",
            status
        )));
    }

    let new_fee = req.fee_cents.unwrap_or(fee_cents).max(0);
    let new_currency = req.currency.clone().unwrap_or(currency).to_uppercase();
    let new_direction = sanitize_fee_direction(req.fee_direction.clone(), &fee_direction);
    let new_host_stays = req.host_stays.unwrap_or(host_stays);
    let new_target = if new_host_stays {
        None
    } else {
        req.target_directory_id.or(target)
    };
    if !new_host_stays && new_target.is_none() {
        return Err(AppError::Validation(
            "host_stays=false requires a target directory".to_string(),
        ));
    }
    let new_notes = req.notes.clone();

    sqlx::query(
        "UPDATE business_transfers SET fee_cents = $1, currency = $2, fee_direction = $3, \
         host_stays = $4, target_directory_id = $5, \
         notes = COALESCE($6, notes), updated_at = now() WHERE id = $7",
    )
    .bind(new_fee)
    .bind(&new_currency)
    .bind(&new_direction)
    .bind(new_host_stays)
    .bind(new_target)
    .bind(new_notes)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    write_event(
        &mut tx,
        id,
        Some(business_id),
        &claims,
        "terms_updated",
        Some("pending"),
        Some("pending"),
        json!({
            "fee_cents": new_fee, "currency": new_currency, "fee_direction": new_direction,
            "host_stays": new_host_stays, "target_directory_id": new_target,
        }),
    )
    .await?;

    tx.commit().await?;
    let row = fetch_transfer(&s.db, id).await?;
    Ok(Json(json!(row)))
}

// ─────────────────────────────────────────────────────────────────────────────
// POST /api/v1/transfers/:id/accept — the atomic ownership move
// ─────────────────────────────────────────────────────────────────────────────

pub async fn accept_transfer(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let actor = actor_id(&claims)?;
    let email = actor_email(&s.db, &claims).await.unwrap_or_default();

    let mut tx = s.db.begin().await?;
    let (
        status,
        business_id,
        to_user_id,
        from_user_id,
        to_email,
        from_email,
        _to_tenant_id,
        fee_cents,
        currency,
        fee_direction,
        host_stays,
        target,
    ) = require_pending_transfer(&mut tx, id).await?;
    if status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Transfer is already {}",
            status
        )));
    }

    let incoming_email = to_email.clone().unwrap_or_default().to_lowercase();
    let is_incoming =
        to_user_id == Some(actor) || (!incoming_email.is_empty() && incoming_email == email);
    if !is_super_admin(&claims) && !is_incoming {
        return Err(AppError::Forbidden(
            "Only the incoming owner or an admin can accept this transfer".to_string(),
        ));
    }

    // Bind the incoming account. An invitation only converts when a real account
    // exists for the address — never assume one.
    let incoming = match to_user_id {
        Some(u) => sqlx::query_as::<_, (Uuid, Option<Uuid>, String, Option<String>)>(
            "SELECT id, tenant_id, email, name FROM users WHERE id = $1",
        )
        .bind(u)
        .fetch_optional(&mut *tx)
        .await?,
        None => sqlx::query_as::<_, (Uuid, Option<Uuid>, String, Option<String>)>(
            "SELECT id, tenant_id, email, name FROM users WHERE lower(email) = lower($1) ORDER BY created_at ASC LIMIT 1",
        )
        .bind(&incoming_email)
        .fetch_optional(&mut *tx)
        .await?,
    };
    let (incoming_id, incoming_tenant, incoming_email_val, incoming_name) = incoming.ok_or_else(|| {
        AppError::BadRequest(format!(
            "No account exists for {}. The incoming owner must register first — the transfer stays pending until then.",
            incoming_email
        ))
    })?;

    let business = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>)>(
        "SELECT owner_id, directory_id FROM businesses WHERE id = $1 FOR UPDATE",
    )
    .bind(business_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;
    let (old_owner, old_directory) = business;

    // host_stays = true  -> the hosting directory does not change.
    // host_stays = false -> re-home into the admin-chosen target directory.
    let new_directory = if host_stays {
        old_directory
    } else {
        target.or(old_directory)
    };
    if !host_stays && new_directory.is_none() {
        return Err(AppError::Validation(
            "host_stays=false requires a target directory".to_string(),
        ));
    }

    // 1. The listing itself. When the directory changes, a category that belongs
    //    to the old directory is cleared rather than left pointing across hosts.
    sqlx::query(
        "UPDATE businesses SET owner_id = $1, directory_id = $2, \
         category_id = CASE \
             WHEN $2::uuid IS NOT DISTINCT FROM directory_id THEN category_id \
             WHEN EXISTS (SELECT 1 FROM directory_categories c WHERE c.id = businesses.category_id AND c.directory_id = $2) THEN category_id \
             ELSE NULL END, \
         updated_at = now() WHERE id = $3",
    )
    .bind(incoming_id)
    .bind(new_directory)
    .bind(business_id)
    .execute(&mut *tx)
    .await?;

    // 2. The claim row — this is what makes a portal user an "owner". The
    //    subscription attached to it is deliberately preserved.
    let claim_update = sqlx::query(
        "UPDATE claimed_businesses SET owner_email = $1, user_id = $2, \
         owner_name = COALESCE($3, owner_name), updated_at = now() WHERE business_id = $4",
    )
    .bind(&incoming_email_val)
    .bind(incoming_id)
    .bind(incoming_name.clone())
    .bind(business_id)
    .execute(&mut *tx)
    .await?;

    if claim_update.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO claimed_businesses (business_id, owner_email, owner_name, user_id, is_active, verified_at) \
             VALUES ($1, $2, $3, $4, true, now())",
        )
        .bind(business_id)
        .bind(&incoming_email_val)
        .bind(incoming_name.clone())
        .bind(incoming_id)
        .execute(&mut *tx)
        .await?;
    }

    // 3. The fee, recorded on whoever the transfer says pays it. 'platform' means
    //    the platform absorbs it: the row is kept (waived) so the audit is complete.
    if fee_cents > 0 {
        let (payer, payee, fee_status) = match fee_direction.as_str() {
            "incoming" => (Some(incoming_id), from_user_id, "payable"),
            "outgoing" => (from_user_id, Some(incoming_id), "payable"),
            _ => (None, None, "waived"),
        };
        sqlx::query(
            "INSERT INTO business_transfer_fees \
             (transfer_id, business_id, payer_user_id, payee_user_id, amount_cents, currency, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(business_id)
        .bind(payer)
        .bind(payee)
        .bind(fee_cents)
        .bind(&currency)
        .bind(fee_status)
        .execute(&mut *tx)
        .await?;
    }

    // 4. The transfer row.
    sqlx::query(
        "UPDATE business_transfers SET status = 'accepted', decided_at = now(), updated_at = now(), \
         to_user_id = $1, to_email = $2, to_tenant_id = COALESCE($3, to_tenant_id) WHERE id = $4",
    )
    .bind(incoming_id)
    .bind(&incoming_email_val)
    .bind(incoming_tenant)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    // 5. The audit event.
    write_event(
        &mut tx,
        id,
        Some(business_id),
        &claims,
        "accepted",
        Some("pending"),
        Some("accepted"),
        json!({
            "new_owner_user_id": incoming_id,
            "new_owner_email": incoming_email_val,
            "previous_owner_user_id": old_owner,
            "previous_owner_email": from_email,
            "host_stays": host_stays,
            "directory_from": old_directory,
            "directory_to": new_directory,
            "fee_cents": fee_cents,
            "currency": currency,
            "fee_direction": fee_direction,
        }),
    )
    .await?;

    tx.commit().await?;

    let row = fetch_transfer(&s.db, id).await?;
    Ok(Json(json!({
        "transfer": row,
        "owner_reassigned_to": incoming_id,
        "directory_id": new_directory,
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// POST /api/v1/transfers/:id/decline | /cancel
// ─────────────────────────────────────────────────────────────────────────────

async fn decide(
    s: &AppState,
    claims: &Claims,
    id: Uuid,
    new_status: &str,
    side: &str,
) -> ApiResult<Transfer> {
    let actor = actor_id(claims)?;
    let email = actor_email(&s.db, claims).await.unwrap_or_default();
    let admin = is_admin(claims);

    let mut tx = s.db.begin().await?;
    let (status, business_id, to_user_id, from_user_id, to_email, from_email, _, _, _, _, _, _) =
        require_pending_transfer(&mut tx, id).await?;
    if status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Transfer is already {}",
            status
        )));
    }

    let mine = if side == "incoming" {
        to_user_id == Some(actor)
            || to_email.as_deref().map(|e| e.to_lowercase()) == Some(email.clone())
    } else {
        from_user_id == Some(actor)
            || from_email.as_deref().map(|e| e.to_lowercase()) == Some(email.clone())
    };
    if !admin && !mine {
        return Err(AppError::Forbidden(format!(
            "Only the {} party or an admin can {} this transfer",
            side,
            if new_status == "declined" {
                "decline"
            } else {
                "cancel"
            }
        )));
    }

    sqlx::query(
        "UPDATE business_transfers SET status = $1, decided_at = now(), updated_at = now() WHERE id = $2",
    )
    .bind(new_status)
    .bind(id)
    .execute(&mut *tx)
    .await?;

    write_event(
        &mut tx,
        id,
        Some(business_id),
        claims,
        new_status,
        Some("pending"),
        Some(new_status),
        json!({ "by": if admin { "admin" } else { side } }),
    )
    .await?;

    tx.commit().await?;
    fetch_transfer(&s.db, id).await
}

pub async fn decline_transfer(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = decide(&s, &claims, id, "declined", "incoming").await?;
    Ok(Json(json!(row)))
}

pub async fn cancel_transfer(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let row = decide(&s, &claims, id, "cancelled", "outgoing").await?;
    Ok(Json(json!(row)))
}
