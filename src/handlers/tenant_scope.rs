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

use sqlx::PgPool;
use uuid::Uuid;

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
