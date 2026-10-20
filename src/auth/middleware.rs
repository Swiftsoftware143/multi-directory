use super::models::Claims;
use crate::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;

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
    encode(&Header::default(), claims, &encoding_key)
}

/// Is the principal named by a token's `sub` claim still a live account?
///
/// Kanban t_fb37dfc0 (the fleet class fixed on the sibling apps by t_53fcb666). A JWT is a
/// bearer credential that `verify_token` accepts for its whole 24 h life on signature, `exp`,
/// `iss` and `aud` alone — it never looks at the row it names. So deleting (or disabling) an
/// account left its token working: measured 2026-10-20 on this app, `delete from users` and the
/// same bearer still collected 200 from `/my-reviews`, `/admin/reports`, `/admin/reports/stats`,
/// `/templates`, `/directories`, `/deals`, `/crm/deals` and `/admin/audience/overview` (only
/// `/auth/me` noticed, because that handler re-reads the user itself).
///
/// This is the ONE primary-key read that closes it, called from `routes::auth_guard` — the single
/// boundary every private route already passes through — so the refusal lands on every method and
/// on handlers that never touch `users` themselves.
///
/// TWO identity tables, because this app mints two shapes of session: staff/business/directory
/// accounts are rows in `users`, while the business portal and the public entry points sign a
/// shopper in as a `visitor` (`POST /visitor/login`, `sub = visitor_accounts.id`, `tid = nil`).
/// Both are checked against `sub` so neither kind of sign-in is broken; a `sub` that names no row
/// in either table is dead. `is_active` defaults to true and is nullable on `visitor_accounts`,
/// so only an explicit `false` disables an account.
///
/// A `sub` that is not a uuid names no row (`Ok(false)`); a database that cannot answer is
/// `Err(AppError::Database)` — 500, never a silent pass.
pub async fn principal_is_live(db: &PgPool, sub: &str) -> Result<bool, AppError> {
    let Ok(id) = Uuid::parse_str(sub) else {
        return Ok(false);
    };

    let live: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM users            WHERE id = $1 AND COALESCE(is_active, true)
             UNION ALL
             SELECT 1 FROM visitor_accounts WHERE id = $1 AND COALESCE(is_active, true)
         )",
    )
    .bind(id)
    .fetch_one(db)
    .await?;

    Ok(live)
}

#[cfg(test)]
mod principal_liveness_tests {
    use super::principal_is_live;

    /// A `sub` that is not a uuid names no row — refused before any query is issued, so a
    /// hand-crafted/garbage claim costs no database round trip. `connect_lazy` never dials.
    #[tokio::test]
    async fn a_non_uuid_subject_names_no_account() {
        let db =
            sqlx::PgPool::connect_lazy("postgres://user:pw@127.0.0.1:1/none").expect("lazy pool");
        for sub in [
            "",
            "system",
            "not-a-uuid",
            "00000000-0000-0000-0000-00000000000",
        ] {
            assert!(
                !principal_is_live(&db, sub).await.unwrap_or(false),
                "{sub:?} must not resolve to an account"
            );
        }
    }
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
