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

// ── operator support sessions (kanban t_a9c33fea) ───────────────────────────────────────────────
//
// `handlers::impersonate` mints a 1-hour token that signs in AS a tenant account, with the
// operator's id in the `impersonating` claim. Ending that session used to be a client-side
// gesture only (swap localStorage back), so the support bearer kept answering 200 on every route
// for the rest of its hour — from any tab that still held it. `impersonation_sessions` is the
// durable record that makes "ended" mean something; this is the read behind it.

/// The ONE admin path an impersonation session may reach: the one that ends it.
///
/// `operator_guard` refuses the whole operator surface to a support session (its role claim is the
/// TENANT's, never the operator's), and that refusal is correct — with exactly one exception, the
/// route whose whole purpose is to end the session. Without the exception the operator could only
/// escape by logging out, and the token they were trying to kill would outlive the attempt.
pub const ADMIN_STOP_IMPERSONATION: &str = "/api/v1/admin/stop-impersonation";

/// The same route as a `route_layer` INSIDE the `/api/v1` router sees it: axum rewrites
/// `req.uri()` when a request enters a nested router (the un-nested original is kept separately in
/// `OriginalUri`), so `operator_guard` — which is a route_layer on this route — is handed the path
/// without the nest prefix. Measured the hard way: with only the absolute spelling, the guard
/// refused the support session on this route with 403 and the stop never reached its handler.
pub const ADMIN_STOP_IMPERSONATION_NESTED: &str = "/admin/stop-impersonation";

/// True when `path` is the one admin route a support session must be able to call. Compared
/// exactly against the two spellings this app can hand a guard (absolute at the app boundary, nest
/// relative inside it), never by prefix: `/stop-impersonation/`, `/stop-impersonations` and
/// `/impersonate` are DIFFERENT routes and must stay refused.
pub fn is_stop_impersonation(path: &str) -> bool {
    path == ADMIN_STOP_IMPERSONATION || path == ADMIN_STOP_IMPERSONATION_NESTED
}

/// The operator-surface decision for one request: `true` means answer 403.
///
/// Pure, so the single carve-out is unit-tested: the exception requires BOTH the exact path AND an
/// `impersonating` claim, and only the impersonate handler ever sets that claim. A tenant's own
/// token therefore still cannot reach this route, and a support session still reaches nothing else
/// on the admin surface.
pub fn operator_surface_denied(path: &str, claims: &Claims) -> bool {
    if claims.impersonating.is_some() && is_stop_impersonation(path) {
        return false;
    }
    !is_super_admin(claims)
}

/// The durable identity of a support session: the account it signs in as, the operator who minted
/// it, and the token's own `iat`.
///
/// `iat` is second-granularity and the only claim that differs between two mints for the same pair,
/// so it names exactly one mint without adding a `jti` to [`Claims`] (a field added now would be
/// absent from every token already in the wild). `None` = not a support session at all, which is
/// why an ordinary sign-in pays no query for this check.
pub fn impersonation_session_key(claims: &Claims) -> Option<(Uuid, Uuid, i64)> {
    let operator = Uuid::parse_str(claims.impersonating.as_deref()?).ok()?;
    let target = Uuid::parse_str(&claims.sub).ok()?;
    Some((target, operator, claims.iat as i64))
}

/// Has the support session this token belongs to been ENDED?
///
/// `Ok(false)` for an ordinary sign-in (no query) and for a support session this app has no record
/// of — a token minted before `impersonation_sessions` existed, i.e. the deploy window. The
/// signature is the trust anchor either way; the row exists to make ENDING enforceable, and
/// `stop_impersonation` writes a tombstone even for a session it has no row for, so the first
/// attempt to end one is enough.
///
/// `Ok(true)` also for an `impersonating` claim whose pair is not addressable: a token naming no
/// real (account, operator) pair cannot be one of this app's mints, and a credential that cannot be
/// proven live is refused.
pub async fn impersonation_session_ended(db: &PgPool, claims: &Claims) -> Result<bool, AppError> {
    if claims.impersonating.is_none() {
        return Ok(false);
    }
    let Some((target_user_id, operator_id, token_iat)) = impersonation_session_key(claims) else {
        return Ok(true);
    };

    let stopped: Option<bool> = sqlx::query_scalar(
        "SELECT stopped_at IS NOT NULL FROM impersonation_sessions \
          WHERE target_user_id = $1 AND operator_id = $2 AND token_iat = $3 LIMIT 1",
    )
    .bind(target_user_id)
    .bind(operator_id)
    .bind(token_iat)
    .fetch_optional(db)
    .await?;

    Ok(stopped.unwrap_or(false))
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
        Ok(Some(state)) if session_is_acceptable(&state, &claims) => {
            // kanban t_a9c33fea — the FOURTH arm: a support session the operator has ENDED is dead
            // even though its account is live and its signature is fine. Only tokens carrying an
            // `impersonating` claim reach the lookup, so an ordinary sign-in pays nothing for it.
            match impersonation_session_ended(db, &claims).await {
                Ok(false) => SessionVerdict::Live(claims),
                Ok(true) => {
                    tracing::warn!(
                        "support session already ended for sub={} operator={:?}",
                        claims.sub,
                        claims.impersonating
                    );
                    SessionVerdict::Dead
                }
                Err(e) => {
                    tracing::error!(
                        "support-session state lookup failed for {}: {e}",
                        claims.sub
                    );
                    SessionVerdict::Unknown
                }
            }
        }
        Ok(_) => SessionVerdict::Dead,
        Err(e) => {
            tracing::error!("session state lookup failed for {}: {e}", claims.sub);
            SessionVerdict::Unknown
        }
    }
}

#[cfg(test)]
mod support_session_tests {
    use super::{
        impersonation_session_key, is_stop_impersonation, operator_surface_denied,
        ADMIN_STOP_IMPERSONATION,
    };
    use crate::auth::models::Claims;

    fn claims(role: &str, sub: &str, impersonating: Option<&str>) -> Claims {
        Claims {
            sub: sub.to_string(),
            tid: "11111111-1111-1111-1111-111111111111".to_string(),
            role: role.to_string(),
            exp: 2_000_000_000,
            iat: 1_700_000_000,
            aud: Some("multidirectory-api".to_string()),
            iss: Some("multidirectory".to_string()),
            impersonating: impersonating.map(str::to_string),
        }
    }

    const TARGET: &str = "22222222-2222-2222-2222-222222222222";
    const OPERATOR: &str = "33333333-3333-3333-3333-333333333333";

    /// The carve-out is ONE path, spelled exactly, and only for a support session. Everything else
    /// on the admin surface stays refused to a tenant-role claim — including the near-misses, so a
    /// later route named `/stop-impersonations` is not silently admitted.
    #[test]
    fn a_support_session_may_reach_the_route_that_ends_it_and_nothing_else() {
        let support = claims("admin", TARGET, Some(OPERATOR));
        assert!(
            !operator_surface_denied(ADMIN_STOP_IMPERSONATION, &support),
            "a support session must be able to end itself"
        );
        assert!(is_stop_impersonation(ADMIN_STOP_IMPERSONATION));
        // Both spellings a guard can be handed: absolute at the app boundary, nest-relative inside
        // the `/api/v1` router.
        assert!(is_stop_impersonation("/admin/stop-impersonation"));
        for near in [
            "/api/v1/admin/stop-impersonation/",
            "/api/v1/admin/stop-impersonation/anything",
            "/admin/stop-impersonation/",
            "/admin/stop-impersonations",
            "/api/v1/admin/stop-impersonate",
            "/api/v1/admin/stop-impersonations",
            "/api/v1/admin/impersonate",
            "/api/v1/admin/tenants",
            "/api/v1/admin",
        ] {
            assert!(
                !is_stop_impersonation(near),
                "{near} must not be treated as the stop route"
            );
            assert!(
                operator_surface_denied(near, &support),
                "{near} must stay refused to a support session"
            );
        }
    }

    /// A tenant's own token cannot use the carve-out: it carries no `impersonating` claim, so even
    /// the exact stop path is refused to it before the handler runs.
    #[test]
    fn a_real_tenant_session_is_still_refused_the_operator_surface() {
        let tenant = claims("admin", TARGET, None);
        assert!(operator_surface_denied(ADMIN_STOP_IMPERSONATION, &tenant));
        assert!(operator_surface_denied("/api/v1/admin/tenants", &tenant));
        // …and an operator is admitted everywhere, which is what the guard did before the carve-out.
        let operator = claims("super_admin", OPERATOR, None);
        assert!(!operator_surface_denied(
            ADMIN_STOP_IMPERSONATION,
            &operator
        ));
        assert!(!operator_surface_denied("/api/v1/admin/tenants", &operator));
    }

    /// The durable key names one mint: (account, operator, token iat). An ordinary sign-in has no
    /// key at all, so the revocation read is skipped for it, and a claim whose pair is not
    /// addressable cannot produce a spurious match.
    #[test]
    fn the_support_session_key_is_the_mint_triple() {
        let support = claims("admin", TARGET, Some(OPERATOR));
        let key = impersonation_session_key(&support).expect("a support session has a key");
        assert_eq!(key.0.to_string(), TARGET);
        assert_eq!(key.1.to_string(), OPERATOR);
        assert_eq!(key.2, 1_700_000_000);

        assert!(impersonation_session_key(&claims("admin", TARGET, None)).is_none());
        assert!(
            impersonation_session_key(&claims("admin", "not-a-uuid", Some(OPERATOR))).is_none()
        );
        assert!(impersonation_session_key(&claims("admin", TARGET, Some("nope"))).is_none());
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
