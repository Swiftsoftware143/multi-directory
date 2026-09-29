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
//!
//! Round 6 (B39): a key is now SCOPED — to the platform, to one NETWORK, or to one DIRECTORY.
//! A directory with no key of its own inherits its network's key, which is what lets David set a
//! Google Places key ONCE for the ZaarHub network and have all ten cities use it. The ONLY
//! resolution path for a directory-scoped read is [`resolve_provider_key_scoped`]
//! (directory -> network -> platform). Unconfigured returns `None`: callers log-and-skip.
//! [`mask_key`] is the only representation of a credential that ever leaves this process.

use axum::{
    extract::{Extension, Path, Query, State},
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
use crate::security::provider_key_crypto as keycrypto;
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
    /// Scope this key to ONE network: every directory inside it inherits the key.
    /// Mutually exclusive with `directory_id` (the database enforces it too).
    pub network_id: Option<Uuid>,
    /// Scope this key to ONE directory: it overrides whatever its network would inherit.
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ProviderKeyResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub provider: String,
    pub label: String,
    pub is_default: bool,
    pub api_key: String, // masked in response — never unmasked over the wire
    /// True when a usable credential is stored for this row (the mask is not the key).
    pub has_key: bool,
    pub base_url: Option<String>,
    pub metadata: Value,
    pub is_active: bool,
    pub scope: String,
    /// "platform" | "network" | "directory" — which place this row belongs to.
    pub scope_kind: String,
    /// Human name of that place ("platform", the network slug, or the directory slug).
    pub scope_name: String,
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct AvailableProviderResponse {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub requires_base_url: bool,
    /// Label for the extra input that requires_base_url demands (NULL = the client falls back to
    /// "Base URL"). DataForSEO needs the HTTP Basic-auth LOGIN there, i.e. the account email, so a
    /// hardcoded "Base URL" label would tell the admin to type the wrong thing.
    pub field_label: Option<String>,
    /// One-line hint shown with that extra input (NULL = no hint).
    pub field_help: Option<String>,
    pub requires_metadata: Value,
    pub icon: Option<String>,
}

/// Build the client-facing row. `api_key` arrives AS STORED (enc:v1 ciphertext for every row
/// written since migration 095) and is decrypted here before masking, so a client never sees
/// ciphertext and the mask always derives from the DECRYPTED credential.
async fn row_to_response(db: &sqlx::PgPool, row: &sqlx::postgres::PgRow) -> ProviderKeyResponse {
    let stored: String = row.try_get::<String, _>("api_key").unwrap_or_default();
    let resolved = keycrypto::decrypt_for_display(db, &stored).await;
    let network_id: Option<Uuid> = row.try_get("network_id").unwrap_or(None);
    let directory_id: Option<Uuid> = row.try_get("directory_id").unwrap_or(None);
    let (scope_kind, scope_name) = scope_of(db, network_id, directory_id).await;
    ProviderKeyResponse {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        provider: row.get("provider"),
        label: row
            .try_get("label")
            .unwrap_or_else(|_| DEFAULT_LABEL.to_string()),
        is_default: row.try_get("is_default").unwrap_or(false),
        api_key: if resolved.is_empty() {
            String::new()
        } else {
            mask_key(&resolved)
        },
        has_key: !resolved.is_empty(),
        base_url: row.try_get("base_url").unwrap_or(None),
        metadata: row.get("metadata"),
        is_active: row.try_get("is_active").unwrap_or(false),
        scope: row.get("scope"),
        scope_kind,
        scope_name,
        network_id,
        directory_id,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Name the place a key belongs to: the platform, a network slug, or a directory slug.
async fn scope_of(
    db: &sqlx::PgPool,
    network_id: Option<Uuid>,
    directory_id: Option<Uuid>,
) -> (String, String) {
    if let Some(dir) = directory_id {
        let slug = sqlx::query_scalar::<_, String>("SELECT slug FROM directories WHERE id = $1")
            .bind(dir)
            .fetch_optional(db)
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| dir.to_string());
        return ("directory".to_string(), slug);
    }
    if let Some(net) = network_id {
        let slug = sqlx::query_scalar::<_, String>("SELECT slug FROM networks WHERE id = $1")
            .bind(net)
            .fetch_optional(db)
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| net.to_string());
        return ("network".to_string(), slug);
    }
    ("platform".to_string(), "platform".to_string())
}

/// Mask an API key showing only first 4 and last 4 characters.
/// David's requirement (B39): "mostly blocked out with asterisks so he knows one is stored" —
/// `AIza••••••••7f2c`. This is the ONLY form of a credential that may leave this process: never
/// the raw value, never in HTML/JSON/log/error. A key too short to preview reveals nothing.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.is_empty() {
        String::new()
    } else if chars.len() <= 8 {
        // Too short to show a prefix AND a suffix without revealing most of the value.
        "*".repeat(chars.len())
    } else {
        let first4: String = chars[..4].iter().collect();
        let last4: String = chars[chars.len() - 4..].iter().collect();
        format!("{}{}{}", first4, "•".repeat(8), last4)
    }
}

/// Shared selection SQL — default row first, then most recently updated active row.
/// `api_key` is selected AS STORED (ciphertext); `row_to_response` decrypts it in Rust with
/// the env-only master key, so no SQL path here depends on a key held in the database.
const SELECT_KEYS: &str = "SELECT id, tenant_id, provider, label, is_default, \
        api_key, base_url, \
        metadata, is_active, scope, network_id, directory_id, \
        created_at::text, updated_at::text \
     FROM provider_keys";

/// Resolve "the key for provider X" for the SYSTEM tenant (platform-wide keys),
/// falling back to any tenant that has an active key.
/// Default row wins; otherwise the most recently updated active row.
pub async fn resolve_provider_key(db: &sqlx::PgPool, provider: &str) -> Option<String> {
    let stored = sqlx::query_scalar::<_, String>(
        r#"SELECT api_key FROM provider_keys
           WHERE provider = $1 AND is_active = true
           ORDER BY (tenant_id = '00000000-0000-0000-0000-000000000000'::uuid) DESC,
                    is_default DESC, updated_at DESC
           LIMIT 1"#,
    )
    .bind(provider)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;

    keycrypto::decrypt_for_use(db, &stored, provider).await
}

/// Resolve "the key for provider X" for one tenant: the tenant's own keys first
/// (default row, then most recent), then the system/global key.
pub async fn resolve_provider_key_for_tenant(
    db: &sqlx::PgPool,
    tenant_id: Uuid,
    provider: &str,
) -> Option<String> {
    let stored = sqlx::query_scalar::<_, String>(
        r#"SELECT api_key FROM provider_keys
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
    .flatten()?;

    keycrypto::decrypt_for_use(db, &stored, provider).await
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
    let mut out = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        // Decryption happens per row in Rust (the master key is not available to SQL).
        out.push(row_to_response(db, row).await);
    }
    Ok(out)
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
                "has_key": k.has_key,
                "is_default": k.is_default,
                "is_active": k.is_active,
                "scope_kind": k.scope_kind,
                "scope_name": k.scope_name,
                "network_id": k.network_id,
                "directory_id": k.directory_id,
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
    // A key is scoped to at most ONE place: a directory, a network, or the platform (B39).
    if req.network_id.is_some() && req.directory_id.is_some() {
        return Err(AppError::Validation(
            "A key is scoped to a network OR a directory, not both".into(),
        ));
    }
    if let Some(net) = req.network_id {
        let found = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM networks WHERE id = $1")
            .bind(net)
            .fetch_one(&s.db)
            .await?;
        if found == 0 {
            return Err(AppError::NotFound("Network not found".into()));
        }
    }
    if let Some(dir) = req.directory_id {
        let found = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = $1")
            .bind(dir)
            .fetch_one(&s.db)
            .await?;
        if found == 0 {
            return Err(AppError::NotFound("Directory not found".into()));
        }
    }
    // The `scope` string is derived from the ids so it can never contradict the columns.
    let scope = if req.directory_id.is_some() {
        "directory".to_string()
    } else if req.network_id.is_some() {
        "network".to_string()
    } else {
        req.scope.unwrap_or_else(|| "tenant".to_string())
    };
    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or(DEFAULT_LABEL)
        .to_string();

    // An explicitly-default key demotes every other key of that provider IN THE SAME SCOPE first
    // (a directory default never disturbs the network default it inherits from).
    let make_default = req.is_default == Some(true);
    if make_default {
        let demote = match (req.network_id, req.directory_id) {
            (_, Some(dir)) => sqlx::query(
                "UPDATE provider_keys SET is_default = false \
                 WHERE directory_id = $1 AND provider = $2 AND is_default = true",
            )
            .bind(dir)
            .bind(&req.provider),
            (Some(net), None) => sqlx::query(
                "UPDATE provider_keys SET is_default = false \
                 WHERE network_id = $1 AND provider = $2 AND is_default = true",
            )
            .bind(net)
            .bind(&req.provider),
            (None, None) => sqlx::query(
                "UPDATE provider_keys SET is_default = false \
                 WHERE tenant_id = $1 AND provider = $2 AND is_default = true \
                   AND network_id IS NULL AND directory_id IS NULL",
            )
            .bind(tenant_id)
            .bind(&req.provider),
        };
        demote.execute(&s.db).await?;
    }

    // First key saved for a provider IN THAT SCOPE becomes its default automatically.
    let any_existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM provider_keys \
         WHERE tenant_id = $1 AND provider = $2 \
           AND directory_id IS NOT DISTINCT FROM $3 \
           AND network_id IS NOT DISTINCT FROM $4",
    )
    .bind(tenant_id)
    .bind(&req.provider)
    .bind(req.directory_id)
    .bind(req.network_id)
    .fetch_one(&s.db)
    .await?;
    let is_default = make_default || any_existing == 0;

    // BYOK credential: encrypt BEFORE it reaches the database. Fail-closed — if the master key
    // is missing this errors, it never stores the value the customer typed (migration 095's
    // CHECK constraint refuses a plaintext write anyway).
    let stored_api_key = keycrypto::encrypt_for_storage(&s.db, &req.api_key).await?;
    let sql = format!(
        "INSERT INTO provider_keys (tenant_id, provider, label, api_key, base_url, metadata, is_active, scope, is_default, network_id, directory_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (tenant_id, provider, label) \
         DO UPDATE SET api_key = EXCLUDED.api_key, \
                       base_url = EXCLUDED.base_url, \
                       metadata = EXCLUDED.metadata, \
                       is_active = EXCLUDED.is_active, \
                       scope = EXCLUDED.scope, \
                       is_default = EXCLUDED.is_default, \
                       network_id = EXCLUDED.network_id, \
                       directory_id = EXCLUDED.directory_id, \
                       updated_at = NOW() \
         RETURNING id, tenant_id, provider, label, is_default, \
                   api_key, base_url, \
                   metadata, is_active, scope, network_id, directory_id, \
                   created_at::text, updated_at::text"
    );
    let row = sqlx::query(&sql)
        .bind(tenant_id)
        .bind(&req.provider)
        .bind(&label)
        .bind(&stored_api_key)
        .bind(&req.base_url)
        .bind(&metadata)
        .bind(is_active)
        .bind(&scope)
        .bind(is_default)
        .bind(req.network_id)
        .bind(req.directory_id)
        .fetch_one(&s.db)
        .await?;

    let resp = row_to_response(&s.db, &row).await;

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
        "SELECT key, name, description, requires_base_url, field_label, field_help, requires_metadata, icon \
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
            field_label: row.get("field_label"),
            field_help: row.get("field_help"),
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

// ── B39: scope resolution (directory -> network -> platform) ─────────────────────────────────

/// Where a resolved credential comes from — the UI shows this so the admin can tell an OWN key
/// from one INHERITED from the network (David's requirement 1c).
#[derive(Debug, Clone, Serialize)]
pub struct KeyOrigin {
    /// "directory" | "network" | "platform"
    pub source: String,
    /// Slug of the directory / network that supplies it, or "platform".
    pub source_name: String,
    pub label: String,
    pub key_id: Uuid,
    /// The masked preview. The raw value NEVER leaves this function.
    pub masked: String,
}

/// The ONE lookup behind every scoped read. Priority: the directory's own key, else its
/// network's key, else a platform key. A standalone directory (network_id IS NULL) never
/// reaches the network layer, so it keeps its own key. Returns `None` when nothing is set.
async fn lookup_scoped(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    provider: &str,
) -> Option<(String, KeyOrigin)> {
    let row = sqlx::query(
        r#"SELECT pk.id, pk.api_key, pk.label,
                  CASE WHEN pk.directory_id = $2 THEN 'directory'
                       WHEN pk.network_id IS NOT NULL THEN 'network'
                       ELSE 'platform' END AS scope_kind,
                  COALESCE(d.slug, n.slug, 'platform') AS scope_name
           FROM provider_keys pk
           LEFT JOIN directories d ON d.id = pk.directory_id
           LEFT JOIN networks n ON n.id = pk.network_id
           WHERE pk.provider = $1
             AND pk.is_active = true
             AND (pk.directory_id = $2
                  OR pk.network_id = (SELECT network_id FROM directories WHERE id = $2)
                  OR (pk.directory_id IS NULL AND pk.network_id IS NULL))
           ORDER BY CASE WHEN pk.directory_id = $2 THEN 0
                         WHEN pk.network_id IS NOT NULL THEN 1
                         ELSE 2 END,
                    pk.is_default DESC, pk.updated_at DESC
           LIMIT 1"#,
    )
    .bind(provider)
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;

    let key_id: Uuid = row.try_get("id").ok()?;
    let stored: String = row.try_get("api_key").ok()?;
    let label: String = row
        .try_get("label")
        .unwrap_or_else(|_| DEFAULT_LABEL.to_string());
    let source: String = row.try_get("scope_kind").ok()?;
    let source_name: String = row.try_get("scope_name").ok()?;

    let plain = keycrypto::decrypt_for_use(db, &stored, provider).await?;
    let masked = mask_key(&plain);
    Some((
        plain,
        KeyOrigin {
            source,
            source_name,
            label,
            key_id,
            masked,
        },
    ))
}

/// Resolve the credential a directory MUST use: directory -> its network -> platform.
/// `None` means genuinely UNCONFIGURED: the caller logs and skips the capability. It never
/// panics and never reports success it cannot prove.
pub async fn resolve_provider_key_scoped(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    provider: &str,
) -> Option<String> {
    match lookup_scoped(db, directory_id, provider).await {
        Some((key, _)) => Some(key),
        None => {
            tracing::warn!(
                "provider '{}' is unconfigured for directory {} (no own key, no network key, \
                 no platform key) — capability skipped, nothing was faked",
                provider,
                directory_id
            );
            None
        }
    }
}

/// Resolution for the ADMIN SCREEN: where would this directory's key come from, and what does
/// the masked preview look like. Same single path as the runtime resolver.
pub async fn provider_key_origin_for_directory(
    db: &sqlx::PgPool,
    directory_id: Uuid,
    provider: &str,
) -> Option<KeyOrigin> {
    lookup_scoped(db, directory_id, provider)
        .await
        .map(|(_, origin)| origin)
}

#[derive(Debug, Deserialize)]
pub struct EffectiveKeysQuery {
    pub directory_id: Uuid,
}

/// GET /api/v1/admin/provider-keys/effective?directory_id=… — for EVERY provider, whether the
/// directory has one in force, where it comes from (own / inherited from the network / platform)
/// and its masked preview. The full value is never in this response.
pub async fn list_effective_keys(
    Extension(_claims): Extension<Claims>,
    State(s): State<AppState>,
    Query(q): Query<EffectiveKeysQuery>,
) -> ApiResult<impl IntoResponse> {
    let dir = sqlx::query(
        r#"SELECT d.id, d.slug, d.name, d.network_id,
                  n.slug AS network_slug, n.name AS network_name, n.root_domain
           FROM directories d
           LEFT JOIN networks n ON n.id = d.network_id
           WHERE d.id = $1"#,
    )
    .bind(q.directory_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let providers = sqlx::query("SELECT key, name FROM available_providers ORDER BY name ASC")
        .fetch_all(&s.db)
        .await?;

    let mut out: Vec<Value> = Vec::with_capacity(providers.len());
    for p in providers.iter() {
        let provider: String = p.get("key");
        let name: String = p.get("name");
        let origin = provider_key_origin_for_directory(&s.db, q.directory_id, &provider).await;
        out.push(json!({
            "provider": provider,
            "name": name,
            "has_key": origin.is_some(),
            "source": origin.as_ref().map(|o| o.source.clone()),
            "source_name": origin.as_ref().map(|o| o.source_name.clone()),
            "source_label": origin.as_ref().map(|o| o.label.clone()),
            "masked": origin.as_ref().map(|o| o.masked.clone()),
            "key_id": origin.as_ref().map(|o| o.key_id),
        }));
    }

    let directory_id: Uuid = dir.get("id");
    let directory_slug: String = dir.get("slug");
    let directory_name: String = dir.get("name");
    let network_slug: Option<String> = dir.try_get("network_slug").unwrap_or(None);
    let network_name: Option<String> = dir.try_get("network_name").unwrap_or(None);
    let root_domain: Option<String> = dir.try_get("root_domain").unwrap_or(None);

    Ok(Json(json!({
        "success": true,
        "directory": { "id": directory_id, "slug": directory_slug, "name": directory_name },
        "network": network_slug.map(|slug| json!({
            "slug": slug,
            "name": network_name,
            "root_domain": root_domain,
        })),
        "data": out
    })))
}

#[derive(Debug, Deserialize)]
pub struct KeyActiveRequest {
    pub is_active: bool,
}

/// POST /api/v1/admin/provider-keys/id/:id/active — deactivate a key (the value stays, the
/// resolver stops choosing it) or bring it back. David's requirement 1b.
pub async fn set_provider_key_active(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<KeyActiveRequest>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let affected = sqlx::query(
        "UPDATE provider_keys SET is_active = $1, updated_at = NOW() \
         WHERE id = $2 AND tenant_id = $3",
    )
    .bind(req.is_active)
    .bind(id)
    .bind(tenant_id)
    .execute(&s.db)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound("Provider key not found".into()));
    }

    let keys = fetch_keys(&s.db, tenant_id).await?;
    Ok(Json(json!({
        "success": true,
        "message": if req.is_active { "Key activated" } else { "Key deactivated (value kept)" },
        "data": keys
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderKeyRequest {
    /// Paste a NEW value to rotate the key — the old one is never required or returned.
    pub api_key: Option<String>,
    pub label: Option<String>,
    pub base_url: Option<String>,
    pub metadata: Option<Value>,
    pub is_active: Option<bool>,
    /// "platform" | "network" | "directory" — moves the key to another scope.
    pub scope: Option<String>,
    pub network_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
}

/// PUT /api/v1/admin/provider-keys/id/:id — change a stored key without knowing the old value:
/// paste a new one (stored encrypted, same enc:v1 path), rename it, move it to another scope,
/// or toggle it. The response carries only the new mask.
pub async fn update_provider_key_by_id(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProviderKeyRequest>,
) -> ApiResult<impl IntoResponse> {
    let tenant_id = Uuid::parse_str(&claims.tid).map_err(|_| AppError::Unauthorized)?;

    let row = sqlx::query(
        r#"SELECT id, api_key, label, base_url, metadata, is_active, is_default,
                  network_id, directory_id
           FROM provider_keys WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Provider key not found".into()))?;

    let label: String = row
        .try_get("label")
        .unwrap_or_else(|_| DEFAULT_LABEL.to_string());
    let new_label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or(&label)
        .to_string();
    let base_url: Option<String> = row.try_get("base_url").unwrap_or(None);
    let new_base_url = req.base_url.or(base_url);
    let metadata: Value = row.try_get("metadata").unwrap_or_else(|_| json!({}));
    let new_metadata = req.metadata.unwrap_or(metadata);
    let is_active: bool = row.try_get("is_active").unwrap_or(true);
    let new_active = req.is_active.unwrap_or(is_active);
    let current_net: Option<Uuid> = row.try_get("network_id").unwrap_or(None);
    let current_dir: Option<Uuid> = row.try_get("directory_id").unwrap_or(None);

    let (new_net, new_dir) = match req.scope.as_deref() {
        Some("platform") | Some("tenant") => (None, None),
        Some("network") => (req.network_id.or(current_net), None),
        Some("directory") => (None, req.directory_id.or(current_dir)),
        Some(other) => {
            return Err(AppError::Validation(format!(
                "Unknown scope '{}' — use platform, network or directory",
                other
            )))
        }
        None => (
            req.network_id.or(current_net),
            req.directory_id.or(current_dir),
        ),
    };
    if new_net.is_some() && new_dir.is_some() {
        return Err(AppError::Validation(
            "A key is scoped to a network OR a directory, not both".into(),
        ));
    }
    if let Some(net) = new_net {
        let found = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM networks WHERE id = $1")
            .bind(net)
            .fetch_one(&s.db)
            .await?;
        if found == 0 {
            return Err(AppError::NotFound("Network not found".into()));
        }
    }
    if let Some(dir) = new_dir {
        let found = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = $1")
            .bind(dir)
            .fetch_one(&s.db)
            .await?;
        if found == 0 {
            return Err(AppError::NotFound("Directory not found".into()));
        }
    }
    let scope_moved = new_net != current_net || new_dir != current_dir;
    let scope = if new_dir.is_some() {
        "directory"
    } else if new_net.is_some() {
        "network"
    } else {
        "tenant"
    };
    // Moving a key to another scope drops its default flag: the partial unique index allows only
    // one default per scope and the resolver does not need the flag to find a lone key.
    let new_default = if scope_moved {
        false
    } else {
        row.try_get("is_default").unwrap_or(false)
    };

    // A pasted value is rotated through the SAME encrypted-at-rest path; an omitted value keeps
    // whatever is stored (so rotating never requires knowing the old key).
    let new_api_key: Option<String> = match req.api_key.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => Some(keycrypto::encrypt_for_storage(&s.db, v).await?),
        _ => None,
    };
    let stored_api_key: String = row.try_get("api_key").unwrap_or_default();
    let final_api_key = new_api_key.unwrap_or(stored_api_key);

    let updated = sqlx::query(
        r#"UPDATE provider_keys
           SET api_key = $1, label = $2, base_url = $3, metadata = $4, is_active = $5,
               scope = $6, network_id = $7, directory_id = $8, is_default = $9, updated_at = NOW()
           WHERE id = $10 AND tenant_id = $11
           RETURNING id, tenant_id, provider, label, is_default, api_key, base_url,
                     metadata, is_active, scope, network_id, directory_id,
                     created_at::text, updated_at::text"#,
    )
    .bind(&final_api_key)
    .bind(&new_label)
    .bind(&new_base_url)
    .bind(&new_metadata)
    .bind(new_active)
    .bind(scope)
    .bind(new_net)
    .bind(new_dir)
    .bind(new_default)
    .bind(id)
    .bind(tenant_id)
    .fetch_one(&s.db)
    .await?;

    let resp = row_to_response(&s.db, &updated).await;
    Ok(Json(json!({ "success": true, "data": resp })))
}

#[cfg(test)]
mod tests {
    use super::mask_key;

    #[test]
    fn mask_shows_only_first_and_last_four() {
        let m = mask_key("AIzaSyD-EXAMPLEKEY-9f7f2c");
        assert!(m.starts_with("AIza"), "prefix: {}", m);
        assert!(m.ends_with("7f2c"), "suffix: {}", m);
        assert!(!m.contains("EXAMPLEKEY"), "body leaked: {}", m);
        assert_eq!(m.chars().filter(|c| *c == '•').count(), 8);
    }

    #[test]
    fn short_keys_reveal_nothing() {
        assert_eq!(mask_key("abcdefgh"), "********");
        assert_eq!(mask_key("abc"), "***");
        assert_eq!(mask_key(""), "");
    }
}
