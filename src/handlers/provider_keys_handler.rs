//! Provider keys CRUD — tenant-scoped API key storage per provider.
//!
//! Round 5 (T0): keys are **named**. A tenant can hold many keys for the same
//! provider, each with a human `label` ("Palm Bay project", "Test account").
//! Every key carries `is_default`; ONE default per (tenant, provider) is enforced
//! by a partial unique index (migration 082).
//!
//! Endpoints:
//! - GET    /api/v1/admin/provider-keys              (list ALL keys, grouped by provider, masked)
//! - POST   /api/v1/admin/provider-keys              (create/update one key: provider + label)
//! - PUT    /api/v1/admin/provider-keys/:provider    (legacy alias — writes the 'default' label)
//! - DELETE /api/v1/admin/provider-keys/:provider    (delete EVERY key for a provider)
//! - DELETE /api/v1/admin/provider-keys/id/:id       (delete ONE named key)
//! - POST   /api/v1/admin/provider-keys/id/:id/default (make one named key the default)
//! - GET    /api/v1/admin/provider-keys/:provider/test (verify the resolved key)
//! - GET    /api/v1/available-providers               (public list)
//!
//! Any code that needs "the key for provider X" MUST call
//! [`resolve_provider_key`] / [`resolve_provider_key_for_tenant`] — default row
//! first, then the most recently updated active row. One resolver, no ad-hoc SQL.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// Label used when the caller does not name a key.
pub const DEFAULT_LABEL: &str = "default";

#[derive(Debug, Deserialize)]
pub struct UpsertProviderKeyRequest {
    pub provider: String,
    pub api_key: String,
    /// Human name for this key. Defaults to "default" when absent.
    pub label: Option<String>,
    /// Make this key the one resolved for its provider.
    pub is_default: Option<bool>,
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
    pub label: String,
    pub is_default: bool,
    pub api_key: String, // masked in response — never unmasked over the wire
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

fn row_to_response(row: &sqlx::postgres::PgRow) -> ProviderKeyResponse {
    let resolved: String = row
        .try_get::<Option<String>, _>("api_key")
        .unwrap_or(None)
        .unwrap_or_default();
    ProviderKeyResponse {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        provider: row.get("provider"),
        label: row
            .try_get("label")
            .unwrap_or_else(|_| DEFAULT_LABEL.to_string()),
        is_default: row.try_get("is_default").unwrap_or(false),
        api_key: mask_key(&resolved),
        base_url: row.get("base_url"),
        metadata: row.get("metadata"),
        is_active: row.get("is_active"),
        scope: row.get("scope"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Mask an API key showing only first 4 and last 4 characters.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        "****".to_string()
    } else {
        let first4: String = chars[..4].iter().collect();
        let last4: String = chars[chars.len() - 4..].iter().collect();
        format!("{}...{}", first4, last4)
    }
}

/// Shared selection SQL — default row first, then most recently updated active row.
const SELECT_KEYS: &str = "SELECT id, tenant_id, provider, label, is_default, \
        COALESCE(decrypt_provider_key(api_key_encrypted), api_key) as api_key, \
        CASE WHEN base_url_encrypted IS NOT NULL \
            THEN decrypt_provider_key(base_url_encrypted) ELSE base_url END as base_url, \
        metadata, is_active, scope, created_at::text, updated_at::text \
     FROM provider_keys";

/// Resolve "the key for provider X" for the SYSTEM tenant (platform-wide keys),
/// falling back to any tenant that has an active key.
/// Default row wins; otherwise the most recently updated active row.
pub async fn resolve_provider_key(db: &sqlx::PgPool, provider: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT COALESCE(decrypt_provider_key(api_key_encrypted), api_key) FROM provider_keys
           WHERE provider = $1 AND is_active = true
           ORDER BY (tenant_id = '00000000-0000-0000-0000-000000000000'::uuid) DESC,
                    is_default DESC, updated_at DESC
           LIMIT 1"#,
    )
    .bind(provider)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// Resolve "the key for provider X" for one tenant: the tenant's own keys first
/// (default row, then most recent), then the system/global key.
pub async fn resolve_provider_key_for_tenant(
    db: &sqlx::PgPool,
    tenant_id: Uuid,
    provider: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT COALESCE(decrypt_provider_key(api_key_encrypted), api_key) FROM provider_keys
           WHERE provider = $1 AND is_active = true
             AND (tenant_id = $2 OR tenant_id = '00000000-0000-0000-0000-000000000000'::uuid)
           ORDER BY (tenant_id = $2) DESC, is_default DESC, updated_at DESC
           LIMIT 1"#,
    )
    .bind(provider)
    .bind(tenant_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

async fn validate_provider_exists(db: &sqlx::PgPool, provider: &str) -> Result<(), AppError> {
    let exists =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM available_providers WHERE key = $1")
            .bind(provider)
            .fetch_one(db)
            .await?;

    if exists == 0 {
        return Err(AppError::NotFound(format!(
            "Provider '{}' is not supported",
            provider
        )));
    }
    Ok(())
}

async fn fetch_keys(
    db: &sqlx::PgPool,
    tenant_id: Uuid,
) -> Result<Vec<ProviderKeyResponse>, AppError> {
    let sql = format!(
        "{} WHERE tenant_id = $1 \
         ORDER BY provider ASC, is_default DESC, updated_at DESC",
        SELECT_KEYS
    );
    let rows = sqlx::query(&sql).bind(tenant_id).fetch_all(db).await?;
    Ok(rows.iter().map(row_to_response).collect())
}

/// GET /api/v1/admin/provider-keys — every key, plus a provider-grouped map.
pub async fn list_provider_keys(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let keys = fetch_keys(&s.db, tenant_id).await?;

    let mut grouped: serde_json::Map<String, Value> = serde_json::Map::new();
    for k in &keys {
        grouped
            .entry(k.provider.clone())
            .or_insert_with(|| Value::Array(vec![]));
        if let Some(Value::Array(arr)) = grouped.get_mut(&k.provider) {
            arr.push(json!({
                "id": k.id,
                "label": k.label,
                "api_key": k.api_key, // masked
                "is_default": k.is_default,
                "is_active": k.is_active,
                "updated_at": k.updated_at,
            }));
        }
    }

    Ok(Json(json!({
        "success": true,
        "data": keys,
        "grouped": grouped
    })))
}

/// POST /api/v1/admin/provider-keys — create or update ONE named key.
pub async fn upsert_provider_key(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Json(req): Json<UpsertProviderKeyRequest>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    validate_provider_exists(&s.db, &req.provider).await?;

    let metadata = req.metadata.unwrap_or(json!({}));
    let is_active = req.is_active.unwrap_or(true);
    let scope = req.scope.unwrap_or_else(|| "tenant".to_string());
    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or(DEFAULT_LABEL)
        .to_string();

    // An explicitly-default key demotes every other key of that provider first.
    let make_default = req.is_default == Some(true);
    if make_default {
        sqlx::query(
            "UPDATE provider_keys SET is_default = false \
             WHERE tenant_id = $1 AND provider = $2 AND is_default = true",
        )
        .bind(tenant_id)
        .bind(&req.provider)
        .execute(&s.db)
        .await?;
    }

    // First key saved for a provider becomes its default automatically.
    let any_existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM provider_keys WHERE tenant_id = $1 AND provider = $2",
    )
    .bind(tenant_id)
    .bind(&req.provider)
    .fetch_one(&s.db)
    .await?;
    let is_default = make_default || any_existing == 0;

    // Store plaintext api_key in api_key — trigger auto-encrypts to api_key_encrypted.
    let sql = format!(
        "INSERT INTO provider_keys (tenant_id, provider, label, api_key, base_url, metadata, is_active, scope, is_default) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (tenant_id, provider, label) \
         DO UPDATE SET api_key = EXCLUDED.api_key, \
                       base_url = EXCLUDED.base_url, \
                       metadata = EXCLUDED.metadata, \
                       is_active = EXCLUDED.is_active, \
                       scope = EXCLUDED.scope, \
                       is_default = EXCLUDED.is_default, \
                       updated_at = NOW() \
         RETURNING id, tenant_id, provider, label, is_default, \
                   COALESCE(decrypt_provider_key(api_key_encrypted), api_key) as api_key, \
                   CASE WHEN base_url_encrypted IS NOT NULL \
                       THEN decrypt_provider_key(base_url_encrypted) \
                       ELSE base_url END as base_url, \
                   metadata, is_active, scope, created_at::text, updated_at::text"
    );
    let row = sqlx::query(&sql)
        .bind(tenant_id)
        .bind(&req.provider)
        .bind(&label)
        .bind(&req.api_key)
        .bind(&req.base_url)
        .bind(&metadata)
        .bind(is_active)
        .bind(&scope)
        .bind(is_default)
        .fetch_one(&s.db)
        .await?;

    let resp = row_to_response(&row);

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "success": true,
            "data": resp
        })),
    ))
}

/// POST /api/v1/admin/provider-keys/id/:id/default — make one named key the default.
pub async fn set_default_provider_key(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let provider = sqlx::query_scalar::<_, String>(
        "SELECT provider FROM provider_keys WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Provider key not found".into()))?;

    sqlx::query(
        "UPDATE provider_keys SET is_default = false \
         WHERE tenant_id = $1 AND provider = $2 AND is_default = true",
    )
    .bind(tenant_id)
    .bind(&provider)
    .execute(&s.db)
    .await?;

    sqlx::query("UPDATE provider_keys SET is_default = true, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    let keys = fetch_keys(&s.db, tenant_id).await?;
    Ok(Json(json!({
        "success": true,
        "message": format!("'{}' is now the default {} key", provider, provider),
        "data": keys
    })))
}

/// DELETE /api/v1/admin/provider-keys/id/:id — delete ONE named key.
pub async fn delete_provider_key_by_id(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let row = sqlx::query(
        "SELECT provider, is_default FROM provider_keys WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Provider key not found".into()))?;
    let provider: String = row.get("provider");
    let was_default: bool = row.get("is_default");

    sqlx::query("DELETE FROM provider_keys WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    // Never leave a provider without a default: promote the most recent key left.
    if was_default {
        sqlx::query(
            "UPDATE provider_keys SET is_default = true \
             WHERE id = (SELECT id FROM provider_keys \
                         WHERE tenant_id = $1 AND provider = $2 \
                         ORDER BY updated_at DESC LIMIT 1)",
        )
        .bind(tenant_id)
        .bind(&provider)
        .execute(&s.db)
        .await?;
    }

    Ok(Json(json!({
        "success": true,
        "message": format!("Provider key deleted ({})", provider)
    })))
}

/// DELETE /api/v1/admin/provider-keys/:provider — remove EVERY key for a provider.
pub async fn delete_provider_key(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let result = sqlx::query("DELETE FROM provider_keys WHERE tenant_id = $1 AND provider = $2")
        .bind(tenant_id)
        .bind(&provider)
        .execute(&s.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "No provider key found for '{}'",
            provider
        )));
    }

    Ok(Json(json!({
        "success": true,
        "message": format!("All provider keys for '{}' deleted", provider)
    })))
}

/// GET /api/v1/available-providers
pub async fn list_available_providers(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(
        "SELECT key, name, description, requires_base_url, requires_metadata, icon \
         FROM available_providers \
         ORDER BY name ASC",
    )
    .fetch_all(&s.db)
    .await?;

    let providers: Vec<AvailableProviderResponse> = rows
        .iter()
        .map(|row| AvailableProviderResponse {
            key: row.get("key"),
            name: row.get("name"),
            description: row.get("description"),
            requires_base_url: row.get("requires_base_url"),
            requires_metadata: row.get("requires_metadata"),
            icon: row.get("icon"),
        })
        .collect();

    Ok(Json(json!({
        "success": true,
        "data": providers
    })))
}

/// GET /api/v1/admin/provider-keys/:provider/test — check the RESOLVED key for a provider.
pub async fn test_provider_key(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let key = resolve_provider_key_for_tenant(&s.db, tenant_id, &provider)
        .await
        .ok_or_else(|| {
            AppError::NotFound(format!("No API key found for provider '{}'", provider))
        })?;

    let default_label = sqlx::query_scalar::<_, String>(
        "SELECT label FROM provider_keys WHERE provider = $1 AND is_default = true LIMIT 1",
    )
    .bind(&provider)
    .fetch_optional(&s.db)
    .await?
    .unwrap_or_else(|| DEFAULT_LABEL.to_string());

    Ok(Json(json!({
        "provider": provider,
        "configured": true,
        "resolved_label": default_label,
        "key_preview": mask_key(&key), // masked — never the raw credential
        "message": format!("{} API key is configured (default: {})", provider, default_label)
    })))
}
