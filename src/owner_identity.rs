//! Which account is this caller?
//!
//! `POST /auth/login` looks the email up in `users` FIRST and only falls back to
//! `visitor_accounts`, so the same person can end up authenticated as an account in either
//! table. Ownership records written by the public claim route (`claimed_businesses`) carry the
//! claimant's EMAIL and no account id at all until a transfer links one, so the owner's email is
//! the only thing that can be matched against them.
//!
//! Reading that email from `visitor_accounts` alone silently denied ownership to every account
//! that lives in `users` — i.e. everyone who signed up through the business portal itself
//! (`/auth/register` writes `users`). Measured 2026-11-04: a claimed business never appeared in
//! the owner's portal, the review queue came back empty, and approving a review on their own
//! listing answered 403 "You can only moderate reviews of a business you own."

use sqlx::PgPool;
use uuid::Uuid;

/// The email of the account `uid` in whichever identity table holds it.
pub async fn caller_email(db: &PgPool, uid: Uuid) -> Result<Option<String>, sqlx::Error> {
    const FROM_USERS: &str = "SELECT email FROM users WHERE id = $1";
    const FROM_VISITORS: &str = "SELECT email FROM visitor_accounts WHERE id = $1";

    let staff: Option<String> = sqlx::query_scalar(FROM_USERS)
        .bind(uid)
        .fetch_optional(db)
        .await?;
    if staff.is_some() {
        return Ok(staff);
    }
    sqlx::query_scalar(FROM_VISITORS)
        .bind(uid)
        .fetch_optional(db)
        .await
}
