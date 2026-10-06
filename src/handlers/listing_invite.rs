//! Supplier onboarding invite link — card B82.
//!
//! The operator hands a draft/prospect record (card B81) to the supplier who owns it: the
//! Businesses tab mints a ONE-TIME link bound to THAT business, the supplier opens it and
//! completes the listing. Completing ADOPTS the existing record — it UPDATEs the draft in
//! place and links an owner account to it — instead of inserting a second business, so a
//! prospect that is claimed becomes ONE row, never a duplicate. The same submit also
//! establishes the self-serve supplier session, so the supplier lands logged in.
//!
//! Three endpoints:
//!   * `POST /admin/businesses/:id/listing-invite`      — operator-guarded: mint a link.
//!   * `GET  /listing-invites/:token`                   — public: prefill the complete form.
//!   * `POST /listing-invites/:token/complete`          — public: adopt + create the account.
//!
//! The token is the credential (same model as the claim-verification token, migration 141), so
//! the link works from a mail client with no session. See `is_public` in `routes.rs` for the
//! allowlist entries; the admin mint stays behind `operator_guard`.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::create_token;
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// Public page the invite lands on (served from the bind-mounted frontend dir).
const COMPLETE_PAGE: &str = "/complete-listing.html";

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    /// Optional: pre-fill / restrict the invite to one address (shown, not enforced).
    pub email: Option<String>,
    /// Link lifetime in days (default 30, clamped 1..365).
    pub expires_days: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CompleteInviteRequest {
    pub password: String,
    pub contact_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub description: Option<String>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
}

/// Absolute origin of this request, honouring the reverse proxy, so the minted link is copyable.
fn request_origin(headers: &HeaderMap) -> String {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(',').next().unwrap_or(h).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "zaarhub.com".to_string());
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|p| p.split(',').next().unwrap_or(p).trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "https".to_string());
    format!("{}://{}", proto, host)
}

/// Best-effort operator id from the (operator-guarded) bearer token; never fatal.
fn operator_id(state: &AppState, headers: &HeaderMap) -> Option<Uuid> {
    let raw = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))?;
    let claims = crate::auth::middleware::verify_token(raw, &state.config.jwt_secret).ok()?;
    Uuid::parse_str(&claims.sub).ok()
}

/// `POST /admin/businesses/:id/listing-invite` — operator mints a one-time completion link.
pub async fn create_listing_invite(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(business_id): Path<Uuid>,
    Json(req): Json<CreateInviteRequest>,
) -> ApiResult<impl IntoResponse> {
    let biz_name: String = sqlx::query_scalar("SELECT name FROM businesses WHERE id = $1")
        .bind(business_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Business not found".to_string()))?;

    let days = req.expires_days.unwrap_or(30).clamp(1, 365);
    let email = req
        .email
        .as_deref()
        .filter(|e| !e.trim().is_empty())
        .and_then(|e| crate::security::email_addr::normalize(e).ok());

    let token = Uuid::new_v4().simple().to_string();
    let id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO listing_invites \
             (id, business_id, token, email, created_by, expires_at, probe_harness) \
         VALUES ($1, $2, $3, $4, $5, now() + ($6::int * interval '1 day'), $7)",
    )
    .bind(id)
    .bind(business_id)
    .bind(&token)
    .bind(&email)
    .bind(operator_id(&s, &headers))
    .bind(days as i32)
    .bind(crate::probe_harness::from_headers(&headers))
    .execute(&s.db)
    .await
    .map_err(|e| AppError::Internal(format!("Could not create the invite: {e}")))?;

    let path = format!("{}?invite={}", COMPLETE_PAGE, token);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "business_id": business_id,
            "business_name": biz_name,
            "token": token,
            "path": path,
            "url": format!("{}{}", request_origin(&headers), path),
            "expires_in_days": days,
        })),
    ))
}

/// `GET /listing-invites/:token` — public prefill for the "complete your listing" form.
pub async fn get_listing_invite(
    State(s): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let row = sqlx::query(
        "SELECT i.business_id, i.email, \
                (i.used_at IS NOT NULL) AS used, \
                (i.expires_at IS NOT NULL AND i.expires_at < now()) AS expired, \
                b.name, b.description, b.address, b.city, b.state, b.zip, \
                b.phone, b.email AS business_email, b.website, COALESCE(b.status, 'active') AS status, \
                d.slug AS directory_slug, d.name AS directory_name \
         FROM listing_invites i \
         JOIN businesses b ON b.id = i.business_id \
         LEFT JOIN directories d ON d.id = b.directory_id \
         WHERE i.token = $1",
    )
    .bind(&token)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("This invite link is not valid.".to_string()))?;

    if row.try_get::<bool, _>("used").unwrap_or(false) {
        return Err(AppError::BadRequest(
            "This invite link has already been used.".to_string(),
        ));
    }
    if row.try_get::<bool, _>("expired").unwrap_or(false) {
        return Err(AppError::BadRequest(
            "This invite link has expired.".to_string(),
        ));
    }

    let pick = |k: &str| -> Option<String> { row.try_get::<Option<String>, _>(k).ok().flatten() };

    Ok((
        StatusCode::OK,
        Json(json!({
            "valid": true,
            "business_id": row.try_get::<Uuid, _>("business_id").ok(),
            "name": pick("name"),
            "description": pick("description"),
            "address": pick("address"),
            "city": pick("city"),
            "state": pick("state"),
            "zip": pick("zip"),
            "phone": pick("phone"),
            "email": pick("business_email").or_else(|| pick("email")),
            "website": pick("website"),
            "status": pick("status"),
            "directory_slug": pick("directory_slug"),
            "directory_name": pick("directory_name"),
        })),
    ))
}

/// `POST /listing-invites/:token/complete` — public: ADOPT the draft business and log the
/// supplier in. One transaction: update the business, create/reuse the account, (re)link the
/// claim, burn the invite.
pub async fn complete_listing_invite(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(req): Json<CompleteInviteRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.password.len() < 6 {
        return Err(AppError::Validation(
            "Password must be at least 6 characters".to_string(),
        ));
    }

    let mut tx =
        s.db.begin()
            .await
            .map_err(|e| AppError::Internal(format!("Could not start completion: {e}")))?;

    // Lock the invite so two concurrent submissions cannot both complete it.
    let inv = sqlx::query(
        "SELECT id, business_id, email, \
                (used_at IS NOT NULL) AS used, \
                (expires_at IS NOT NULL AND expires_at < now()) AS expired \
         FROM listing_invites WHERE token = $1 FOR UPDATE",
    )
    .bind(&token)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound("This invite link is not valid.".to_string()))?;

    if inv.try_get::<bool, _>("used").unwrap_or(false) {
        return Err(AppError::BadRequest(
            "This invite link has already been used.".to_string(),
        ));
    }
    if inv.try_get::<bool, _>("expired").unwrap_or(false) {
        return Err(AppError::BadRequest(
            "This invite link has expired.".to_string(),
        ));
    }

    let invite_id: Uuid = inv.try_get("id")?;
    let business_id: Uuid = inv.try_get("business_id")?;
    let invite_email: Option<String> = inv.try_get("email")?;

    // Identity for the account: prefer what the supplier typed, else the invite's address.
    let raw_email = req
        .email
        .as_deref()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .or(invite_email)
        .ok_or_else(|| {
            AppError::Validation(
                "An email address is required to complete the listing.".to_string(),
            )
        })?;
    let email = crate::security::email_addr::normalize(&raw_email).map_err(AppError::Validation)?;

    let probe = crate::probe_harness::from_headers(&headers);

    use argon2::{
        password_hash::{
            rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
        },
        Argon2,
    };
    let argon2 = Argon2::default();

    // Reuse an existing account for this email ONLY if the supplied password proves ownership
    // (adopt, don't duplicate); otherwise a fresh account is created.
    let existing = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, password_hash FROM visitor_accounts WHERE lower(email) = $1 LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?;

    let (visitor_id, created_account) = match existing {
        Some((vid, hash)) => {
            let ok = PasswordHash::new(&hash)
                .ok()
                .map(|parsed| {
                    argon2
                        .verify_password(req.password.as_bytes(), &parsed)
                        .is_ok()
                })
                .unwrap_or(false);
            if !ok {
                return Err(AppError::Duplicate(
                    "An account with this email already exists. Enter its correct password to \
                     claim this listing, or use a different email."
                        .to_string(),
                ));
            }
            (vid, false)
        }
        None => {
            let salt = SaltString::generate(&mut OsRng);
            let password_hash = argon2
                .hash_password(req.password.as_bytes(), &salt)
                .map_err(|e| AppError::Hash(e.to_string()))?
                .to_string();
            let visitor = sqlx::query_as::<_, crate::handlers::portal::VisitorAccount>(
                "INSERT INTO visitor_accounts \
                     (email, password_hash, name, phone, business_type) \
                 VALUES ($1, $2, $3, $4, \
                     (SELECT business_type FROM businesses WHERE id = $5)) \
                 RETURNING *",
            )
            .bind(&email)
            .bind(&password_hash)
            .bind(&req.contact_name)
            .bind(&req.phone)
            .bind(business_id)
            .fetch_one(&mut *tx)
            .await?;
            (visitor.id, true)
        }
    };

    // ADOPT the draft record — update in place, never insert a second business.
    sqlx::query(
        "UPDATE businesses SET \
             name        = COALESCE(NULLIF($2, ''), name), \
             description = COALESCE(NULLIF($3, ''), description), \
             address     = COALESCE(NULLIF($4, ''), address), \
             city        = COALESCE(NULLIF($5, ''), city), \
             state       = COALESCE(NULLIF($6, ''), state), \
             zip         = COALESCE(NULLIF($7, ''), zip), \
             phone       = COALESCE(NULLIF($8, ''), phone), \
             email       = COALESCE(NULLIF($9, ''), email), \
             website     = COALESCE(NULLIF($10, ''), website), \
             status      = 'active', \
             claimed     = true, \
             updated_at  = NOW() \
         WHERE id = $1",
    )
    .bind(business_id)
    .bind(req.contact_name.as_deref().map(str::trim).unwrap_or(""))
    .bind(req.description.as_deref().map(str::trim).unwrap_or(""))
    .bind(req.address.as_deref().map(str::trim).unwrap_or(""))
    .bind(req.city.as_deref().map(str::trim).unwrap_or(""))
    .bind(req.state.as_deref().map(str::trim).unwrap_or(""))
    .bind(req.zip.as_deref().map(str::trim).unwrap_or(""))
    .bind(req.phone.as_deref().map(str::trim).unwrap_or(""))
    .bind(&email)
    .bind(req.website.as_deref().map(str::trim).unwrap_or(""))
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Could not adopt the listing: {e}")))?;

    // Link the owner (one claim per business — update if a placeholder row already exists).
    sqlx::query(
        "INSERT INTO claimed_businesses \
             (business_id, owner_email, owner_name, owner_phone, verified_at, is_active, \
              visitor_account_id, probe_harness) \
         VALUES ($1, $2, $3, $4, NOW(), true, $5, $6) \
         ON CONFLICT (business_id) DO UPDATE SET \
             owner_email = EXCLUDED.owner_email, \
             owner_name = EXCLUDED.owner_name, \
             owner_phone = EXCLUDED.owner_phone, \
             verified_at = NOW(), \
             is_active = true, \
             visitor_account_id = EXCLUDED.visitor_account_id",
    )
    .bind(business_id)
    .bind(&email)
    .bind(&req.contact_name)
    .bind(&req.phone)
    .bind(visitor_id)
    .bind(&probe)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Could not link the owner: {e}")))?;

    sqlx::query(
        "UPDATE listing_invites SET used_at = NOW(), used_by_visitor_id = $2 WHERE id = $1",
    )
    .bind(invite_id)
    .bind(visitor_id)
    .execute(&mut *tx)
    .await?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("Could not complete the listing: {e}")))?;

    // Establish the self-serve supplier session (same claim shape as b2b_register).
    let visitor = sqlx::query_as::<_, crate::handlers::portal::VisitorAccount>(
        "SELECT * FROM visitor_accounts WHERE id = $1",
    )
    .bind(visitor_id)
    .fetch_one(&s.db)
    .await?;

    let now_ts = chrono::Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: visitor.id.to_string(),
        tid: Uuid::nil().to_string(),
        role: "visitor".to_string(),
        exp: now_ts + s.config.jwt_access_expiry as usize,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };
    let jwt = create_token(&claims, &s.config.jwt_secret)?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "access_token": jwt,
            "token_type": "Bearer",
            "expires_in": s.config.jwt_access_expiry,
            "adopted_business_id": business_id,
            "created_account": created_account,
            "visitor": {
                "id": visitor.id,
                "email": visitor.email,
                "name": visitor.name,
                "phone": visitor.phone,
                "business_id": business_id,
            },
        })),
    ))
}
