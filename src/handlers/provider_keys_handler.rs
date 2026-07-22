//! Provider keys CRUD â€” tenant-scoped API key storage per provider.
//!
//! Endpoints:
//! - GET    /api/v1/admin/provider-keys          (list tenant keys, masked)
//! - POST   /api/v1/admin/provider-keys          (upsert a key for a provider)
//! - DELETE /api/v1/admin/provider-keys/:provider (remove a key)
//! - GET    /api/v1/available-providers           (public list)

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;
use sqlx::Row;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::auth::models::Claims;

#[derive(Debug, Deserialize)]
pub struct UpsertProviderKeyRequest {
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub is_active: Option<bool>,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderKeyResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub api_key: String,  // masked in response
    pub base_url: Option<String>,
    pub metadata: Value,
    pub is_active: bool,
    pub scope: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct AvailableProviderResponse {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub requires_base_url: bool,
    pub requires_metadata: Value,
    pub icon: Option<String>,
}

/// Mask an API key showing only first 4 and last 4 characters.
fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        "****".to_string()
    } else {
        let first4 = &key[..4];
        let last4 = &key[key.len()-4..];
        format!("{}...{}", first4, last4)
    }
}

async fn validate_provider_exists(db: &sqlx::PgPool, provider: &str) -> Result<(), AppError> {
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM available_providers WHERE key = $1"
    )
    .bind(provider)
    .fetch_one(db)
    .await?;

    if exists == 0 {
        return Err(AppError::NotFound(format!(
            "Provider '{}' is not supported", provider
        )));
    }
    Ok(())
}

/// GET /api/v1/admin/provider-keys
pub async fn list_provider_keys(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid)
        .map_err(|_| AppError::Unauthorized)?;

    let rows = sqlx::query(
        r#"SELECT id, tenant_id, provider, 
                decrypt_provider_key(api_key_encrypted) as api_key,
                CASE WHEN base_url_encrypted IS NOT NULL 
                    THEN decrypt_provider_key(base_url_encrypted) 
                    ELSE NULL END as base_url,
                metadata, is_active, scope, 
                created_at::text, updated_at::text
         FROM provider_keys 
         WHERE tenant_id = $1 
         ORDER BY provider ASC"#
    )
    .bind(tenant_id)
    .fetch_all(&s.db)
    .await?;

    let keys: Vec<ProviderKeyResponse> = rows.iter().map(|row| {
        ProviderKeyResponse {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            provider: row.get("provider"),
            api_key: mask_key(row.get::<String, _>("api_key").as_str()),
            base_url: row.get("base_url"),
            metadata: row.get("metadata"),
            is_active: row.get("is_active"),
            scope: row.get("scope"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }).collect();

    Ok(Json(json!({
        "success": true,
        "data": keys
    })))
}

/// POST /api/v1/admin/provider-keys
pub async fn upsert_provider_key(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Json(req): Json<UpsertProviderKeyRequest>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid)
        .map_err(|_| AppError::Unauthorized)?;

    // Validate that the provider is in the available list
    validate_provider_exists(&s.db, &req.provider).await?;

    let metadata = req.metadata.unwrap_or(json!({}));
    let is_active = req.is_active.unwrap_or(true);
    let scope = req.scope.unwrap_or_else(|| "tenant".to_string());

    // Store plaintext api_key in api_key column — trigger auto-encrypts to api_key_encrypted
    let row = sqlx::query(
        "INSERT INTO provider_keys (tenant_id, provider, api_key, base_url, metadata, is_active, scope) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (tenant_id, provider) \
         DO UPDATE SET api_key = EXCLUDED.api_key, \
                       base_url = EXCLUDED.base_url, \
                       metadata = EXCLUDED.metadata, \
                       is_active = EXCLUDED.is_active, \
                       scope = EXCLUDED.scope, \
                       updated_at = NOW() \
         RETURNING id, tenant_id, provider, \
                   decrypt_provider_key(api_key_encrypted) as api_key, \
                   CASE WHEN base_url_encrypted IS NOT NULL \
                       THEN decrypt_provider_key(base_url_encrypted) \
                       ELSE NULL END as base_url, \
                   metadata, is_active, scope, \
                   created_at::text, updated_at::text"
    )
    .bind(tenant_id)
    .bind(&req.provider)
    .bind(&req.api_key)  // plaintext — trigger encrypts
    .bind(&req.base_url)
    .bind(&metadata)
    .bind(is_active)
    .bind(&scope)
    .fetch_one(&s.db)
    .await?;

    let resp = ProviderKeyResponse {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        provider: row.get("provider"),
        api_key: mask_key(row.get::<String, _>("api_key").as_str()),
        base_url: row.get("base_url"),
        metadata: row.get("metadata"),
        is_active: row.get("is_active"),
        scope: row.get("scope"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    };

    Ok((StatusCode::CREATED, Json(json!({
        "success": true,
        "data": resp
    }))))
}

/// DELETE /api/v1/admin/provider-keys/:provider
pub async fn delete_provider_key(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid)
        .map_err(|_| AppError::Unauthorized)?;

    let result = sqlx::query(
        "DELETE FROM provider_keys WHERE tenant_id = $1 AND provider = $2"
    )
    .bind(tenant_id)
    .bind(&provider)
    .execute(&s.db)
    .await?;

    if result.rows_affected() == 0 {
		return Err(AppError::NotFound(format!(
            "No provider key found for '{}'", provider
        )));
    }

    Ok(Json(json!({
        "success": true,
        "message": format!("Provider key '{}' deleted", provider)
    })))
}

/// GET /api/v1/available-providers
pub async fn list_available_providers(
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(
        "SELECT key, name, description, requires_base_url, requires_metadata, icon \
         FROM available_providers \
         ORDER BY name ASC"
    )
    .fetch_all(&s.db)
    .await?;

    let providers: Vec<AvailableProviderResponse> = rows.iter().map(|row| {
        AvailableProviderResponse {
            key: row.get("key"),
            name: row.get("name"),
            description: row.get("description"),
            requires_base_url: row.get("requires_base_url"),
            requires_metadata: row.get("requires_metadata"),
            icon: row.get("icon"),
        }
    }).collect();

    Ok(Json(json!({
        "success": true,
        "data": providers
    })))
}


/// GET /api/v1/provider-keys/:provider/test — test if a provider key is configured
pub async fn test_provider_key(
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let key = sqlx::query_scalar::<_, String>(
        r#"SELECT decrypt_provider_key(api_key_encrypted) FROM provider_keys 
         WHERE provider = $1 AND is_active = true LIMIT 1"#
    )
    .bind(&provider)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("No API key found for provider '{}'", provider)))?;

    let preview = if key.len() > 8 {
        format!("{}...{}", &key[..4], &key[key.len()-4..])
    } else {
        "****".to_string()
    };

    Ok(Json(json!({
        "provider": provider,
        "configured": true,
        "key_preview": preview,
        "message": format!("{} API key is configured", provider)
    })))
}
