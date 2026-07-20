//! Business Dashboard — content mention metrics for business owners.
//! BL13 extension: per-business counts from blog posts, business articles,
//! and trap door (programmatic) pages.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::auth::models::Claims;
use crate::error::{AppError, ApiResult};
use crate::handlers::portal::{BusinessProfile, BusinessSubscriptionInfo};

// ── Response Types ──

#[derive(Debug, Serialize)]
pub struct MentionCounts {
    pub this_week: i64,
    pub this_month: i64,
    pub all_time: i64,
}

#[derive(Debug, Serialize)]
pub struct BusinessMetrics {
    pub blog_post_mentions: MentionCounts,
    pub business_article_mentions: MentionCounts,
    pub trap_door_mentions: MentionCounts,
    pub total_mentions: MentionCounts,
}

#[derive(Debug, Serialize)]
pub struct ClaimedBusinessDashboard {
    pub business: BusinessProfile,
    pub metrics: BusinessMetrics,
    pub subscription: Option<BusinessSubscriptionInfo>,
}

#[derive(Debug, Serialize)]
pub struct BusinessDashboardResponse {
    pub claimed_businesses: Vec<ClaimedBusinessDashboard>,
}

// ── Handler ──

/// GET /api/v1/portal/business/dashboard
///
/// Returns each claimed business for the authenticated user along with
/// content mention metrics (blog posts, business articles, trap door pages).
pub async fn business_dashboard(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // ── Find claimed businesses for this user ──
    let claims_rows = sqlx::query_as::<_, ClaimedBusinessRow>(
        r#"SELECT id, business_id, owner_email, owner_name, owner_phone, user_id, is_active, created_at
           FROM claimed_businesses
           WHERE user_id = $1
           ORDER BY created_at DESC"#
    )
    .bind(user_id)
    .fetch_all(&s.db)
    .await?;

    let mut dashboard_items = Vec::new();

    for claim in &claims_rows {
        // ── Fetch business details (with category name from join) ──
        let biz = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<serde_json::Value>)>(
            r#"SELECT b.id, b.name, dc.name as category, b.city, b.state, b.phone, b.website, b.images
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.id = $1"#
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        let business_profile = match biz {
            Some((id, name, category, city, state, phone, website, images)) => BusinessProfile {
                id, name, category, city, state, phone, website, images,
            },
            None => continue,
        };

        let business_id = business_profile.id;

        // ── Query mention counts ──
        let blog_all = count_blog_mentions(&s.db, business_id, None).await?;
        let blog_week = count_blog_mentions(&s.db, business_id, Some("7 days")).await?;
        let blog_month = count_blog_mentions(&s.db, business_id, Some("30 days")).await?;

        let article_all = count_article_mentions(&s.db, business_id, None).await?;
        let article_week = count_article_mentions(&s.db, business_id, Some("7 days")).await?;
        let article_month = count_article_mentions(&s.db, business_id, Some("30 days")).await?;

        let trapdoor_all = count_trapdoor_mentions(&s.db, business_id, None).await?;
        let trapdoor_week = count_trapdoor_mentions(&s.db, business_id, Some("7 days")).await?;
        let trapdoor_month = count_trapdoor_mentions(&s.db, business_id, Some("30 days")).await?;

        let metrics = BusinessMetrics {
            blog_post_mentions: MentionCounts {
                this_week: blog_week,
                this_month: blog_month,
                all_time: blog_all,
            },
            business_article_mentions: MentionCounts {
                this_week: article_week,
                this_month: article_month,
                all_time: article_all,
            },
            trap_door_mentions: MentionCounts {
                this_week: trapdoor_week,
                this_month: trapdoor_month,
                all_time: trapdoor_all,
            },
            total_mentions: MentionCounts {
                this_week: blog_week + article_week + trapdoor_week,
                this_month: blog_month + article_month + trapdoor_month,
                all_time: blog_all + article_all + trapdoor_all,
            },
        };

        // ── Fetch subscription info ──
        let sub = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<String>, Option<String>, Option<String>, Option<rust_decimal::Decimal>, Option<chrono::NaiveDate>, Option<chrono::NaiveDate>, Option<bool>)>(
            r#"SELECT bs.id, bs.tier_id, pt.name, bs.status, bs.billing_cycle, bs.price_paid, bs.start_date, bs.end_date, bs.auto_renew
               FROM business_subscriptions bs
               LEFT JOIN plan_tiers pt ON pt.id = bs.tier_id
               WHERE bs.business_id = $1
               ORDER BY bs.created_at DESC
               LIMIT 1"#
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        let subscription = sub.map(|(id, tier_id, tier_name, status, billing_cycle, price_paid, start_date, end_date, auto_renew)| {
            BusinessSubscriptionInfo {
                id, tier_id, tier_name,
                status, billing_cycle, price_paid, start_date, end_date, auto_renew,
            }
        });

        dashboard_items.push(ClaimedBusinessDashboard {
            business: business_profile,
            metrics,
            subscription,
        });
    }

    Ok(Json(json!(BusinessDashboardResponse {
        claimed_businesses: dashboard_items,
    })))
}

// ── Helper query functions ──

async fn count_blog_mentions(db: &sqlx::PgPool, business_id: Uuid, interval: Option<&str>) -> ApiResult<i64> {
    let (sql, bind_interval) = match interval {
        Some(days) => (
            format!(
                "SELECT COUNT(*) FROM blog_posts \
                 WHERE $1 = ANY(mentioned_business_ids) AND status = 'published' \
                 AND created_at >= NOW() - INTERVAL '{}'",
                days
            ),
            true,
        ),
        None => (
            "SELECT COUNT(*) FROM blog_posts \
             WHERE $1 = ANY(mentioned_business_ids) AND status = 'published'"
                .to_string(),
            false,
        ),
    };

    let count: (i64,) = if bind_interval {
        let (c,) = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(business_id)
            .fetch_one(db)
            .await?;
        (c,)
    } else {
        let (c,) = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(business_id)
            .fetch_one(db)
            .await?;
        (c,)
    };

    Ok(count.0)
}

async fn count_article_mentions(db: &sqlx::PgPool, business_id: Uuid, interval: Option<&str>) -> ApiResult<i64> {
    let (sql, bind_interval) = match interval {
        Some(days) => (
            format!(
                "SELECT COUNT(*) FROM business_articles \
                 WHERE business_id = $1 AND status = 'published' \
                 AND created_at >= NOW() - INTERVAL '{}'",
                days
            ),
            true,
        ),
        None => (
            "SELECT COUNT(*) FROM business_articles \
             WHERE business_id = $1 AND status = 'published'"
                .to_string(),
            false,
        ),
    };

    let count: (i64,) = if bind_interval {
        let (c,) = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(business_id)
            .fetch_one(db)
            .await?;
        (c,)
    } else {
        let (c,) = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(business_id)
            .fetch_one(db)
            .await?;
        (c,)
    };

    Ok(count.0)
}

async fn count_trapdoor_mentions(db: &sqlx::PgPool, business_id: Uuid, interval: Option<&str>) -> ApiResult<i64> {
    let (sql, bind_interval) = match interval {
        Some(days) => (
            format!(
                "SELECT COUNT(*) FROM programmatic_pages \
                 WHERE $1 = ANY(mentioned_business_ids) AND status = 'published' \
                 AND created_at >= NOW() - INTERVAL '{}'",
                days
            ),
            true,
        ),
        None => (
            "SELECT COUNT(*) FROM programmatic_pages \
             WHERE $1 = ANY(mentioned_business_ids) AND status = 'published'"
                .to_string(),
            false,
        ),
    };

    let count: (i64,) = if bind_interval {
        let (c,) = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(business_id)
            .fetch_one(db)
            .await?;
        (c,)
    } else {
        let (c,) = sqlx::query_as::<_, (i64,)>(&sql)
            .bind(business_id)
            .fetch_one(db)
            .await?;
        (c,)
    };

    Ok(count.0)
}

// ── Local helper type (mirrors portal's ClaimedBusinessRow for local use) ──

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClaimedBusinessRow {
    pub id: Uuid,
    pub business_id: Uuid,
    pub owner_email: String,
    pub owner_name: Option<String>,
    pub owner_phone: Option<String>,
    pub user_id: Option<Uuid>,
    pub is_active: Option<bool>,
    pub created_at: Option<chrono::DateTime<Utc>>,
}
