//! Public IncentiveSwift campaign widget — serves a standalone HTML embed for a business's
//! campaign, mirroring the booking page (increment 1 of the ECOSYSTEM WIDGETS work).
//!
//! A visitor stays INSIDE the directory: the business detail page renders this route in an
//! iframe when the operator has turned the IncentiveSwift campaign widget on for that business
//! (`businesses.incentive_widget_enabled`) and set its external campaign slug
//! (`businesses.incentive_campaign_slug`). This page resolves the IncentiveSwift base URL at
//! call time and embeds IncentiveSwift's own embeddable campaign surface, so the offer capture
//! runs natively inside IncentiveSwift without the directory ever proxying leads.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::AppState;

/// IncentiveSwift base URL — read from INCENTIVESWIFT_URL at runtime, fallback localhost:8083.
/// Mirrors `coreswift::coreswift_url()`; never panics, so an unconfigured host degrades to the
/// local default rather than aborting (this crate is built with `panic = "abort"`).
fn incentiveswift_url() -> String {
    std::env::var("INCENTIVESWIFT_URL").unwrap_or_else(|_| "http://localhost:8083".to_string())
}

/// GET /api/v1/campaign/:slug/:business_id
///
/// Renders a public HTML page that embeds the business's IncentiveSwift campaign. Looks up the
/// directory + business, resolves the campaign slug, and frames IncentiveSwift's embeddable
/// campaign surface. A business with no campaign slug configured returns 404 (the business page
/// never renders the widget for such a business, so this is a defensive arm).
pub async fn campaign_widget(
    State(s): State<AppState>,
    Path((slug, business_id_or_slug)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    // 1. Directory by slug — its id scopes the business lookup, its name decorates the header.
    let (dir_id, dir_name): (Uuid, String) =
        sqlx::query_as("SELECT id, name FROM directories WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound(format!(
                "Directory '{}' not found",
                slug
            )))?;

    // 2. Business by UUID or slug — we need its name and its IncentiveSwift campaign slug.
    let business = if let Ok(bid) = Uuid::parse_str(&business_id_or_slug) {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT name, incentive_campaign_slug FROM businesses WHERE id = $1 AND directory_id = $2",
        )
        .bind(bid)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    } else {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT name, incentive_campaign_slug FROM businesses WHERE slug = $1 AND directory_id = $2",
        )
        .bind(&business_id_or_slug)
        .bind(dir_id)
        .fetch_optional(&s.db)
        .await?
    };

    let (business_name, campaign_slug) =
        business.ok_or(AppError::NotFound("Business not found".to_string()))?;

    let campaign_slug = campaign_slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(AppError::NotFound(
            "This business has no campaign configured".to_string(),
        ))?;

    // 3. Frame IncentiveSwift's embeddable campaign surface (public, no key needed).
    let embed_url = format!(
        "{}/api/v1/embed/campaign/{}",
        incentiveswift_url(),
        campaign_slug
    );

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{business_title} — Special Offer</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #fff; }}
        .campaign-header {{
            padding: 12px 16px;
            background: #f7f7f8;
            border-bottom: 1px solid #e5e5e6;
            font-size: 15px;
            font-weight: 600;
            color: #1f2937;
        }}
        .campaign-sub {{ font-size: 12px; font-weight: 400; color: #6b7280; margin-top: 2px; }}
        .campaign-frame {{ width: 100%; height: 420px; border: 0; display: block; }}
    </style>
</head>
<body>
    <div class="campaign-header">{business_title}<div class="campaign-sub">A special offer from {dir_name}</div></div>
    <iframe class="campaign-frame" src="{embed_url}" title="Special offer from {business_title}" loading="lazy"></iframe>
</body>
</html>"#,
        business_title = html_escape(&business_name),
        dir_name = html_escape(&dir_name),
        embed_url = html_escape(&embed_url),
    );

    Ok(axum::response::Html(html))
}

/// Escape a string for safe interpolation into HTML text/attribute positions.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
