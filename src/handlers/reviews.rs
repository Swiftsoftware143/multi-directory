//! Review CRUD and moderation handlers.

use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult, validate_pagination};
use crate::models::*;

/// GET /api/v1/reviews — list all reviews (admin)
pub async fn list_reviews(
    State(s): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<impl IntoResponse> {
    let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
    let per_page = params.get("per_page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    let status_filter = params.get("status");

    let total = if let Some(ref st) = status_filter {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM reviews WHERE status = \x241 "
        )
        .bind(st)
        .fetch_one(&s.db)
        .await?
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM reviews "
        )
        .fetch_one(&s.db)
        .await?
    };

    let reviews = if let Some(ref st) = status_filter {
        sqlx::query_as::<_, Review>(
            "SELECT * FROM reviews WHERE status = \x241 ORDER BY created_at DESC LIMIT \x242 OFFSET \x243 "
        )
        .bind(st)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, Review>(
            "SELECT * FROM reviews ORDER BY created_at DESC LIMIT \x241 OFFSET \x242 "
        )
        .bind(per_page)
        .bind(offset)
        .fetch_all(&s.db)
        .await?
    };

    let total_pages = (total as f64 / per_page as f64).ceil() as i64;

    Ok(Json(json!(PaginatedResponse {
        data: reviews,
        page,
        per_page,
        total,
        total_pages,
    })))
}

/// POST /api/v1/reviews — create a new review (public submission, no auth)
pub async fn create_review(
    State(s): State<AppState>,
    Json(req): Json<CreateReviewRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.rating < 1 || req.rating > 5 {
        return Err(AppError::Validation("Rating must be between 1 and 5".to_string()));
    }

    // Verify business exists
    let biz_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM businesses WHERE id = \x241 "
    )
    .bind(req.business_id)
    .fetch_one(&s.db)
    .await?;

    if biz_exists == 0 {
        return Err(AppError::NotFound("Business not found".to_string()));
    }

    let review = sqlx::query_as::<_, Review>(
        r#"INSERT INTO reviews (business_id, rating, title, content, reviewer_name, reviewer_email, source, source_url, directory_id, status)
           VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, 'pending')
           RETURNING *"#
    )
    .bind(req.business_id)
    .bind(req.rating)
    .bind(&req.title)
    .bind(&req.content)
    .bind(&req.reviewer_name)
    .bind(&req.reviewer_email)
    .bind(&req.source)
    .bind(&req.source_url)
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    Ok((StatusCode::CREATED, Json(json!(review))))
}

/// GET /api/v1/reviews/:id — get single review
pub async fn get_review(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = sqlx::query_as::<_, Review>(
        "SELECT * FROM reviews WHERE id = \x241 "
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Review not found".to_string()))?;

    Ok(Json(json!(review)))
}

/// PUT /api/v1/reviews/:id — update review (admin moderation)
pub async fn update_review(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReviewRequest>,
) -> ApiResult<impl IntoResponse> {
    let review = sqlx::query_as::<_, Review>(
        r#"UPDATE reviews SET
           rating = COALESCE(\x241, rating),
           title = COALESCE(\x242, title),
           content = COALESCE(\x243, content),
           reviewer_name = COALESCE(\x244, reviewer_name),
           reviewer_email = COALESCE(\x245, reviewer_email),
           featured = COALESCE(\x246, featured),
           source = COALESCE(\x247, source),
           source_url = COALESCE(\x248, source_url),
           updated_at = NOW()
           WHERE id = \x249 RETURNING *"#
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

/// DELETE /api/v1/reviews/:id — delete a review
pub async fn delete_review(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let result = sqlx::query("DELETE FROM reviews WHERE id = \x241")
        .bind(id)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Review not found".to_string()));
    }

    Ok(Json(json!({"message": "Review deleted successfully"})))
}

/// POST /api/v1/reviews/:id/approve — approve a review
pub async fn approve_review(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = sqlx::query_as::<_, Review>(
        r#"UPDATE reviews SET status = 'approved', is_verified = true, updated_at = NOW()
           WHERE id = \x241 RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Review not found".to_string()))?;

    // Update business rating aggregates
    if let Some(biz_id) = review.business_id {
        sqlx::query(
            r#"UPDATE businesses SET
               rating = (SELECT ROUND(AVG(rating)::numeric, 1) FROM reviews WHERE business_id = \x241 AND status = 'approved'),
               review_count = (SELECT COUNT(*) FROM reviews WHERE business_id = \x241 AND status = 'approved'),
               updated_at = NOW()
               WHERE id = \x241"#
        )
        .bind(biz_id)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!(review)))
}

/// POST /api/v1/reviews/:id/reject — reject a review
pub async fn reject_review(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let review = sqlx::query_as::<_, Review>(
        r#"UPDATE reviews SET status = 'rejected', updated_at = NOW()
           WHERE id = \x241 RETURNING *"#
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Review not found".to_string()))?;

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
           FROM reviews WHERE business_id = \x241 AND status = 'approved'"#
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
    let _dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
    let per_page = params.get("per_page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(50);
    let (page, per_page) = validate_pagination(Some(page), Some(per_page));
    let offset = (page - 1) * per_page;

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reviews WHERE business_id = \x241 AND status = 'approved'"
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
