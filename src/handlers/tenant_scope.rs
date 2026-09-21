//! Tenant / directory / business scope guards — Round 13 IDOR audit.
//!
//! Multi-Directory's model: the platform operator (role `super_admin`) runs the shared
//! city directories, and every other user is a tenant-scoped `admin` (business owner,
//! supplier, or a future directory-owning customer). Every handler that takes an object
//! id from the caller must confirm the object is reachable by that caller, otherwise a
//! tenant can read or write another tenant's rows by guessing an id (IDOR).
//!
//! Rules implemented here:
//!   * `super_admin`                     -> platform operator, sees everything.
//!   * directory owned by `owner_id`     -> users.tenant_id must equal the caller's `tid`.
//!   * business                          -> claimed by the caller, owned by the caller's
//!                                          user, or inside a directory the caller's tenant owns.
//!
//! Failures return 404 (`NotFound`) rather than 403 so the endpoint does not confirm that
//! an object with that id exists at all.

use axum::http::HeaderMap;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::middleware::verify_token;
use crate::auth::models::Claims;
use crate::error::AppError;

/// Tenant id embedded in the caller's JWT.
pub fn caller_tenant(claims: &Claims) -> Result<Uuid, AppError> {
    Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)
}

/// Caller's user id.
pub fn caller_user(claims: &Claims) -> Result<Uuid, AppError> {
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

/// The platform operator — the account that operates the shared directories.
pub fn is_platform_operator(claims: &Claims) -> bool {
    claims.role == "super_admin"
}

/// Verify the caller's JWT straight from the request headers.
///
/// Handlers must NOT rely on `Extension<Claims>`: a route's position in the router does not
/// guarantee `auth_guard` injected the claims (the public-path bypass returns before injection,
/// and per-router layering means a route added after a `.layer()` call is not covered), and a
/// missing extension panics into an HTTP 500 for *every* caller. Reading and verifying the bearer
/// token inside the handler holds no matter which router group the route ends up in.
pub fn claims_from_headers(headers: &HeaderMap, jwt_secret: &str) -> Result<Claims, AppError> {
    headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)
        .and_then(|t| verify_token(t, jwt_secret).map_err(|_| AppError::Unauthorized))
}

/// Guard for a row that carries an owning `user_id` (api_keys, webhooks): the caller must be the
/// platform operator or a user of the same tenant as the row's owner. 404 on failure.
pub async fn assert_user_row_tenant(
    db: &PgPool,
    claims: &Claims,
    table: &str,
    id: Uuid,
    not_found: &str,
) -> Result<(), AppError> {
    if is_platform_operator(claims) {
        return Ok(());
    }
    let tid = caller_tenant(claims)?;
    // `table` is a compile-time literal at every call site, never caller input.
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM {table} t JOIN users u ON u.id = t.user_id \
         WHERE t.id = $1 AND u.tenant_id = $2)"
    );
    let ok = sqlx::query_scalar::<_, bool>(&sql)
        .bind(id)
        .bind(tid)
        .fetch_one(db)
        .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::NotFound(not_found.to_string()))
    }
}

/// Resolve a directory slug to its id (404 when it does not exist).
pub async fn directory_id_by_slug(db: &PgPool, slug: &str) -> Result<Uuid, AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM directories WHERE slug = $1")
        .bind(slug)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("directory not found".into()))
}

/// May this caller administer (read secrets of / write config for) this directory?
pub async fn can_admin_directory(
    db: &PgPool,
    claims: &Claims,
    directory_id: Uuid,
) -> Result<bool, AppError> {
    if is_platform_operator(claims) {
        return Ok(true);
    }
    let tid = caller_tenant(claims)?;
    let ok = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM directories d \
         JOIN users u ON u.id = d.owner_id \
         WHERE d.id = $1 AND u.tenant_id = $2)",
    )
    .bind(directory_id)
    .bind(tid)
    .fetch_one(db)
    .await?;
    Ok(ok)
}

/// Guard for directory-scoped routes addressed by directory id. 404 on failure.
pub async fn assert_directory_admin(
    db: &PgPool,
    claims: &Claims,
    directory_id: Uuid,
) -> Result<(), AppError> {
    if can_admin_directory(db, claims, directory_id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound("directory not found".into()))
    }
}

/// Guard for directory-scoped routes addressed by slug. Returns the directory id.
pub async fn assert_directory_admin_by_slug(
    db: &PgPool,
    claims: &Claims,
    slug: &str,
) -> Result<Uuid, AppError> {
    let id = directory_id_by_slug(db, slug).await?;
    assert_directory_admin(db, claims, id).await?;
    Ok(id)
}

/// May this caller administer this business (owner, claimant, or directory operator)?
pub async fn can_admin_business(
    db: &PgPool,
    claims: &Claims,
    business_id: Uuid,
) -> Result<bool, AppError> {
    if is_platform_operator(claims) {
        return Ok(true);
    }
    let tid = caller_tenant(claims)?;
    let uid = caller_user(claims)?;
    let ok = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
            SELECT 1 FROM claimed_businesses c \
             WHERE c.business_id = $1 AND c.user_id = $3 AND c.is_active \
            UNION ALL \
            SELECT 1 FROM businesses b JOIN users u ON u.id = b.owner_id \
             WHERE b.id = $1 AND u.tenant_id = $2 \
            UNION ALL \
            SELECT 1 FROM businesses b \
              JOIN directories d ON d.id = b.directory_id \
              JOIN users du ON du.id = d.owner_id \
             WHERE b.id = $1 AND du.tenant_id = $2 \
         )",
    )
    .bind(business_id)
    .bind(tid)
    .bind(uid)
    .fetch_one(db)
    .await?;
    Ok(ok)
}

/// Guard for business-scoped routes. 404 on failure.
pub async fn assert_business_admin(
    db: &PgPool,
    claims: &Claims,
    business_id: Uuid,
) -> Result<(), AppError> {
    if can_admin_business(db, claims, business_id).await? {
        Ok(())
    } else {
        Err(AppError::NotFound("business not found".into()))
    }
}

/// Guard for business-scoped routes the business dashboard calls. A business is the caller's when
/// they administer it (owner, claim by user id, or directory operator) OR when the caller's user
/// email matches the active public claim on it — the claim form records `owner_email` only and
/// never `user_id`, so an email match is the only ownership link a self-serve owner has. 404 on
/// failure (never 403) so a probe cannot confirm the business exists.
pub async fn assert_business_admin_or_claimant(
    db: &PgPool,
    claims: &Claims,
    business_id: Uuid,
) -> Result<(), AppError> {
    if can_admin_business(db, claims, business_id).await? {
        return Ok(());
    }
    let uid = caller_user(claims)?;
    let email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(uid)
        .fetch_optional(db)
        .await?;
    if let Some(email) = email {
        let claimed = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM claimed_businesses \
             WHERE business_id = $1 AND owner_email = $2 AND is_active)",
        )
        .bind(business_id)
        .bind(&email)
        .fetch_one(db)
        .await?;
        if claimed {
            return Ok(());
        }
    }
    Err(AppError::NotFound("business not found".into()))
}

/// Guard for a submission: the submission's directory must be one the caller administers
/// (a submission with no directory belongs to the platform operator alone).
pub async fn assert_submission_admin(
    db: &PgPool,
    claims: &Claims,
    submission_id: Uuid,
) -> Result<(), AppError> {
    if is_platform_operator(claims) {
        return Ok(());
    }
    let tid = caller_tenant(claims)?;
    let ok = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
            SELECT 1 FROM submissions s \
              JOIN directories d ON d.id = s.directory_id \
              JOIN users u ON u.id = d.owner_id \
             WHERE s.id = $1 AND u.tenant_id = $2 \
         )",
    )
    .bind(submission_id)
    .bind(tid)
    .fetch_one(db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::NotFound("Submission not found".into()))
    }
}

/// Guard for a business transfer: the caller must be a party (either tenant side), unless
/// they are the platform operator.
pub async fn assert_transfer_party(
    db: &PgPool,
    claims: &Claims,
    transfer_id: Uuid,
) -> Result<(), AppError> {
    if is_platform_operator(claims) {
        return Ok(());
    }
    let tid = caller_tenant(claims)?;
    let ok = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM business_transfers t \
         WHERE t.id = $1 AND (t.from_tenant_id = $2 OR t.to_tenant_id = $2))",
    )
    .bind(transfer_id)
    .bind(tid)
    .fetch_one(db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::NotFound("Transfer not found".into()))
    }
}

/// Guard for a deal: the deal's business or directory must be one the caller administers.
pub async fn assert_deal_admin(
    db: &PgPool,
    claims: &Claims,
    deal_id: Uuid,
) -> Result<(), AppError> {
    if is_platform_operator(claims) {
        return Ok(());
    }
    let tid = caller_tenant(claims)?;
    let uid = caller_user(claims)?;
    let ok = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS ( \
            SELECT 1 FROM deals dl \
              JOIN directories d ON d.id = dl.directory_id \
              JOIN users du ON du.id = d.owner_id \
             WHERE dl.id = $1 AND du.tenant_id = $2 \
            UNION ALL \
            SELECT 1 FROM deals dl \
              JOIN claimed_businesses c ON c.business_id = dl.business_id \
             WHERE dl.id = $1 AND c.user_id = $3 AND c.is_active \
            UNION ALL \
            SELECT 1 FROM deals dl \
              JOIN businesses b ON b.id = dl.business_id \
              JOIN users bu ON bu.id = b.owner_id \
             WHERE dl.id = $1 AND bu.tenant_id = $2 \
         )",
    )
    .bind(deal_id)
    .bind(tid)
    .bind(uid)
    .fetch_one(db)
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::NotFound("Deal not found".into()))
    }
}
