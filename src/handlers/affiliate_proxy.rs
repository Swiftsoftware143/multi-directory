//! Affiliate Proxy — routes directory business owner affiliate management
//! requests to FunnelSwift's affiliate system.
//!
//! These routes sit INSIDE the MD auth guard (require MD JWT).
//! They resolve the MD user -> FS tenant/affiliate and generate an FS-compatible
//! JWT on-the-fly to proxy the request through.
//!
//! FunnelSwift base URL: http://127.0.0.1:8080 (same host)
//! FS JWT secret shared via FS_JWT_SECRET env var.

use axum::{
    extract::{Extension, State},
    response::IntoResponse,
    Json,
};
use reqwest::Client;
use serde_json::{json, Value};
use uuid::Uuid;
use jsonwebtoken::{encode, EncodingKey, Header};

use crate::AppState;
use crate::auth::models::Claims;
use crate::error::{AppError, ApiResult};

const FS_BASE: &str = "http://127.0.0.1:8080/api/v1";

fn http() -> Client {
    Client::new()
}

/// Generate an FS-compatible JWT using jsonwebtoken
fn make_fs_jwt(
    md_user_id: &str,
    email: &str,
    fs_secret: &str,
    tenant_id: &str,
    aff_id: &str,
) -> Result<String, AppError> {
    use std::collections::HashMap;
    let now = chrono::Utc::now().timestamp() as usize;
    let mut claims = HashMap::new();
    claims.insert("sub", serde_json::Value::String(md_user_id.to_string()));
    claims.insert("aff_id", serde_json::Value::String(aff_id.to_string()));
    claims.insert("tenant_id", serde_json::Value::String(tenant_id.to_string()));
    claims.insert("iat", serde_json::Value::Number(now.into()));
    claims.insert("exp", serde_json::Value::Number((now + 300).into()));

    let header = Header::new(jsonwebtoken::Algorithm::HS256);
    encode(&header, &claims, &EncodingKey::from_secret(fs_secret.as_bytes()))
        .map_err(|e| AppError::Internal(format!("FS JWT encode failed: {}", e)))
}

/// Look up FS tenant info by MD user email from the FS database
/// Returns (tenant_id, affiliate_id, email) or creates a stub if not found
async fn resolve_fs_tenant(
    s: &AppState,
    md_claims: &Claims,
) -> Result<(String, String, String), AppError> {
    let user_id = Uuid::parse_str(&md_claims.sub)
        .map_err(|_| AppError::Unauthorized)?;

    // Get email from MD users table
    let email: String = sqlx::query_scalar(
        "SELECT email FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("DB lookup failed".into()))?
    .ok_or_else(|| AppError::NotFound("User not found in MD".into()))?;

    // Look up the user's FS database connection (same PostgreSQL)
    // Check if the user has any businesses claimed in MD
    let claimed_biz: Option<(i64,)> = sqlx::query_as(
        "SELECT COUNT(*) FROM claimed_businesses WHERE user_id = $1"
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("DB lookup failed".into()))?;

    let biz_count = claimed_biz.map(|(c,)| c).unwrap_or(0);

    // For now, tenants in FS are looked up by email (user email = tenant owner email)
    // Or we use a default tenant ID (the one created for the MD app)
    // First try: find a tenant in FS whose owner has this email
    let fs_tenant: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM tenants WHERE owner_email = $1 OR id = (SELECT MIN(id)::text FROM tenants) LIMIT 1"
    )
    .bind(&email)
    .fetch_optional(&s.db) // FunnelSwift shares the same DB
    .await
    .map_err(|_| AppError::Internal("FS tenant lookup failed".into()))?;

    let tenant_id = match fs_tenant {
        Some((id,)) => id,
        None => {
            // Fallback: use a generic tenant. Return error if no tenant exists.
            let any_tenant: Option<(String,)> = sqlx::query_as(
                "SELECT id::text FROM tenants ORDER BY created_at ASC LIMIT 1"
            )
            .fetch_optional(&s.db)
            .await
            .map_err(|_| AppError::Internal("FS tenant lookup failed".into()))?;

            any_tenant
                .map(|(id,)| id)
                .ok_or_else(|| AppError::NotFound("No tenant configured in FunnelSwift".into()))?
        }
    };

    // Check if this user has an affiliate account in FS
    let affiliate_id: String = sqlx::query_scalar(
        "SELECT id FROM affiliate_users WHERE email = $1 LIMIT 1"
    )
    .bind(&email)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("FS affiliate lookup failed".into()))?
    .unwrap_or_else(|| format!("AFF-NONE-{}", &Uuid::new_v4().to_string()[..8].to_uppercase()));

    Ok((tenant_id, affiliate_id, email))
}

/// Proxy a POST to FunnelSwift with the generated FS JWT
async fn proxy_post(
    path: &str,
    body: &Value,
    fs_secret: &str,
    tenant_id: &str,
    aff_id: &str,
    email: &str,
    md_user_id: &str,
) -> Result<Value, AppError> {
    let token = make_fs_jwt(md_user_id, email, fs_secret, tenant_id, aff_id)?;
    let url = format!("{}{}", FS_BASE, path);

    let resp = http()
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("FS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("FS parse failed: {}", e)))
}

/// Proxy a GET to FunnelSwift with the generated FS JWT
async fn proxy_get(
    path: &str,
    fs_secret: &str,
    tenant_id: &str,
    aff_id: &str,
    email: &str,
    md_user_id: &str,
) -> Result<Value, AppError> {
    let token = make_fs_jwt(md_user_id, email, fs_secret, tenant_id, aff_id)?;
    let url = format!("{}{}", FS_BASE, path);

    let resp = http()
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("FS request failed: {}", e)))?;

    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("FS parse failed: {}", e)))
}

// ── Connect / Signup ──

/// POST /affiliate/connect — sign up as an affiliate for FunnelSwift
pub async fn affiliate_signup(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    let payload = json!({
        "email": email,
        "first_name": body.get("first_name").and_then(|v| v.as_str()).unwrap_or(""),
        "last_name": body.get("last_name").and_then(|v| v.as_str()).unwrap_or(""),
        "password": body.get("password").and_then(|v| v.as_str()).unwrap_or_default(),
        "selected_apps": body.get("selected_apps").and_then(|v| v.as_array()).map(|a| {
            a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "affiliate_code": body.get("affiliate_code").and_then(|v| v.as_str()).map(String::from),
    });

    let result = proxy_post(
        "/affiliate/signup",
        &payload,
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}

/// POST /affiliate/login — login as FunnelSwift affiliate
pub async fn affiliate_login(
    State(s): State<AppState>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    let email = body.get("email")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("email required".into()))?;

    let tenant_id: String = sqlx::query_scalar(
        "SELECT id::text FROM tenants ORDER BY created_at ASC LIMIT 1"
    )
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("DB lookup failed".into()))?
    .unwrap_or_default();

    let aff_id: String = sqlx::query_scalar(
        "SELECT id FROM affiliate_users WHERE email = $1 LIMIT 1"
    )
    .bind(email)
    .fetch_optional(&s.db)
    .await
    .map_err(|_| AppError::Internal("FS lookup failed".into()))?
    .unwrap_or_default();

    let result = proxy_post(
        "/affiliate/login",
        &body,
        &secret,
        &tenant_id,
        &aff_id,
        email,
        "00000000-0000-0000-0000-000000000000",
    ).await?;

    Ok(Json(result))
}

/// GET /affiliate/dashboard — get affiliate dashboard data from FS
pub async fn affiliate_dashboard(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    // Proxy the dashboard call
    let result = proxy_post(
        "/affiliate/dashboard",
        &json!({
            "token": ""
        }),
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}

/// GET /affiliate/links — list affiliate links from FS
pub async fn affiliate_links(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    let result = proxy_get(
        "/affiliate-links",
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}

/// POST /affiliate/links — create an affiliate link via FS
pub async fn create_affiliate_link(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    // Ensure the affiliate_id is included
    let payload = json!({
        "affiliate_id": body.get("affiliate_id").and_then(|v| v.as_str()).unwrap_or(&aff_id),
        "target_app": body.get("target_app").and_then(|v| v.as_str()).unwrap_or("directory"),
        "product_id": body.get("product_id"),
        "target_url": body.get("target_url"),
    });

    let result = proxy_post(
        "/affiliate-links",
        &payload,
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}

/// GET /affiliate/stats — get affiliate stats from FS
pub async fn affiliate_stats(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    let result = proxy_get(
        "/affiliate-stats",
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}

/// GET /affiliate/products — list affiliate products from FS
pub async fn affiliate_products(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    let result = proxy_get(
        "/affiliate-products",
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}

/// GET /affiliate/conversions — list affiliate conversions from FS
pub async fn affiliate_conversions(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let (tenant_id, aff_id, email) = resolve_fs_tenant(&s, &claims).await?;
    let secret = std::env::var("FS_JWT_SECRET")
        .unwrap_or_else(|_| s.config.jwt_secret.clone());

    let result = proxy_get(
        "/affiliate-conversions",
        &secret,
        &tenant_id,
        &aff_id,
        &email,
        &claims.sub,
    ).await?;

    Ok(Json(result))
}
