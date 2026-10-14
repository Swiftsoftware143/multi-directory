use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use super::models::Claims;
use crate::error::AppError;
use crate::AppState;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn auth_middleware(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, &s.config.jwt_secret).map_err(|_| AppError::Unauthorized)?;

    let mut req = req;
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

    let decoding_key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 30;
    validation.validate_exp = true;
    validation.set_issuer(&["multidirectory"]);
    validation.set_audience(&["multidirectory-api"]);

    let token_data = decode::<Claims>(token, &decoding_key, &validation)?;
    Ok(token_data.claims)
}

pub fn create_token(claims: &Claims, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    use jsonwebtoken::{encode, EncodingKey, Header};

    let encoding_key = EncodingKey::from_secret(secret.as_bytes());
    Ok(encode(&Header::default(), claims, &encoding_key)?)
}

/// Is the caller the platform operator?
///
/// The app's flat role model mints `role = "admin"` for EVERY account created through the public
/// `/auth/register` route, so `admin` is a tenant role and never an operator signal (measured
/// 2026-10-14: a stranger's ordinary signup token cleared the checkout-book, enrichment-search,
/// community-event and poll gates). Operator-only surfaces therefore test `super_admin`
/// (`handlers::tenant_scope::is_platform_operator` is the canonical form of this rule).
pub fn is_admin(claims: &Claims) -> bool {
    is_super_admin(claims)
}

/// Check if user is super_admin
pub fn is_super_admin(claims: &Claims) -> bool {
    claims.role == "super_admin"
}

/// Check if user is a business owner
pub fn is_business_owner(claims: &Claims) -> bool {
    claims.role == "business_owner"
}

/// Check if user is a visitor account
pub fn is_visitor(claims: &Claims) -> bool {
    claims.role == "visitor"
}

/// A presented API key, resolved to the session of the user who owns it.
pub struct ApiKeyIdentity {
    /// `api_keys.id` — used for usage bookkeeping and the per-minute ceiling.
    pub key_id: Uuid,
    /// The owner's session claims (subject, tenant, role).
    pub claims: Claims,
    /// The key's own per-minute ceiling. 0 means no limit.
    pub rate_limit_per_minute: i32,
}

/// Resolve a raw API key — the `md_…` value shown once on the admin "API Access" screen — to the
/// session of the user who owns it, so an issued key can actually call the API it was created for.
///
/// The visible prefix is never a credential: matching is on the SHA-256 of the FULL key, exactly
/// as it was stored at creation. `None` means unknown, revoked, expired, or a disabled owner.
/// Callers exchange the returned claims for a short-lived session token so that *every* handler
/// (both `Extension<Claims>` readers and the ones that re-read the header) sees a normal sign-in.
pub async fn resolve_api_key(db: &PgPool, raw: &str) -> Option<ApiKeyIdentity> {
    let key_hash =
        hex::encode(ring::digest::digest(&ring::digest::SHA256, raw.as_bytes()).as_ref());

    let row: Option<(Uuid, Uuid, Uuid, String, Option<i32>)> = sqlx::query_as(
        "SELECT k.id, k.user_id, u.tenant_id, u.role, k.rate_limit_per_minute
           FROM api_keys k
           JOIN users u ON u.id = k.user_id
          WHERE k.key_hash = $1
            AND k.is_active = true
            AND u.is_active = true
            AND (k.expires_at IS NULL OR k.expires_at > NOW())
          LIMIT 1",
    )
    .bind(&key_hash)
    .fetch_optional(db)
    .await
    .ok()?;

    let (key_id, user_id, tenant_id, role, limit) = row?;
    let now_ts = chrono::Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        tid: tenant_id.to_string(),
        role,
        // The minted session is short-lived; the key's own `expires_at` governs real validity.
        exp: now_ts + 3600,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };

    Some(ApiKeyIdentity {
        key_id,
        claims,
        rate_limit_per_minute: limit.unwrap_or(0),
    })
}
