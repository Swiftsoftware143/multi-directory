//! Account API keys — minted on account creation, retrievable on demand (kanban t_7863696b).
//!
//! David, 2026-11-11 (correcting the earlier "shown once, never again" draft): a key is stored
//! TWICE and both halves have exactly one job.
//!
//!   * `api_keys.key_hash`      — SHA-256 hex. Authentication (`routes.rs`, the api-key middleware)
//!                                reads this and NOTHING else.
//!   * `api_keys.key_encrypted` — the same key sealed with the fleet provider-key envelope
//!                                (`enc:v1:…`, pgp_sym_encrypt under `PROVIDER_KEY_ENC_SECRET`).
//!                                Read ONLY to show the key back to its owner.
//!
//! Authentication never decrypts; reveal never authenticates. A reveal re-checks the caller's own
//! password first (a stolen session token is not enough) and writes an `api_key_reveals` row.
//!
//! Every mint goes through [`mint_on`], so the registration transaction, the admin "create key"
//! action, the regenerate action and the one-time backfill can never disagree about the shape.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::security::provider_key_crypto as crypto;
use crate::AppState;

/// A key that was just minted. `raw_key` is the plaintext; it is sealed before the row is written
/// and handed to the caller's UI, never to a log.
pub struct MintedKey {
    pub id: Uuid,
    pub name: String,
    pub raw_key: String,
    pub prefix: String,
}

fn generate() -> (String, String, String) {
    // Same generator the admin "create key" action uses — one key format, one place.
    crate::handlers::api_complete::generate_api_key()
}

/// Mint one key for an account and INSERT it on the caller's connection.
///
/// The registration path calls this INSIDE its transaction: a rolled-back signup leaves no key
/// row, and a committed one never ends without a key.
pub async fn mint_on(
    conn: &mut PgConnection,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    name: &str,
) -> Result<MintedKey, AppError> {
    let (full, hash, prefix) = generate();
    // Fail closed: without `PROVIDER_KEY_ENC_SECRET` the seal errors instead of storing the key in
    // the clear — and an account is never left holding a key nobody can ever see again.
    let sealed = crypto::seal_in(&mut *conn, &full)
        .await
        .map_err(|e| AppError::Internal(format!("could not seal the API key: {e}")))?;

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, tenant_id, name, key_hash, key_prefix, key_encrypted)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(name)
    .bind(&hash)
    .bind(&prefix)
    .bind(&sealed)
    .execute(&mut *conn)
    .await?;

    Ok(MintedKey {
        id,
        name: name.to_string(),
        raw_key: full,
        prefix,
    })
}

#[derive(Deserialize)]
pub struct RevealRequest {
    /// The caller's CURRENT password — a stolen session token must not be enough to read a live
    /// credential.
    pub password: String,
}

fn caller(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<(crate::auth::models::Claims, Uuid, Option<Uuid>), AppError> {
    let claims =
        crate::handlers::tenant_scope::claims_from_headers(headers, &state.config.jwt_secret)?;
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;
    let tenant_id = claims.tid.parse::<Uuid>().ok().filter(|t| !t.is_nil());
    Ok((claims, user_id, tenant_id))
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string())
        })
}

/// POST /api/v1/admin/api-keys/:id/reveal — show the key again (repeatable).
///
/// The plaintext is decrypted here and nowhere else on a read path. Authentication still hashes the
/// presented key and compares `key_hash`.
pub async fn reveal_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<RevealRequest>,
) -> ApiResult<impl IntoResponse> {
    let (claims, user_id, tenant_id) = caller(&headers, &state)?;

    let row = sqlx::query(
        "SELECT user_id, key_prefix, key_encrypted FROM api_keys
         WHERE id = $1 AND is_active = true",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    // The key's OWNER may reveal it; the platform operator console may too (it is the surface that
    // issues the keys). Anyone else is refused before the password is even read.
    let owner_id: Option<Uuid> = row.get("user_id");
    if owner_id != Some(user_id)
        && crate::auth::middleware::operator_surface_denied("/api/v1/admin/api-keys", &claims)
    {
        return Err(AppError::Forbidden(
            "This API key belongs to another account".to_string(),
        ));
    }

    let prefix: String = row.get("key_prefix");
    let sealed: Option<String> = row.get("key_encrypted");

    // ── re-authenticate ────────────────────────────────────────────────────────────────────────
    let stored: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1 AND is_active = true")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    let stored = stored.ok_or(AppError::Unauthorized)?;
    if !crate::security::password_check::matches(&stored, &req.password) {
        return Err(AppError::InvalidCredentials);
    }

    let sealed = sealed.ok_or_else(|| {
        AppError::Conflict(
            "This key was created before keys were stored recoverably, so it cannot be displayed. \
             Regenerate it to get a key you can view and copy."
                .to_string(),
        )
    })?;
    let full = crypto::open_in(&state.db, &sealed).await.map_err(|e| {
        tracing::error!(error = %e, key = %id, "API key reveal: sealed copy could not be opened");
        AppError::Internal(
            "This key could not be opened by the server's encryption key. Regenerate it."
                .to_string(),
        )
    })?;
    if full.is_empty() {
        return Err(AppError::Internal(
            "This key could not be opened by the server's encryption key. Regenerate it."
                .to_string(),
        ));
    }

    if let Err(e) = sqlx::query(
        "INSERT INTO api_key_reveals (id, key_id, tenant_id, user_id, key_prefix, ip_address, user_agent)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(&prefix)
    .bind(client_ip(&headers))
    .bind(headers.get("user-agent").and_then(|v| v.to_str().ok()))
    .execute(&state.db)
    .await
    {
        tracing::error!(error = %e, key = %id, "API key reveal: audit row could not be written");
    }
    tracing::info!(
        key = %id,
        key_prefix = %prefix,
        user = %user_id,
        ip = ?client_ip(&headers),
        "API key REVEALED"
    );

    Ok(Json(
        json!({ "id": id.to_string(), "prefix": prefix, "key": full }),
    ))
}

/// POST /api/v1/admin/api-keys/:id/regenerate — one atomic revoke-and-issue.
///
/// Insert the new key, delete the old one, commit once. A failure at either step rolls both back,
/// so the customer is never left with no key at all; the old key keeps working until the swap
/// commits and stops working on the very next request after it.
pub async fn regenerate_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let (claims, user_id, tenant_id) = caller(&headers, &state)?;

    let row = sqlx::query(
        "SELECT user_id, tenant_id, name FROM api_keys WHERE id = $1 AND is_active = true",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let row = row.ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;
    let owner_id: Option<Uuid> = row.get("user_id");
    if owner_id != Some(user_id)
        && crate::auth::middleware::operator_surface_denied("/api/v1/admin/api-keys", &claims)
    {
        return Err(AppError::Forbidden(
            "This API key belongs to another account".to_string(),
        ));
    }
    // The replacement stays with the same owner and tenant as the key it replaces.
    let owner_id = owner_id.unwrap_or(user_id);
    let tenant_id: Option<Uuid> = row.get::<Option<Uuid>, _>("tenant_id").or(tenant_id);
    let name: String = row.get("name");

    let mut tx = state.db.begin().await?;
    let minted = mint_on(&mut tx, owner_id, tenant_id, &name).await?;
    sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    tracing::info!(
        new_key = %minted.id,
        old_key = %id,
        user = %user_id,
        key_prefix = %minted.prefix,
        "API key REGENERATED — the previous key no longer authenticates"
    );

    Ok(Json(json!({
        "id": minted.id.to_string(),
        "rotated_from": id.to_string(),
        "name": minted.name,
        "prefix": minted.prefix,
        "key": minted.raw_key,
        "note": "The previous key no longer works. This one can be viewed and copied again at any time."
    })))
}

/// One-time backfill: every account that exists today gets a key it can view.
///
/// Gated on the `fleet_data_backfills` marker so it runs ONCE — a customer who deliberately
/// revoked every key must not have one silently re-minted on the next boot.
pub async fn backfill_missing_keys(db: &PgPool) -> Result<usize, AppError> {
    const MARKER: &str = "api_keys_auto_v1";

    let done: Option<String> =
        sqlx::query_scalar("SELECT name FROM fleet_data_backfills WHERE name = $1")
            .bind(MARKER)
            .fetch_optional(db)
            .await?;
    if done.is_some() {
        return Ok(0);
    }

    let mut tx = db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7263114002)")
        .execute(&mut *tx)
        .await?;

    let users: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT u.id, u.tenant_id FROM users u
         WHERE u.is_active = true
           AND NOT EXISTS (
                SELECT 1 FROM api_keys k WHERE k.user_id = u.id AND k.is_active = true
           )
         ORDER BY u.created_at",
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut minted = 0usize;
    for (user_id, tenant_id) in users {
        mint_on(&mut tx, user_id, tenant_id, "auto-provisioned").await?;
        minted += 1;
    }

    let unrecoverable: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM api_keys WHERE is_active = true AND key_encrypted IS NULL",
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO fleet_data_backfills (name, detail) VALUES ($1, $2)
         ON CONFLICT (name) DO NOTHING",
    )
    .bind(MARKER)
    .bind(format!(
        "minted {minted} key(s) for accounts with none; {unrecoverable} pre-existing key(s) have no \
         sealed copy (plaintext unrecoverable — regenerate to hand off)"
    ))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(minted)
}

/// The account's own key, for the settings screen: prefix, created, last used, and whether the
/// View control can show it.
pub async fn my_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let (_claims, user_id, _tenant) = caller(&headers, &state)?;
    let rows = sqlx::query(
        "SELECT id, name, key_prefix, is_active, last_used_at, created_at,
                (key_encrypted IS NOT NULL) AS recoverable
         FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    let keys: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<String, _>("name"),
                "prefix": r.get::<String, _>("key_prefix"),
                "is_active": r.get::<bool, _>("is_active"),
                "last_used_at": r.get::<Option<DateTime<Utc>>, _>("last_used_at"),
                "created_at": r.get::<DateTime<Utc>, _>("created_at"),
                "recoverable": r.get::<bool, _>("recoverable"),
            })
        })
        .collect();

    Ok(Json(json!({ "keys": keys })))
}
