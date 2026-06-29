//! Directory branding handlers.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::models::*;

/// GET /api/v1/directories/:slug/branding
pub async fn get_branding(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query_as::<_, Directory>(
        "SELECT * FROM directories WHERE slug = \x241 "
    )
    .bind(&slug)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound(format!("Directory '{}' not found", slug)))?;

    let branding = sqlx::query_as::<_, DirectoryBranding>(
        "SELECT * FROM directory_branding WHERE directory_id = \x241 "
    )
    .bind(dir.id)
    .fetch_optional(&s.db)
    .await?;

    Ok(Json(json!(branding)))
}

/// PUT /api/v1/admin/branding/:directory_id
pub async fn update_branding(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<UpdateBrandingRequest>,
) -> ApiResult<impl IntoResponse> {
    // Check directory exists
    let dir_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = \x241 "
    )
    .bind(directory_id)
    .fetch_one(&s.db)
    .await?;

    if dir_exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    // Upsert branding
    let branding = sqlx::query_as::<_, DirectoryBranding>(
        r#"INSERT INTO directory_branding (directory_id, primary_color, secondary_color, accent_color,
           background_color, text_color, heading_color, link_color, button_background, button_text,
           heading_font, body_font, logo_url, favicon_url, meta_title_template, meta_description_template)
           VALUES (\x241, \x242, \x243, \x244, \x245, \x246, \x247, \x248, \x249, \x2410, \x2411, \x2412, \x2413, \x2414, \x2415, \x2416)
           ON CONFLICT (directory_id) DO UPDATE SET
           primary_color = COALESCE(\x242, directory_branding.primary_color),
           secondary_color = COALESCE(\x243, directory_branding.secondary_color),
           accent_color = COALESCE(\x244, directory_branding.accent_color),
           background_color = COALESCE(\x245, directory_branding.background_color),
           text_color = COALESCE(\x246, directory_branding.text_color),
           heading_color = COALESCE(\x247, directory_branding.heading_color),
           link_color = COALESCE(\x248, directory_branding.link_color),
           button_background = COALESCE(\x249, directory_branding.button_background),
           button_text = COALESCE(\x2410, directory_branding.button_text),
           heading_font = COALESCE(\x2411, directory_branding.heading_font),
           body_font = COALESCE(\x2412, directory_branding.body_font),
           logo_url = COALESCE(\x2413, directory_branding.logo_url),
           favicon_url = COALESCE(\x2414, directory_branding.favicon_url),
           meta_title_template = COALESCE(\x2415, directory_branding.meta_title_template),
           meta_description_template = COALESCE(\x2416, directory_branding.meta_description_template),
           updated_at = NOW()
           RETURNING *"#
    )
    .bind(directory_id)
    .bind(&req.primary_color)
    .bind(&req.secondary_color)
    .bind(&req.accent_color)
    .bind(&req.background_color)
    .bind(&req.text_color)
    .bind(&req.heading_color)
    .bind(&req.link_color)
    .bind(&req.button_background)
    .bind(&req.button_text)
    .bind(&req.heading_font)
    .bind(&req.body_font)
    .bind(&req.logo_url)
    .bind(&req.favicon_url)
    .bind(&req.meta_title_template)
    .bind(&req.meta_description_template)
    .fetch_one(&s.db)
    .await?;

    Ok(Json(json!(branding)))
}

/// POST /api/v1/admin/branding/:directory_id/extract
pub async fn extract_colors(
    State(s): State<AppState>,
    Path(directory_id): Path<Uuid>,
    Json(req): Json<ExtractColorsRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.url.is_empty() {
        return Err(AppError::Validation("URL is required".to_string()));
    }

    // Fetch the website and try to extract colors
    let colors = match extract_colors_from_url(&req.url).await {
        Ok(colors) => colors,
        Err(e) => return Err(AppError::BadRequest(format!("Failed to extract colors: {}", e))),
    };

    // Update branding with extracted colors
    let branding = sqlx::query_as::<_, DirectoryBranding>(
        r#"UPDATE directory_branding SET
           primary_color = COALESCE(\x241, primary_color),
           secondary_color = COALESCE(\x242, secondary_color),
           accent_color = COALESCE(\x243, accent_color),
           background_color = COALESCE(\x244, background_color),
           text_color = COALESCE(\x245, text_color),
           heading_color = \x246,
           extracted_from_url = \x247,
           updated_at = NOW()
           WHERE directory_id = \x248
           RETURNING *"#
    )
    .bind(&colors.primary_color)
    .bind(&colors.secondary_color)
    .bind(&colors.accent_color)
    .bind(&colors.background_color)
    .bind(&colors.text_color)
    .bind(&colors.heading_color)
    .bind(&req.url)
    .bind(directory_id)
    .fetch_optional(&s.db)
    .await?;

    match branding {
        Some(b) => Ok(Json(json!(b))),
        None => Err(AppError::NotFound("Branding not found for this directory. Create branding first.".to_string())),
    }
}

/// Color extraction result
#[derive(Debug, serde::Serialize)]
struct ExtractedColors {
    primary_color: String,
    secondary_color: String,
    accent_color: String,
    background_color: String,
    text_color: String,
    heading_color: String,
}

/// Extract colors from a website URL by analyzing CSS and meta tags
async fn extract_colors_from_url(url: &str) -> Result<ExtractedColors, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("MultiDirectory-ColorExtractor/1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;

    let html = resp.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Simple color extraction from inline styles and CSS
    let mut primary = String::new();
    let mut background = String::new();
    let mut text = String::new();
    let mut accent = String::new();

    // Look for common CSS color patterns
    for line in html.lines() {
        if primary.is_empty() {
            if let Some(c) = extract_hex_color(line, &["--primary", "primary-color", "brand-primary"]) {
                primary = c;
            }
        }
        if background.is_empty() {
            if let Some(c) = extract_hex_color(line, &["--bg", "background-color", "--background"]) {
                background = c;
            }
        }
        if text.is_empty() {
            if let Some(c) = extract_hex_color(line, &["--text", "text-color", "color:", "--color-body"]) {
                text = c;
            }
        }
        if accent.is_empty() {
            if let Some(c) = extract_hex_color(line, &["--accent", "accent-color", "--secondary"]) {
                accent = c;
            }
        }
    }

    Ok(ExtractedColors {
        primary_color: if primary.is_empty() { "#3B82F6".to_string() } else { primary },
        secondary_color: if accent.is_empty() { "#10B981".to_string() } else { accent.clone() },
        accent_color: if accent.is_empty() { "#F59E0B".to_string() } else { accent },
        background_color: if background.is_empty() { "#FFFFFF".to_string() } else { background },
        text_color: if text.is_empty() { "#1F2937".to_string() } else { text },
        heading_color: "#111827".to_string(),
    })
}

/// Extract hex color from a line containing CSS variable names
fn extract_hex_color(line: &str, keywords: &[&str]) -> Option<String> {
    let lower = line.to_lowercase();
    if !keywords.iter().any(|k| lower.contains(k)) {
        return None;
    }

    // Find hex color patterns
    for part in line.split(|c: char| c == ':' || c == ';' || c == ' ' || c == '"' || c == '\'') {
        let part = part.trim();
        if part.starts_with('#') && part.len() == 7 {
            return Some(part.to_string());
        }
    }

    None
}
