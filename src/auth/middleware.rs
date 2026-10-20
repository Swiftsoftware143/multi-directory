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

/// The state of the identity row a token's `sub` names — the ONE read behind every session check
/// on this app (kanban t_fb37dfc0 deleted/disabled accounts, t_dd9d99c9 password rotation).
pub struct SessionState {
    /// The row exists and is enabled. `is_active` defaults to true and is nullable on
    /// `visitor_accounts`, so only an explicit `false` disables an account.
    pub is_active: bool,
    /// `users.sessions_valid_from` as a unix epoch: the instant this account last changed or reset
    /// its password. `None` = never rotated, which is every account until it rotates once.
    pub valid_from: Option<i64>,
    /// The role the row grants RIGHT NOW. A token carries the role it was minted with, so a
    /// demoted operator's 24 h token must be compared against this — `operator_guard` reads the
    /// claim, and without this arm a demotion revoked nothing.
    pub role: String,
}

/// Read the identity row a token's `sub` claim names.
///
/// TWO identity tables, because this app mints two shapes of session: staff/business/directory
/// accounts are rows in `users`, while the business portal and the public entry points sign a
/// shopper in as a `visitor` (`POST /visitor/login`, `sub = visitor_accounts.id`, `tid = nil`).
/// Both are read here so neither kind of sign-in is broken; a `sub` that names no row in either
/// table is `Ok(None)` — dead.
///
/// A `sub` that is not a uuid names no row, answered without a database round trip (`Ok(None)`).
/// A database that cannot answer is `Err(AppError::Database)` — 500, never a silent pass.
///
/// Only `users` carries `sessions_valid_from`: `visitor_accounts` has no password writer (its hash
/// is set once by INSERT at registration and never updated), so its arm contributes NULL.
pub async fn session_state(db: &PgPool, sub: &str) -> Result<Option<SessionState>, AppError> {
    let Ok(id) = Uuid::parse_str(sub) else {
        return Ok(None);
    };

    let row: Option<(bool, Option<i64>, String)> = sqlx::query_as(
        "SELECT COALESCE(is_active, true),
                floor(EXTRACT(EPOCH FROM sessions_valid_from))::bigint,
                COALESCE(role, '')
           FROM users            WHERE id = $1
         UNION ALL
         SELECT COALESCE(is_active, true), NULL::bigint, 'visitor'::text
           FROM visitor_accounts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|(is_active, valid_from, role)| SessionState {
        is_active,
        valid_from,
        role,
    }))
}

/// Is the principal named by a token's `sub` claim still a live account?
///
/// A JWT is a bearer credential that `verify_token` accepts for its whole 24 h life on signature,
/// `exp`, `iss` and `aud` alone — it never looks at the row it names. So deleting (or disabling) an
/// account left its token working: measured 2026-10-20 on this app, `delete from users` and the
/// same bearer still collected 200 from `/my-reviews`, `/admin/reports`, `/admin/reports/stats`,
/// `/templates`, `/directories`, `/deals`, `/crm/deals` and `/admin/audience/overview` (only
/// `/auth/me` noticed, because that handler re-reads the user itself).
///
/// (kanban t_fb37dfc0 — the liveness half of [`session_state`], which is what the boundaries read;
/// this predicate survives only as the readable name for "does the row still exist".)
#[cfg(test)]
pub async fn principal_is_live(db: &PgPool, sub: &str) -> Result<bool, AppError> {
    Ok(session_state(db, sub)
        .await?
        .map(|s| s.is_active)
        .unwrap_or(false))
}

/// Does a session minted at `jwt_iat` predate the instant its account last changed or reset its
/// password? (kanban t_dd9d99c9)
///
/// Pure, so both arms are unit-tested. `valid_from` is `users.sessions_valid_from` as a unix epoch
/// (`None` = never restricted, so every token already in the wild keeps working). A session with no
/// `iat` cannot prove it is newer than the change, and the safe answer for a credential is "cannot
/// prove → refuse". `iat` is second-granularity (and the change's own timestamp is floored to a
/// second for this reason), so a session minted in the SAME second as the change cannot be told
/// apart from a re-login and is allowed.
pub fn session_predates_password_change(jwt_iat: Option<i64>, valid_from: Option<i64>) -> bool {
    match valid_from {
        None => false,
        Some(v) => match jwt_iat {
            None => true,
            Some(iat) => iat < v,
        },
    }
}

/// Does a token's role CLAIM exceed the authority its row still grants? (kanban t_dd9d99c9, the
/// sibling of the fleet's t_28184826 demotion arm)
///
/// `super_admin` is this app's platform operator (`tenant_scope::is_platform_operator`) and
/// `operator_guard` decides from `claims.role` alone, so a demoted operator's token kept the
/// operator surface (`GET /admin/network/counts` answered 200 for a row that had been set back to
/// `user` — measured 2026-10-20). Pure, so both arms are unit-tested; only a claim that reaches
/// HIGHER than the row is a violation — a promotion mid-session is not.
pub fn platform_claim_outranks_row(claim_role: &str, row_role: &str) -> bool {
    claim_role == "super_admin" && row_role != "super_admin"
}

/// What a presented bearer token is worth right now.
pub enum SessionVerdict {
    /// The token is ours, it names a live account, and it was minted after that account's last
    /// password change.
    Live(Claims),
    /// Refuse: bad signature/claims, no such account, disabled account, or a session minted
    /// before the account's password change.
    Dead,
    /// The identity row could not be read (database unavailable). Never a pass, but not proof of
    /// a dead session either — the caller must not mistake an outage for a revocation.
    Unknown,
}

/// Is this token acceptable for this row right now? The ONE rule, shared by
/// [`crate::routes::session_gate`] (via [`session_verdict`]) and [`crate::routes::auth_guard`], so
/// the two boundaries can never disagree about what a live session is.
///
/// Three arms, all measured on this app: the row must still exist and be enabled (t_fb37dfc0), the
/// token must not predate the row's last password change (t_dd9d99c9), and its role claim must not
/// reach above what the row still grants (t_28184826 class).
pub fn session_is_acceptable(state: &SessionState, claims: &Claims) -> bool {
    state.is_active
        && !session_predates_password_change(Some(claims.iat as i64), state.valid_from)
        && !platform_claim_outranks_row(&claims.role, &state.role)
}

/// Verify a bearer token AND the session behind it.
///
/// This is the credential check used by [`crate::routes::session_gate`], which sits outside every
/// route — the handlers on this app's public-but-personalising surface (`GET /reviews`,
/// `GET /feed`, `/visitor/*`, `/bookmarks`, `/saved-places`, the RFQ/`/b2b` readers …) parse the
/// `Authorization` header themselves and never pass through `auth_guard`, so a revocation checked
/// only at that middleware would be a second door left open.
///
/// An API key (`md_…`) is NOT a session and must never be measured against `sessions_valid_from`;
/// the gate skips those before calling this.
pub async fn session_verdict(db: &PgPool, token: &str, secret: &str) -> SessionVerdict {
    let Ok(claims) = verify_token(token, secret) else {
        return SessionVerdict::Dead;
    };

    match session_state(db, &claims.sub).await {
        Ok(Some(state)) if session_is_acceptable(&state, &claims) => SessionVerdict::Live(claims),
        Ok(_) => SessionVerdict::Dead,
        Err(e) => {
            tracing::error!("session state lookup failed for {}: {e}", claims.sub);
            SessionVerdict::Unknown
        }
    }
}

#[cfg(test)]
mod principal_liveness_tests {
    use super::{platform_claim_outranks_row, principal_is_live, session_predates_password_change};

    /// A `sub` that is not a uuid names no row — refused before any query is issued, so a
    /// hand-crafted/garbage claim costs no database round trip. `connect_lazy` never dials.
    #[tokio::test]
    async fn a_non_uuid_subject_names_no_account() {
        let db =
            sqlx::PgPool::connect_lazy("postgres://user:***@127.0.0.1:1/none").expect("lazy pool");
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

    /// kanban t_dd9d99c9 — both arms of the session-revocation predicate.
    #[test]
    fn a_session_is_refused_only_when_it_predates_the_password_change() {
        // Never rotated: every token in the wild keeps working (additive change).
        assert!(!session_predates_password_change(Some(1_000), None));
        // Minted before the change: refused.
        assert!(session_predates_password_change(Some(1_000), Some(2_000)));
        // Minted in the SAME second as the change: allowed — `iat` is second-granularity, so a
        // re-login and a pre-change session cannot be told apart.
        assert!(!session_predates_password_change(Some(2_000), Some(2_000)));
        // Minted after the change: allowed.
        assert!(!session_predates_password_change(Some(3_000), Some(2_000)));
        // A session that cannot prove when it was minted is refused when the account HAS rotated.
        assert!(session_predates_password_change(None, Some(2_000)));
        // …and is harmless when it never has.
        assert!(!session_predates_password_change(None, None));
    }

    /// kanban t_dd9d99c9 — a role CLAIM may not reach above the row.
    #[test]
    fn a_role_claim_above_the_row_is_refused() {
        // The demotion this arm exists for: minted as the operator, row set back to `user`.
        assert!(platform_claim_outranks_row("super_admin", "user"));
        assert!(platform_claim_outranks_row("super_admin", "admin"));
        // …and the honest cases: unchanged, promoted mid-session, or an ordinary account.
        assert!(!platform_claim_outranks_row("super_admin", "super_admin"));
        assert!(!platform_claim_outranks_row("user", "user"));
        assert!(!platform_claim_outranks_row("visitor", "visitor"));
        assert!(!platform_claim_outranks_row("user", "super_admin"));
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
