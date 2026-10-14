//! Review CRUD and moderation handlers.

use axum::{
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{validate_pagination, ApiResult, AppError};
use crate::models::*;
use crate::AppState;

/// GET /api/v1/reviews — list all reviews (admin)
pub async fn list_reviews(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let page = params
        .get("page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    // Moderation gate. This route lives in the public allowlist — business-detail.html calls it
    // anonymously with `?status=approved` — so an anonymous caller must only ever see reviews a
    // business owner (or an admin) has APPROVED. Before this, a request with `business_id` and no
    // `status` returned EVERY status: a pending or rejected review's text answered 200 to anyone
    // on the internet (measured 2026-10-13 on https://zaarhub.com/api/v1/reviews?business_id=…,
    // which returned a `pending` row anonymously). A signed-in caller (owner portal, admin console)
    // sends a valid session token and may filter to any status; anonymous callers are pinned to
    // `approved`.
    let requested_status: Option<String> = params.get("status").map(|s| s.to_string());
    let privileged = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(h) => {
            let token = h.strip_prefix("Bearer ").unwrap_or(h);
            crate::auth::middleware::verify_token(token, &s.config.jwt_secret).is_ok()
        }
        None => false,
    };
    let status_filter: Option<String> = if privileged {
        requested_status
    } else {
        Some("approved".to_string())
    };
    // Optional business scoping. This route is public for GET and the business-detail
    // page needs the reviews for ONE business; without a business filter it returned the
    // newest reviews on the whole platform (i.e. somebody else's reviews on every page).
    let business_id = params
        .get("business_id")
        .and_then(|v| Uuid::parse_str(v).ok());
    // Optional directory scoping (B91 — owner portal dashboard + Reviews moderation ask for a
    // single directory's reviews). `reviews.directory_id` is a real column + index, so no join
    // is required.
    let directory_id = params
        .get("directory_id")
        .and_then(|v| Uuid::parse_str(v).ok());

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reviews WHERE (\x241::text IS NULL OR status = \x241) AND (\x242::uuid IS NULL OR business_id = \x242) AND (\x243::uuid IS NULL OR directory_id = \x243 OR business_id IN (SELECT id FROM businesses WHERE directory_id = \x243)) ",
    )
    .bind(status_filter.as_deref())
    .bind(business_id)
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    let reviews = sqlx::query_as::<_, Review>(
        "SELECT * FROM reviews WHERE (\x241::text IS NULL OR status = \x241) AND (\x242::uuid IS NULL OR business_id = \x242) AND (\x243::uuid IS NULL OR directory_id = \x243 OR business_id IN (SELECT id FROM businesses WHERE directory_id = \x243)) ORDER BY created_at DESC LIMIT \x244 OFFSET \x245 ",
    )
    .bind(status_filter.as_deref())
    .bind(business_id)
    .bind(directory_id)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!(PaginatedResponse {
        data: reviews,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// POST /api/v1/reviews — create a review (authenticated; integrity-guarded, card B90)
pub async fn create_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateReviewRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.rating < 1 || req.rating > 5 {
        return Err(AppError::Validation(
            "Rating must be between 1 and 5".to_string(),
        ));
    }

    // Verify business exists
    let biz_exists =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM businesses WHERE id = \x241 ")
            .bind(req.business_id)
            .fetch_one(&s.db)
            .await?;

    if biz_exists == 0 {
        return Err(AppError::NotFound("Business not found".to_string()));
    }

    // ── Review integrity (card B90) ───────────────────────────────────────────
    // Every review write is authenticated (POST /reviews is not in the public allowlist),
    // so we can attribute the review to the caller and refuse the two classic abuses:
    // (1) a business owner reviewing their own listing, (2) the same account reviewing one
    // listing twice. A review is marked "verified customer" ONLY when the caller has a real
    // transaction at this business (a loyalty scan), which is the Angie's-List-style trust
    // signal competitors monetise. Anonymous/legacy rows keep is_verified = false.
    let caller: Option<Uuid> = Uuid::parse_str(&claims.sub).ok();

    if let Some(uid) = caller {
        let owns: i64 = sqlx::query_scalar(
            r#"SELECT (SELECT COUNT(*) FROM businesses WHERE id = $1 AND owner_id = $2)
                    + (SELECT COUNT(*) FROM claimed_businesses
                        WHERE business_id = $1 AND is_active
                          AND (visitor_account_id = $2 OR user_id = $2))"#,
        )
        .bind(req.business_id)
        .bind(uid)
        .fetch_one(&s.db)
        .await?;

        if owns > 0 {
            return Err(AppError::Forbidden(
                "You cannot review a business you own.".to_string(),
            ));
        }

        let already: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM reviews WHERE business_id = $1 AND user_id = $2",
        )
        .bind(req.business_id)
        .bind(uid)
        .fetch_one(&s.db)
        .await?;

        if already > 0 {
            return Err(AppError::Conflict(
                "You have already reviewed this business.".to_string(),
            ));
        }
    }

    // Attribute the review to the account when we can, and default the display identity
    // from it so a review is never anonymous when we know who wrote it.
    let account: Option<(Option<String>, String)> = match caller {
        Some(uid) => {
            sqlx::query_as("SELECT name, email FROM visitor_accounts WHERE id = $1")
                .bind(uid)
                .fetch_optional(&s.db)
                .await?
        }
        None => None,
    };

    // "Verified customer": the caller has an actual loyalty scan at this business.
    let is_verified: bool = match caller {
        Some(uid) => {
            sqlx::query_scalar::<_, i64>(
                r#"SELECT COUNT(*) FROM loyalty_scans ls
                   JOIN loyalty_members lm ON lm.id = ls.member_id
                   WHERE lm.visitor_account_id = $1 AND ls.business_id = $2"#,
            )
            .bind(uid)
            .bind(req.business_id)
            .fetch_one(&s.db)
            .await?
                > 0
        }
        None => false,
    };

    let reviewer_name = req
        .reviewer_name
        .clone()
        .or_else(|| account.as_ref().and_then(|a| a.0.clone()));
    let reviewer_email = account
        .as_ref()
        .map(|a| a.1.clone())
        .or_else(|| req.reviewer_email.clone());
    let source = req.source.clone().or_else(|| {
        if is_verified {
            Some("verified_customer".to_string())
        } else {
            None
        }
    });

    let review = sqlx::query_as::<_, Review>(
        r#"INSERT INTO reviews (business_id, user_id, rating, title, content, reviewer_name, reviewer_email, source, source_url, directory_id, is_verified, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'pending')
           RETURNING *"#
    )
    .bind(req.business_id)
    .bind(caller)
    .bind(req.rating)
    .bind(&req.title)
    .bind(&req.content)
    .bind(&reviewer_name)
    .bind(&reviewer_email)
    .bind(&source)
    .bind(&req.source_url)
    .bind(req.directory_id)
    .bind(Some(is_verified))
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(review))))
}

/// GET /api/v1/reviews/:id — get single review
pub async fn get_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = sqlx::query_as::<_, Review>("SELECT * FROM reviews WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound("Review not found".to_string()))?;

    // An unapproved review is private. The list route already hides non-approved rows from
    // everyone but a moderator; this single-read route must not undo that. A pending or rejected
    // review (and the reviewer's email carried on it) is readable only by its author or by a
    // moderator of its business — anyone else gets the same "not found" as a review that does not
    // exist, so the route cannot be used to confirm that a hidden review is there.
    if review.status.as_deref() != Some("approved") {
        let caller = Uuid::parse_str(&claims.sub).ok();
        let is_author = caller.is_some() && review.user_id == caller;
        if !is_author && !can_moderate(&s.db, &claims, review.business_id).await? {
            return Err(AppError::NotFound("Review not found".to_string()));
        }
    }

    Ok(Json(json!(review)))
}

/// PUT /api/v1/reviews/:id — update review (owner of the business, owner of its directory, or the
/// platform `super_admin`).
pub async fn update_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReviewRequest>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Review>("SELECT * FROM reviews WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound("Review not found".to_string()))?;
    if !can_moderate(&s.db, &claims, existing.business_id).await? {
        return Err(AppError::Forbidden(
            "You can only moderate reviews of a business you own.".to_string(),
        ));
    }

    let review = sqlx::query_as::<_, Review>(
        r#"UPDATE reviews SET
           rating = COALESCE($1, rating),
           title = COALESCE($2, title),
           content = COALESCE($3, content),
           reviewer_name = COALESCE($4, reviewer_name),
           reviewer_email = COALESCE($5, reviewer_email),
           featured = COALESCE($6, featured),
           source = COALESCE($7, source),
           source_url = COALESCE($8, source_url),
           updated_at = NOW()
           WHERE id = $9 RETURNING *"#,
    )
    .bind(req.rating)
    .bind(&req.title)
    .bind(&req.content)
    .bind(&req.reviewer_name)
    .bind(&req.reviewer_email)
    .bind(req.featured)
    .bind(&req.source)
    .bind(&req.source_url)
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Review not found".to_string()))?;

    Ok(Json(json!(review)))
}

/// DELETE /api/v1/reviews/:id — delete a review (owner of the business, owner of its directory, or
/// the platform `super_admin`).
pub async fn delete_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let existing = sqlx::query_as::<_, Review>("SELECT * FROM reviews WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound("Review not found".to_string()))?;
    // An account may always remove a review it wrote; a moderator of the business may remove any
    // review on it. (The customer's own "My Reviews" page offers Delete on their own rows — before
    // this, the author's own delete was refused as if it were somebody else's review.)
    let caller = Uuid::parse_str(&claims.sub).ok();
    let is_author = caller.is_some() && existing.user_id == caller;
    if !is_author && !can_moderate(&s.db, &claims, existing.business_id).await? {
        return Err(AppError::Forbidden(
            "You can only delete a review you wrote, or moderate reviews of a business you own."
                .to_string(),
        ));
    }

    let business_id = existing.business_id;

    let result = sqlx::query("DELETE FROM reviews WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Review not found".to_string()));
    }

    // Keep the listing's rating/count honest: removing a review changes the average. Approve/reject
    // already recompute this; delete used to leave a business showing a rating backed by a review
    // that no longer exists.
    if let Some(bid) = business_id {
        sqlx::query(
            r#"UPDATE businesses SET
               rating = (SELECT ROUND(AVG(rating)::numeric, 1) FROM reviews WHERE business_id = $1 AND status = 'approved'),
               review_count = (SELECT COUNT(*) FROM reviews WHERE business_id = $1 AND status = 'approved'),
               updated_at = NOW()
               WHERE id = $1"#,
        )
        .bind(bid)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({"message": "Review deleted successfully"})))
}

/// POST /api/v1/reviews/:id/approve — approve a review. Authorized for the business's owner (or an
/// active claimant), the owner of its directory, or the platform `super_admin`; recomputes the
/// business rating aggregates. Approving is moderation, not customer verification: the "Verified
/// customer" badge is earned by a real transaction and set at review creation, so this leaves
/// `is_verified` untouched.
pub async fn approve_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = set_review_status_owned(&s, &claims, id, "approved").await?;
    Ok(Json(json!(review)))
}

/// POST /api/v1/reviews/:id/reject — reject a review (same authorization as approve).
pub async fn reject_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = set_review_status_owned(&s, &claims, id, "rejected").await?;
    Ok(Json(json!(review)))
}

/// GET /api/v1/reviews/stats/:business_id - review statistics for a business
pub async fn get_review_stats(
    State(s): State<AppState>,
    Path(business_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    // Use raw query to avoid FromRow issues
    let row = sqlx::query(
        r#"SELECT
           ROUND(AVG(rating)::numeric, 1)::float8 as avg_rating,
           COUNT(*) as total,
           COUNT(*) FILTER (WHERE rating = 1) as r1,
           COUNT(*) FILTER (WHERE rating = 2) as r2,
           COUNT(*) FILTER (WHERE rating = 3) as r3,
           COUNT(*) FILTER (WHERE rating = 4) as r4,
           COUNT(*) FILTER (WHERE rating = 5) as r5
           FROM reviews WHERE business_id = $1 AND status = 'approved'"#,
    )
    .bind(business_id)
    .fetch_optional(&s.db)
    .await?;

    use sqlx::Row;

    if let Some(row) = row {
        let avg: Option<f64> = row.try_get("avg_rating").ok();
        let total: i64 = row.try_get("total").unwrap_or(0);
        let r1: i64 = row.try_get("r1").unwrap_or(0);
        let r2: i64 = row.try_get("r2").unwrap_or(0);
        let r3: i64 = row.try_get("r3").unwrap_or(0);
        let r4: i64 = row.try_get("r4").unwrap_or(0);
        let r5: i64 = row.try_get("r5").unwrap_or(0);

        Ok(Json(json!(ReviewStats {
            business_id,
            average_rating: avg,
            total_reviews: total,
            rating_1: r1,
            rating_2: r2,
            rating_3: r3,
            rating_4: r4,
            rating_5: r5,
        })))
    } else {
        Ok(Json(json!(ReviewStats {
            business_id,
            average_rating: None,
            total_reviews: 0,
            rating_1: 0,
            rating_2: 0,
            rating_3: 0,
            rating_4: 0,
            rating_5: 0,
        })))
    }
}

pub async fn list_business_reviews(
    State(s): State<AppState>,
    Path((slug, business_id)): Path<(String, Uuid)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    // Verify directory exists
    let _dir = sqlx::query_as::<_, Directory>("SELECT * FROM directories WHERE slug = \x241 ")
        .bind(&slug)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound(format!(
            "Directory '{}' not found",
            slug
        )))?;

    let page = params
        .get("page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reviews WHERE business_id = \x241 AND status = 'approved'",
    )
    .bind(business_id)
    .fetch_one(&s.db)
    .await?;

    let reviews = sqlx::query_as::<_, Review>(
        "SELECT * FROM reviews WHERE business_id = \x241 AND status = 'approved' ORDER BY created_at DESC LIMIT \x242 OFFSET \x243 "
    )
    .bind(business_id)
    .bind(per_page)
    .bind(offset)
    .fetch_all(&s.db)
    .await?;

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!(PaginatedResponse {
        data: reviews,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// GET /api/v1/my-reviews — reviews written by the signed-in account (visitor or admin user).
/// Round 9: user-saved.html used to call /reviews/mine, which never existed.
pub async fn my_reviews(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let email: Option<String> = sqlx::query_scalar(
        r#"SELECT email FROM visitor_accounts WHERE id::text = $1
           UNION ALL
           SELECT email FROM users WHERE id::text = $1
           LIMIT 1"#,
    )
    .bind(&claims.sub)
    .fetch_optional(&s.db)
    .await?;

    let rows = sqlx::query_as::<_, Review>(
        r#"SELECT * FROM reviews
           WHERE user_id::text = $1
              OR ($2::text <> '' AND lower(coalesce(reviewer_email, '')) = lower($2))
           ORDER BY created_at DESC
           LIMIT 200"#,
    )
    .bind(&claims.sub)
    .bind(email.unwrap_or_default())
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!(rows)))
}

/// POST /api/v1/reviews/:id/respond — the business owner's PUBLIC reply to a review (card B90).
///
/// Thumbtack / Angie's-List parity: owners answer reviews publicly, which is the trust signal
/// those directories monetise. Owner-scoped — the caller must own the review's business (via
/// `businesses.owner_id` or an active claim), the same ownership test `create_review` uses.
/// Admins may reply on a business's behalf for moderation. An empty reply clears it.
pub async fn respond_to_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<RespondToReviewRequest>,
) -> ApiResult<impl IntoResponse> {
    let review = sqlx::query_as::<_, Review>("SELECT * FROM reviews WHERE id = \x241 ")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound("Review not found".to_string()))?;

    let business_id = review
        .business_id
        .ok_or(AppError::Validation("Review has no business".to_string()))?;

    let uid = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Forbidden("Sign in to reply to a review".to_string()))?;

    // Ownership: outright owner, active claimant, the owning directory's owner, or super_admin.
    // NOT `role == "admin"`: `/auth/register` mints that role for EVERY new signup, so trusting it
    // let any account post a public "owner reply" on a stranger's review (measured 2026-10-14 —
    // a fresh signup answered an unrelated business's review with HTTP 200 and the text was
    // written to the row). Same gate the approve/reject path uses.
    if !can_moderate(&s.db, &claims, Some(business_id)).await? {
        return Err(AppError::Forbidden(
            "You can only reply to reviews of a business you own.".to_string(),
        ));
    }

    let text = req.response.trim();
    if text.chars().count() > 4000 {
        return Err(AppError::Validation(
            "Reply is too long (max 4000 characters)".to_string(),
        ));
    }
    let (response, responded_at, responded_by): (
        Option<String>,
        Option<DateTime<Utc>>,
        Option<Uuid>,
    ) = if text.is_empty() {
        (None, None, None)
    } else {
        (Some(text.to_string()), Some(Utc::now()), Some(uid))
    };

    let updated = sqlx::query_as::<_, Review>(
        r#"UPDATE reviews SET
             owner_response = $1,
             owner_responded_at = $2,
             owner_response_by = $3,
             updated_at = NOW()
           WHERE id = $4 RETURNING *"#,
    )
    .bind(&response)
    .bind(responded_at)
    .bind(responded_by)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(updated)))
}

// ── Business-owner review surface (business-portal.html) ─────────────────────────────────────
//
// `docs/GAMIFICATION_ENGINE.md` advertises `GET /business/reviews/pending` and
// `POST /business/reviews/:id/respond`, and `docs/business-owner-guide.md` tells a listing owner
// to "Approve or Reject" reviews of their own business — but the router had no `/business/reviews/*`
// prefix, so every one of those documented business-side calls 404'd (measured 2026-10-10:
// /business/reviews/pending -> 404, /business/reviews/:id/respond -> 404). These handlers close
// that gap. All four are owner-scoped: the caller must own the review's business outright
// (`businesses.owner_id`) or hold an active claim on it (`claimed_businesses`), the same ownership
// test `create_review` and `respond_to_review` already use.

/// Does `uid` own `business_id`? 1+ if they own it outright or hold an active claim, else 0.
async fn owned_by(db: &sqlx::PgPool, business_id: Uuid, uid: Uuid) -> Result<i64, AppError> {
    let owns: i64 = sqlx::query_scalar(
        r#"SELECT (SELECT COUNT(*) FROM businesses WHERE id = $1 AND owner_id = $2)
                + (SELECT COUNT(*) FROM claimed_businesses
                    WHERE business_id = $1 AND is_active
                      AND (visitor_account_id = $2 OR user_id = $2))"#,
    )
    .bind(business_id)
    .bind(uid)
    .fetch_one(db)
    .await?;
    Ok(owns)
}

/// True when the caller may moderate a review on `business_id`: the platform `super_admin`, the
/// business's owner (or an active claimant), or the owner of the directory the business belongs to.
/// The app's flat role model grants every new account the tenant role `admin`, so `role == "admin"`
/// is NOT an operator signal and is never consulted here.
pub(crate) async fn can_moderate(
    db: &sqlx::PgPool,
    claims: &Claims,
    business_id: Option<Uuid>,
) -> Result<bool, AppError> {
    if claims.role == "super_admin" {
        return Ok(true);
    }
    let uid = match Uuid::parse_str(&claims.sub) {
        Ok(u) => u,
        Err(_) => return Ok(false),
    };
    let Some(bid) = business_id else {
        return Ok(false);
    };
    if owned_by(db, bid, uid).await? > 0 {
        return Ok(true);
    }
    let dir_owns: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM businesses b
           JOIN directories d ON d.id = b.directory_id
           WHERE b.id = $1 AND d.owner_id = $2"#,
    )
    .bind(bid)
    .bind(uid)
    .fetch_one(db)
    .await?;
    Ok(dir_owns > 0)
}

/// GET /api/v1/business/reviews/pending — reviews on the caller's own businesses that the owner
/// has not replied to yet. This is the documented "reviews needing response" list.
pub async fn business_pending_reviews(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let uid = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Forbidden("Sign in to see pending reviews".to_string()))?;

    let rows = sqlx::query_as::<_, Review>(
        r#"SELECT r.* FROM reviews r
           WHERE r.owner_response IS NULL
             AND r.status <> 'rejected'
             AND (
                 EXISTS (SELECT 1 FROM businesses b
                         WHERE b.id = r.business_id AND b.owner_id = $1)
                 OR EXISTS (SELECT 1 FROM claimed_businesses c
                            WHERE c.business_id = r.business_id AND c.is_active
                              AND (c.visitor_account_id = $1 OR c.user_id = $1))
             )
           ORDER BY r.created_at DESC
           LIMIT 200"#,
    )
    .bind(uid)
    .fetch_all(&s.db)
    .await?;

    Ok(Json(json!({ "data": rows })))
}

/// Approve/reject a review on a business the caller owns. Admins moderate on the owner's behalf.
/// Recomputes the business rating aggregates so the public listing reflects the change at once.
async fn set_review_status_owned(
    s: &AppState,
    claims: &Claims,
    id: Uuid,
    status: &str,
) -> Result<Review, AppError> {
    let review = sqlx::query_as::<_, Review>("SELECT * FROM reviews WHERE id = $1")
        .bind(id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::NotFound("Review not found".to_string()))?;

    let business_id = review
        .business_id
        .ok_or(AppError::Validation("Review has no business".to_string()))?;

    if !can_moderate(&s.db, claims, review.business_id).await? {
        return Err(AppError::Forbidden(
            "You can only moderate reviews of a business you own.".to_string(),
        ));
    }

    // Moderation is orthogonal to the "verified customer" trust signal. `is_verified` means the
    // reviewer had a real transaction at this business (a loyalty scan) and is computed once at
    // review creation (create_review); approving a review must NOT mint that badge for an
    // ordinary reviewer, so the status update leaves `is_verified` untouched.
    let updated = sqlx::query_as::<_, Review>(
        r#"UPDATE reviews SET
             status = $1,
             updated_at = NOW()
           WHERE id = $2 RETURNING *"#,
    )
    .bind(status)
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    sqlx::query(
        r#"UPDATE businesses SET
           rating = (SELECT ROUND(AVG(rating)::numeric, 1) FROM reviews WHERE business_id = $1 AND status = 'approved'),
           review_count = (SELECT COUNT(*) FROM reviews WHERE business_id = $1 AND status = 'approved'),
           updated_at = NOW()
           WHERE id = $1"#,
    )
    .bind(business_id)
    .execute(&s.db)
    .await?;

    Ok(updated)
}

/// POST /api/v1/business/reviews/:id/approve — a listing owner approves a review of their own business.
pub async fn business_approve_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = set_review_status_owned(&s, &claims, id, "approved").await?;
    Ok(Json(json!(review)))
}

/// POST /api/v1/business/reviews/:id/reject — a listing owner rejects a review of their own business.
pub async fn business_reject_review(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = set_review_status_owned(&s, &claims, id, "rejected").await?;
    Ok(Json(json!(review)))
}
