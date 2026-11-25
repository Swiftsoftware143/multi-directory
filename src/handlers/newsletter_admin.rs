//! Admin "Newsletter" management — per-city newsletter advert + registration URL
//! (work-queue t_5306e167 follow-up, David 2026-11-25).
//!
//! Each city runs its OWN newsletter on its own domain (Letterman). The public pages show that
//! city's advert image (served from /assets/newsletter/<slug>.jpg + @2x) and link it to the
//! city's registration page. The admin sets both here instead of a code change.
//!
//!   GET  /admin/newsletter-config             list every directory's slug/name/registration URL
//!   PUT  /admin/newsletter-config/:slug       set the registration URL (blank = clear)
//!   POST /admin/newsletter-config/:slug/image multipart upload of the advert (fields `image`
//!                                              = 1x, `image2x` = 2x; 2x optional, falls back to 1x)
//!
//! Operator-guarded (same as the rest of the /admin surface).

use axum::{
    extract::{Multipart, Path, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;

use crate::error::{ApiResult, AppError};
use crate::AppState;

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

/// Resolve the newsletter assets directory (the same bind-mounted `frontend/assets/newsletter`
/// the SPA static server reads). Prefers the container path, then the baremetal checkout.
fn newsletter_assets_dir() -> Result<std::path::PathBuf, AppError> {
    [
        "/opt/swift/multidirectory-rust/frontend/assets/newsletter",
        "/opt/swift/apps/multi-directory/frontend/assets/newsletter",
        "./frontend/assets/newsletter",
    ]
    .iter()
    .map(std::path::Path::new)
    .find(|p| p.is_dir())
    .map(|p| p.to_path_buf())
    .ok_or_else(|| AppError::Internal("newsletter assets dir not found".to_string()))
}

fn is_safe_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// POST /admin/newsletter-config/:slug/image
pub async fn upload_newsletter_image(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    mut multipart: Multipart,
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

    let mut img1: Option<Vec<u8>> = None;
    let mut img2x: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::Validation(e.to_string()))?;
        if bytes.is_empty() {
            continue;
        }
        match name.as_str() {
            "image" => img1 = Some(bytes.to_vec()),
            "image2x" => img2x = Some(bytes.to_vec()),
            _ => {}
        }
    }

    let img1 = img1.ok_or_else(|| AppError::Validation("missing 'image' file (1x)".to_string()))?;
    let img2x = img2x.unwrap_or_else(|| img1.clone());

    let dir = newsletter_assets_dir()?;
    std::fs::write(dir.join(format!("{}.jpg", slug)), &img1)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::write(dir.join(format!("{}@2x.jpg", slug)), &img2x)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(json!({ "slug": slug, "saved": true })))
}
