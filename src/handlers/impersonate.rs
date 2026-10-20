//! Operator support sessions — "sign in as a directory owner" (kanban t_498836c3).
//!
//! David's tenancy rule, leg 3: the platform operator has a SANCTIONED path into a tenant instead
//! of borrowing that tenant's password. Multi-Directory IS multi-tenant — `users.tenant_id` is the
//! tenant, a directory is owned by a user (`directories.owner_id`) and the scoped handlers filter
//! on the caller's `tid` (`/directories` already scopes a buyer tenant to the directories it owns,
//! see B120) — so the operator needs the same switch-into-tenant surface the other fleet apps
//! publish.
//!
//! Routes (all three operator-surface, guarded by `operator_guard` AND re-checked in-handler):
//!   POST /api/v1/admin/impersonate        {"tenant_id": "<uuid>"}   -> a 1-hour token scoped to that tenant
//!   POST /api/v1/admin/stop-impersonation                           -> ENDS that session, returns the operator's own
//!   GET  /api/v1/admin/tenants                                      -> the pick-list the panel renders
//!
//! Properties that matter:
//! * the minted token carries the TARGET account's own role — never the operator's — so a support
//!   session cannot widen its own reach (`operator_guard` refuses it the operator surface, with the
//!   single exception of the route that ends it);
//! * the operator's id rides in the `impersonating` claim, so a support session is distinguishable
//!   from a real sign-in (and from a leaked token);
//! * an inactive tenant is refused here rather than handed a token the scoped handlers would
//!   answer 403/404 for;
//! * every mint is logged at WARN with both identities — the audit trail;
//! * every mint is RECORDED in `impersonation_sessions` (kanban t_a9c33fea), so "stop" is a fact
//!   the boundary can enforce rather than a client-side gesture. Before that table existed the
//!   module said so in as many words — the session was stateless and "a route that returned a
//!   success it cannot enforce would be a lie" — and the cost was measured: a support bearer kept
//!   answering 200 for the rest of its hour after the operator had left, from any tab that held it.

use axum::{extract::State, http::HeaderMap, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::middleware::{create_token, impersonation_session_key};
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::handlers::tenant_scope::{claims_from_headers, is_platform_operator};
use crate::AppState;

/// Support sessions are short-lived: long enough to reproduce a report, short enough that a
/// forgotten tab is not a standing grant.
const IMPERSONATION_TTL_SECS: i64 = 3600;

#[derive(Deserialize)]
pub struct ImpersonateRequest {
    /// Target tenant. `tenant_id` is the fleet's canonical field name; `account_id` is accepted so
    /// the same body works against the apps that spell it that way, and a directory may be named
    /// directly (`directory_id` / `directory_slug`) — the panel's "sign in as this directory's
    /// owner" control does not know the tenant id it is aiming at.
    #[serde(default)]
    pub tenant_id: Option<Uuid>,
    #[serde(default)]
    pub account_id: Option<Uuid>,
    #[serde(default)]
    pub directory_id: Option<Uuid>,
    #[serde(default)]
    pub directory_slug: Option<String>,
}

/// The platform operator, verified from the bearer token inside the handler.
///
/// `operator_guard` already gates both routes; this is defence in depth, because a route added
/// outside that layer must never be able to hand a tenant's identity to a non-operator. Reading
/// the token here (rather than trusting `Extension<Claims>`) is the app's documented pattern —
/// see `handlers::tenant_scope::claims_from_headers`.
fn require_operator(headers: &HeaderMap, s: &AppState) -> Result<Claims, AppError> {
    let claims = claims_from_headers(headers, &s.config.jwt_secret)?;
    if !is_platform_operator(&claims) {
        return Err(AppError::Forbidden(
            "Platform operator access required".to_string(),
        ));
    }
    Ok(claims)
}

/// Resolve which tenant the operator is asking for, from whichever handle they gave.
async fn resolve_target(s: &AppState, req: &ImpersonateRequest) -> Result<Uuid, AppError> {
    if let Some(t) = req.tenant_id.or(req.account_id) {
        return Ok(t);
    }
    if let Some(did) = req.directory_id {
        let owner_tenant = sqlx::query_scalar::<_, Uuid>(
            "SELECT u.tenant_id FROM directories d JOIN users u ON u.id = d.owner_id \
             WHERE d.id = $1",
        )
        .bind(did)
        .fetch_optional(&s.db)
        .await?;
        return owner_tenant.ok_or_else(|| {
            AppError::NotFound("That directory has no owner account to sign in as".to_string())
        });
    }
    if let Some(slug) = req.directory_slug.as_deref().map(str::trim) {
        if !slug.is_empty() {
            let owner_tenant = sqlx::query_scalar::<_, Uuid>(
                "SELECT u.tenant_id FROM directories d JOIN users u ON u.id = d.owner_id \
                 WHERE d.slug = $1",
            )
            .bind(slug)
            .fetch_optional(&s.db)
            .await?;
            return owner_tenant.ok_or_else(|| {
                AppError::NotFound("That directory has no owner account to sign in as".to_string())
            });
        }
    }
    Err(AppError::BadRequest("tenant_id is required".to_string()))
}

/// POST /api/v1/admin/impersonate — mint a token scoped to one tenant.
pub async fn impersonate(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImpersonateRequest>,
) -> ApiResult<Json<Value>> {
    let operator = require_operator(&headers, &s)?;
    let target = resolve_target(&s, &req).await?;

    // The target must exist and be live.
    let tenant = sqlx::query("SELECT id, name, slug, is_active FROM tenants WHERE id = $1")
        .bind(target)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Tenant not found".to_string()))?;

    let is_active: bool = tenant.try_get("is_active").unwrap_or(false);
    if !is_active {
        return Err(AppError::Forbidden(
            "This tenant is inactive and cannot be impersonated.".to_string(),
        ));
    }

    // The impersonated identity is a REAL, active account of that tenant. Its own role is carried
    // into the token — never the operator's — so the support session has exactly the tenant's
    // reach and nothing more. A tenant that owns nothing still has an account (register and the
    // sale/activate flow each create both), so "no account" is a broken tenant, not a normal
    // state, and it is refused by name.
    let user = sqlx::query(
        "SELECT id, email, name, role FROM users WHERE tenant_id = $1 AND is_active = true \
         ORDER BY (role = 'super_admin') ASC, created_at ASC LIMIT 1",
    )
    .bind(target)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| {
        AppError::BadRequest("This tenant has no active account to sign in as.".to_string())
    })?;

    let user_id: Uuid = user.try_get("id")?;
    let email: String = user.try_get("email").unwrap_or_default();
    let name: String = user.try_get("name").unwrap_or_default();
    let role: String = user.try_get("role").unwrap_or_else(|_| "admin".to_string());

    let now = chrono::Utc::now().timestamp() as usize;
    let imp_claims = Claims {
        sub: user_id.to_string(),
        tid: target.to_string(),
        role: role.clone(),
        exp: now + IMPERSONATION_TTL_SECS as usize,
        iat: now,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: Some(operator.sub.clone()),
    };
    let impersonation_token = create_token(&imp_claims, &s.config.jwt_secret)?;

    // The durable record (kanban t_a9c33fea): one row per mint, keyed by (account, operator, iat).
    // `POST /admin/stop-impersonation` stamps `stopped_at` on this row and the boundary then refuses
    // the token with 401. The insert is NOT best-effort on purpose: a support session we cannot end
    // is exactly the defect this table exists to remove, so a mint that cannot be recorded is
    // refused (500) instead of handing the operator an unrevocable credential.
    if let Some((target_user_id, operator_id, token_iat)) = impersonation_session_key(&imp_claims) {
        sqlx::query(
            "INSERT INTO impersonation_sessions \
                 (operator_id, target_tenant_id, target_user_id, token_iat, expires_at) \
             VALUES ($1, $2, $3, $4, to_timestamp($5)) \
             ON CONFLICT (target_user_id, operator_id, token_iat) DO NOTHING",
        )
        .bind(operator_id)
        .bind(target)
        .bind(target_user_id)
        .bind(token_iat)
        .bind(imp_claims.exp as f64)
        .execute(&s.db)
        .await?;
    }

    // Audit trail: both identities. WARN so it survives an info-level log filter.
    tracing::warn!(
        operator = %operator.sub,
        operator_tenant = %operator.tid,
        target_tenant = %target,
        target_user = %email,
        "operator impersonation session minted"
    );

    Ok(Json(json!({
        "token": impersonation_token,
        "token_type": "Bearer",
        "expires_in": IMPERSONATION_TTL_SECS,
        "impersonating": { "operator_id": operator.sub, "operator_tenant_id": operator.tid },
        "tenant": {
            "id": target.to_string(),
            "name": tenant.try_get::<String, _>("name").unwrap_or_default(),
            "slug": tenant.try_get::<String, _>("slug").unwrap_or_default(),
        },
        "user": { "id": user_id.to_string(), "email": email, "name": name, "role": role },
    })))
}

/// POST /api/v1/admin/stop-impersonation — END the support session that calls this route.
///
/// Called BY the support session itself, not by the operator's own browser: the token that needs
/// ending is the one carrying the TENANT's role, and `operator_guard` admits exactly this one admin
/// path for a claim that carries `impersonating` (see `auth::middleware::operator_surface_denied`).
///
/// Three things happen, in this order:
///
/// 1. the session is ENDED durably — `stopped_at` is stamped on its `impersonation_sessions` row, so
///    every boundary (`session_gate`, then `auth_guard`) refuses the token from the next request on.
///    The upsert also tombstones a token minted before that table existed, so the first stop is
///    always enough;
/// 2. the OPERATOR's own session is re-minted from their live row and returned — the console puts it
///    back in place. This is what lets any device holding only the support token get home; the panel
///    used to depend on a copy of the operator token saved in localStorage on the same device;
/// 3. the event is logged at WARN with both identities (the mint has its own line).
///
/// A live credential that is NOT a support session is refused (403): there is nothing to end, and
/// answering 200 anyway would be the lie this route replaced.
pub async fn stop_impersonation(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let claims = claims_from_headers(&headers, &s.config.jwt_secret)?;
    if claims.impersonating.is_none() {
        return Err(AppError::Forbidden(
            "This is not a support session — there is nothing to stop.".to_string(),
        ));
    }
    // An `impersonating` claim whose pair is not addressable cannot be one of our mints.
    let Some((target_user_id, operator_id, token_iat)) = impersonation_session_key(&claims) else {
        return Err(AppError::Unauthorized);
    };

    // 1. End it, durably, before anything else can fail: the operator asked for the session to be
    //    over, so its death must not depend on the rest of this handler succeeding.
    let stopped_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "INSERT INTO impersonation_sessions \
             (operator_id, target_tenant_id, target_user_id, token_iat, expires_at, stopped_at) \
         VALUES ($1, $2::uuid, $3, $4, to_timestamp($5), NOW()) \
         ON CONFLICT (target_user_id, operator_id, token_iat) \
         DO UPDATE SET stopped_at = COALESCE(impersonation_sessions.stopped_at, NOW()) \
         RETURNING stopped_at",
    )
    .bind(operator_id)
    .bind(&claims.tid)
    .bind(target_user_id)
    .bind(token_iat)
    .bind(claims.exp as f64)
    .fetch_one(&s.db)
    .await?;

    // 2. Hand the operator their own session back. Read the row (not a cached claim): a role
    //    withdrawal or a disabled account between mint and stop must not be re-issued as a token.
    let operator = sqlx::query(
        "SELECT id, email, name, role, tenant_id FROM users WHERE id = $1 AND is_active = true",
    )
    .bind(operator_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let op_role: String = operator.try_get("role").unwrap_or_default();
    let op_tenant: Uuid = operator.try_get("tenant_id")?;
    let now = chrono::Utc::now().timestamp() as usize;
    let ttl = s.config.jwt_access_expiry.max(60) as usize;
    let op_claims = Claims {
        sub: operator_id.to_string(),
        tid: op_tenant.to_string(),
        role: op_role.clone(),
        exp: now + ttl,
        iat: now,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };
    if !is_platform_operator(&op_claims) {
        // The account still signs in, but it is not an operator any more: no operator session to
        // restore. (The tenant session is over either way — step 1 already ended it.)
        return Err(AppError::Forbidden(
            "Your operator access has been withdrawn — sign in again.".to_string(),
        ));
    }
    let operator_token = create_token(&op_claims, &s.config.jwt_secret)?;

    let operator_email: String = operator.try_get("email").unwrap_or_default();
    let operator_name: String = operator.try_get("name").unwrap_or_default();

    // 3. Audit. Both identities, one line — the pair that matters when reading the log later.
    tracing::warn!(
        operator = %operator_id,
        operator_tenant = %op_tenant,
        target_tenant = %claims.tid,
        target_user = %claims.sub,
        token_iat,
        "operator support session ENDED"
    );

    Ok(Json(json!({
        "status": "impersonation_stopped",
        "token": operator_token,
        "token_type": "Bearer",
        "expires_in": ttl,
        "operator": {
            "id": operator_id.to_string(),
            "email": operator_email,
            "name": operator_name,
            "role": op_role,
            "tenant_id": op_tenant.to_string(),
        },
        "ended": {
            "target_user_id": target_user_id.to_string(),
            "target_tenant_id": claims.tid,
            "token_iat": token_iat,
            "stopped_at": stopped_at.to_rfc3339(),
        },
        "note": "The support token is dead. Your own session is in `token`.",
    })))
}

/// GET /api/v1/admin/tenants — the operator's pick-list: every tenant, with the counts needed to
/// choose the right one (and whether it is the platform's own).
pub async fn list_tenants(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    require_operator(&headers, &s)?;

    let rows = sqlx::query(
        "SELECT t.id, t.name, t.slug, t.is_active, \
                (SELECT COUNT(*) FROM users u WHERE u.tenant_id = t.id) AS users, \
                (SELECT COUNT(*) FROM directories d JOIN users du ON du.id = d.owner_id \
                  WHERE du.tenant_id = t.id) AS directories \
         FROM tenants t ORDER BY t.name",
    )
    .fetch_all(&s.db)
    .await?;

    let system = crate::system_tenant::system_tenant_uuid();
    let tenants: Vec<Value> = rows
        .iter()
        .map(|r| {
            let id = r.try_get::<Uuid, _>("id").unwrap_or_else(|_| Uuid::nil());
            json!({
                "id": id.to_string(),
                "name": r.try_get::<String, _>("name").unwrap_or_default(),
                "slug": r.try_get::<String, _>("slug").unwrap_or_default(),
                "is_active": r.try_get::<bool, _>("is_active").unwrap_or(false),
                "users": r.try_get::<i64, _>("users").unwrap_or(0),
                "directories": r.try_get::<i64, _>("directories").unwrap_or(0),
                "is_platform": id == system,
            })
        })
        .collect();

    Ok(Json(json!({ "tenants": tenants })))
}
