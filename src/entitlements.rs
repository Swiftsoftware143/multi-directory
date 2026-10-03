//! Plan entitlement enforcement (kanban B113).
//!
//! David's rule: *"Every gate must be enforced in the BACK END — a front-end-only gate is not a
//! gate."* This module is the single shared entitlement check every gated write path calls.
//!
//! DESIGN DECISION (recorded on B113): a business is gated ONLY by an ACTIVE paid row in
//! `business_subscriptions`. A business with no active subscription is *not* limited at all, which
//! preserves today's behaviour exactly for the ~4,000 unsubscribed listings — so shipping this
//! layer cannot change the live directory. Enforcement can only ever restrict a business that has
//! deliberately bought a plan.
//!
//! Usage:
//!     if let Some(limits) = entitlements::limits_for(&s.db, business_id).await? {
//!         limits.require_active_deal(currently_active)?;
//!     }
//!
//! Limits convention (matches `plan_tiers`): `None` = not configured (no gate),
//! a negative `i32` = unlimited, `0` = none allowed.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// The effective limits of a business's active plan. Populated from the joined `plan_tiers` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlanLimits {
    pub tier_name: Option<String>,
    pub max_listings: Option<i32>,
    pub max_deals: Option<i32>,
    pub max_active_deals: Option<i32>,
    pub max_photos: Option<i32>,
    pub max_categories: Option<i32>,
    pub max_industries: Option<i32>,
    pub max_scheduled_rotations: Option<i32>,
    pub featured_listing: Option<bool>,
    pub has_reviews: Option<bool>,
    pub has_analytics: Option<bool>,
    pub has_crm: Option<bool>,
    pub has_email: Option<bool>,
    pub has_call_tracking: Option<bool>,
    pub has_import_export: Option<bool>,
    pub has_api_access: Option<bool>,
    pub allow_custom_branding: Option<bool>,
}

/// Returns the business's active plan limits, or `None` when it has no active subscription
/// (in which case NO gate applies and behaviour is unchanged).
pub async fn limits_for(db: &PgPool, business_id: Uuid) -> Result<Option<PlanLimits>, AppError> {
    let limits = sqlx::query_as::<_, PlanLimits>(
        r#"
        SELECT pt.name AS tier_name,
               pt.max_listings,
               pt.max_deals,
               pt.max_active_deals,
               pt.max_photos,
               pt.max_categories,
               pt.max_industries,
               pt.max_scheduled_rotations,
               pt.featured_listing,
               pt.has_reviews,
               pt.has_analytics,
               pt.has_crm,
               pt.has_email,
               pt.has_call_tracking,
               pt.has_import_export,
               pt.has_api_access,
               pt.allow_custom_branding
        FROM business_subscriptions bs
        JOIN plan_tiers pt ON pt.id = bs.tier_id
        WHERE bs.business_id = $1
          AND bs.status = 'active'
          AND (bs.end_date IS NULL OR bs.end_date >= CURRENT_DATE)
        ORDER BY bs.created_at DESC
        LIMIT 1
        "#,
    )
    .bind(business_id)
    .fetch_optional(db)
    .await?;

    Ok(limits)
}

/// `true` when `value` represents "unlimited": unset (None) or negative.
fn unlimited(value: Option<i32>) -> bool {
    match value {
        None => true,
        Some(n) => n < 0,
    }
}

impl PlanLimits {
    fn plan_name(&self) -> &str {
        self.tier_name.as_deref().unwrap_or("your plan")
    }

    /// A count gate: reject when adding one more would exceed `limit`.
    fn require_room(
        &self,
        label: &str,
        limit: Option<i32>,
        current: i64,
        adding: i64,
    ) -> Result<(), AppError> {
        if unlimited(limit) {
            return Ok(());
        }
        let max = limit.unwrap_or(0) as i64;
        if current + adding > max {
            return Err(AppError::Forbidden(format!(
                "Your {} plan allows {} {}. You have {}; upgrade to add more.",
                self.plan_name(),
                max,
                label,
                current
            )));
        }
        Ok(())
    }

    /// A boolean capability gate.
    fn require_feature(&self, enabled: Option<bool>, label: &str) -> Result<(), AppError> {
        if enabled == Some(false) {
            return Err(AppError::Forbidden(format!(
                "{} is not included in your {} plan. Upgrade to enable it.",
                label,
                self.plan_name()
            )));
        }
        Ok(())
    }

    pub fn require_new_deal(&self, current_total: i64) -> Result<(), AppError> {
        self.require_room("deals", self.max_deals, current_total, 1)
    }

    pub fn require_active_deal(&self, current_active: i64) -> Result<(), AppError> {
        self.require_room("active deals", self.max_active_deals, current_active, 1)
    }

    pub fn require_photos(&self, current: i64, adding: i64) -> Result<(), AppError> {
        self.require_room("photos", self.max_photos, current, adding)
    }

    pub fn require_featured(&self) -> Result<(), AppError> {
        self.require_feature(self.featured_listing, "Featured listings")
    }

    pub fn require_scheduled_rotation(&self) -> Result<(), AppError> {
        self.require_feature(
            // max_scheduled_rotations <= 0 means the capability is off.
            self.max_scheduled_rotations.map(|n| n > 0),
            "Scheduled deal rotation",
        )
    }

    pub fn require_api_access(&self) -> Result<(), AppError> {
        self.require_feature(self.has_api_access, "API access")
    }

    pub fn require_import_export(&self) -> Result<(), AppError> {
        self.require_feature(self.has_import_export, "Import / export")
    }

    pub fn require_crm(&self) -> Result<(), AppError> {
        self.require_feature(self.has_crm, "CRM integration")
    }

    pub fn require_email(&self) -> Result<(), AppError> {
        self.require_feature(self.has_email, "Email campaigns")
    }

    pub fn require_analytics(&self) -> Result<(), AppError> {
        self.require_feature(self.has_analytics, "Analytics")
    }

    pub fn require_call_tracking(&self) -> Result<(), AppError> {
        self.require_feature(self.has_call_tracking, "Call tracking")
    }

    pub fn require_reviews(&self) -> Result<(), AppError> {
        self.require_feature(self.has_reviews, "Review responses")
    }

    pub fn require_custom_branding(&self) -> Result<(), AppError> {
        self.require_feature(self.allow_custom_branding, "Custom branding")
    }

    /// Convenience: count helper for callers that do not already have the count.
    pub async fn count_active_deals(
        &self,
        db: &PgPool,
        business_id: Uuid,
    ) -> Result<i64, AppError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM deals WHERE business_id = $1 AND status = 'active'",
        )
        .bind(business_id)
        .fetch_one(db)
        .await?;
        Ok(n)
    }
}

/// Fetch-and-enforce in one call for the common "business may add one more X" case.
/// Returns `Ok(())` for ungated (unsubscribed) businesses.
pub async fn require_deal_slot(
    db: &PgPool,
    business_id: Uuid,
    status: &str,
    featured: bool,
    rotation: bool,
) -> Result<(), AppError> {
    let Some(limits) = limits_for(db, business_id).await? else {
        return Ok(());
    };
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM deals WHERE business_id = $1")
        .bind(business_id)
        .fetch_one(db)
        .await?;
    limits.require_new_deal(total)?;
    if status == "active" {
        let active = limits.count_active_deals(db, business_id).await?;
        limits.require_active_deal(active)?;
    }
    if featured {
        limits.require_featured()?;
    }
    if rotation {
        limits.require_scheduled_rotation()?;
    }
    Ok(())
}

/// Photo-slot enforcement; returns `Ok(())` for ungated businesses.
pub async fn require_photo_slot(
    db: &PgPool,
    business_id: Uuid,
    adding: i64,
) -> Result<(), AppError> {
    let Some(limits) = limits_for(db, business_id).await? else {
        return Ok(());
    };
    let current: i64 = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_array_length(images), 0)::bigint FROM businesses WHERE id = $1",
    )
    .bind(business_id)
    .fetch_optional(db)
    .await?
    .unwrap_or(0);
    limits.require_photos(current, adding)
}
