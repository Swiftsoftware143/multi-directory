//! Admin "Newsletter" management — per-city newsletter advert + registration URL
//! (work-queue t_5306e167 follow-up, David 2026-11-25).
//!
//! Each city runs its OWN newsletter on its own domain (Letterman). The public pages show that
//! city's advert image (served from /uploads/newsletter/<slug>.jpg + @2x) and link it to the
//! city's registration page. The admin sets both here instead of a code change.
//!
//!   GET  /admin/newsletter-config             list every directory's slug/name/registration URL
//!   PUT  /admin/newsletter-config/:slug       set the registration URL (blank = clear)
//!   POST /admin/newsletter-config/:slug/image upload the advert as JSON base64
//!                                              (`image_base64` = 1x, `image2x_base64` = 2x optional)
//!
//! The advert bytes are stored in the PERSISTENT uploads root (bind-mounted
//! `/opt/swift/www/zaarhub.com/uploads`, NOT the git checkout), so a rebuild cannot wipe them.
//! Auth: `auth_guard` accepts an issued `md_…` API key (it resolves the key to the owner's session
//! and rewrites the header to a short-lived JWT), and `operator_guard` then checks the role — so
//! both a console session and the ZaarHub CEO bot's issued key reach these routes.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;

use crate::error::{ApiResult, AppError};
use crate::AppState;

/// The persistent uploads root — the same bind-mounted store the audience media lives in
/// (`audience::serve_media` serves it back through `/uploads/*`). NOT git-tracked.
const UPLOADS_ROOT: &str = "/opt/swift/www/zaarhub.com/uploads";

/// GET /admin/newsletter-config
pub async fn list_newsletter_config(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(
        "SELECT slug, name, newsletter_registration_url FROM directories ORDER BY name",
    )
    .fetch_all(&s.db)
    .await?;

    Ok(Json(
        json!(rows.iter().map(|r| json!({
            "slug": r.try_get::<String, _>("slug").unwrap_or_default(),
            "name": r.try_get::<String, _>("name").unwrap_or_default(),
            "registration_url": r.try_get::<Option<String>, _>("newsletter_registration_url").ok().flatten(),
        })).collect::<Vec<_>>()),
    ))
}

#[derive(Debug, Deserialize)]
pub struct NewsletterConfigUpdate {
    pub registration_url: Option<String>,
}

/// PUT /admin/newsletter-config/:slug
pub async fn update_newsletter_config(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<NewsletterConfigUpdate>,
) -> ApiResult<impl IntoResponse> {
    let url = body
        .registration_url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());

    let result =
        sqlx::query("UPDATE directories SET newsletter_registration_url = $1 WHERE slug = $2")
            .bind(&url)
            .bind(&slug)
            .execute(&s.db)
            .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    Ok(Json(json!({ "slug": slug, "registration_url": url })))
}

fn is_safe_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The persistent newsletter advert directory (bind-mounted, survives rebuild).
fn newsletter_dir() -> std::path::PathBuf {
    std::path::Path::new(UPLOADS_ROOT).join("newsletter")
}

#[derive(Debug, Deserialize)]
pub struct NewsletterImageUpload {
    pub image_base64: String,
    #[serde(default)]
    pub image2x_base64: Option<String>,
}

/// POST /admin/newsletter-config/:slug/image — JSON base64 upload (1x + optional 2x).
pub async fn upload_newsletter_image(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<NewsletterImageUpload>,
) -> ApiResult<impl IntoResponse> {
    if !is_safe_slug(&slug) {
        return Err(AppError::Validation("invalid slug".to_string()));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM directories WHERE slug = $1)")
            .bind(&slug)
            .fetch_one(&s.db)
            .await?;
    if !exists {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    use base64::Engine;
    let img1 = base64::engine::general_purpose::STANDARD
        .decode(body.image_base64.trim())
        .map_err(|e| AppError::Validation(format!("bad base64 (1x): {e}")))?;
    let img2x = match body.image2x_base64 {
        Some(b2) if !b2.trim().is_empty() => base64::engine::general_purpose::STANDARD
            .decode(b2.trim())
            .map_err(|e| AppError::Validation(format!("bad base64 (2x): {e}")))?,
        _ => img1.clone(),
    };

    let dir = newsletter_dir();
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::write(dir.join(format!("{}.jpg", slug)), &img1)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::write(dir.join(format!("{}@2x.jpg", slug)), &img2x)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(json!({
        "slug": slug,
        "url": format!("/uploads/newsletter/{slug}.jpg"),
        "saved": true,
    })))
}
