//! Native per-entity CoreSwift sync (card B111).
//!
//! David's binding principle: Multi-Directory is a DATA COMPANY that presents as a directory. A
//! user/customer, a local business and a supplier are each a COMPLETE record, and the one external
//! integration — CoreSwift, the hub — must receive that record per entity, with an honest per-record
//! answer to "was this pushed?".
//!
//! This module adds the ORCHESTRATION layer over the single canonical push path
//! ([`crate::coreswift::push_lead_to_coreswift`]): it never opens a second CoreSwift client. For every
//! entity kind it builds the contact payload from the entity's own row, pushes it, and records the
//! outcome in `coreswift_sync_state` (migration 130) so a run is RESUMABLE and IDEMPOTENT:
//!
//!   * one entity        — `POST /integrations/coreswift/sync/push`
//!   * every entity of a kind in a directory — `POST /integrations/coreswift/sync/push-all`
//!   * the per-record state (status, contact id, last push, last real error) — `GET …/sync/state`
//!
//! Honesty rule: unconfigured or failing SAYS SO — the state is `not_configured` / `error` with the
//! real message, never a fake success. A `synced` row is skipped on the next push-all unless
//! `force` is set.
//!
//! Scoping follows the fleet IDOR rules: business/supplier via `can_admin_business` (platform
//! operator, owner, claimant or directory operator); a customer via its `directory_id` admin (or a
//! platform operator when it has none). A row the caller may not touch is a 404, never a 403.

use axum::http::HeaderMap;
use axum::{extract::Query, extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::coreswift::{push_lead_to_coreswift, resolve_lead_conn, LeadPayload};
use crate::error::{ApiResult, AppError};
use crate::handlers::tenant_scope::{
    assert_directory_admin, caller_tenant, can_admin_business, claims_from_headers,
    is_platform_operator,
};
use crate::AppState;

/// The three entity kinds the card names. `user` is accepted as an alias of `customer`.
#[derive(Clone, Copy, PartialEq)]
enum EntityKind {
    Business,
    Supplier,
    Customer,
}

impl EntityKind {
    fn parse(kind: &str) -> Result<Self, AppError> {
        match kind.trim().to_ascii_lowercase().as_str() {
            "business" => Ok(Self::Business),
            "supplier" => Ok(Self::Supplier),
            "customer" | "user" => Ok(Self::Customer),
            other => Err(AppError::Validation(format!(
                "Unknown entity kind '{other}' — expected business, supplier or customer."
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Business => "business",
            Self::Supplier => "supplier",
            Self::Customer => "customer",
        }
    }
}

/// The per-record state row, joined to the entity it describes.
#[derive(sqlx::FromRow)]
struct SyncRow {
    id: Uuid,
    label: Option<String>,
    status: Option<String>,
    coreswift_contact_id: Option<Uuid>,
    last_pushed_at: Option<chrono::DateTime<chrono::Utc>>,
    last_error: Option<String>,
    attempts: Option<i32>,
}

/// Per-record state, entity LEFT JOIN sync state. Compile-time literals only (gate rule 5d).
const STATE_BUSINESS: &str = r#"SELECT b.id, b.name AS label, s.status, s.coreswift_contact_id,
        s.last_pushed_at, s.last_error, s.attempts
    FROM businesses b
    LEFT JOIN coreswift_sync_state s ON s.entity_kind = 'business' AND s.entity_id = b.id
    WHERE b.directory_id = $1
    ORDER BY b.name
    LIMIT $2"#;

const STATE_SUPPLIER: &str = r#"SELECT b.id, b.name AS label, s.status, s.coreswift_contact_id,
        s.last_pushed_at, s.last_error, s.attempts
    FROM businesses b
    LEFT JOIN coreswift_sync_state s ON s.entity_kind = 'supplier' AND s.entity_id = b.id
    WHERE b.directory_id = $1
      AND b.business_type IS NOT NULL AND b.business_type <> 'local'
    ORDER BY b.name
    LIMIT $2"#;

const STATE_CUSTOMER: &str = r#"SELECT v.id, COALESCE(NULLIF(v.name, ''), v.email) AS label, s.status,
        s.coreswift_contact_id, s.last_pushed_at, s.last_error, s.attempts
    FROM visitor_accounts v
    LEFT JOIN coreswift_sync_state s ON s.entity_kind = 'customer' AND s.entity_id = v.id
    WHERE v.directory_id = $1
    ORDER BY v.created_at
    LIMIT $2"#;

/// Candidate ids for a push-all run: not yet `synced` unless forced.
const PENDING_BUSINESS: &str = r#"SELECT b.id FROM businesses b
    LEFT JOIN coreswift_sync_state s ON s.entity_kind = 'business' AND s.entity_id = b.id
    WHERE b.directory_id = $1
      AND ($2::boolean OR s.status IS DISTINCT FROM 'synced')
    ORDER BY b.name
    LIMIT $3"#;

const PENDING_SUPPLIER: &str = r#"SELECT b.id FROM businesses b
    LEFT JOIN coreswift_sync_state s ON s.entity_kind = 'supplier' AND s.entity_id = b.id
    WHERE b.directory_id = $1
      AND b.business_type IS NOT NULL AND b.business_type <> 'local'
      AND ($2::boolean OR s.status IS DISTINCT FROM 'synced')
    ORDER BY b.name
    LIMIT $3"#;

const PENDING_CUSTOMER: &str = r#"SELECT v.id FROM visitor_accounts v
    LEFT JOIN coreswift_sync_state s ON s.entity_kind = 'customer' AND s.entity_id = v.id
    WHERE v.directory_id = $1
      AND ($2::boolean OR s.status IS DISTINCT FROM 'synced')
    ORDER BY v.created_at
    LIMIT $3"#;

/// One entity's own row, enough to build a contact.
const LOAD_BUSINESS: &str = r#"SELECT b.id, b.directory_id, b.name, b.email, b.phone, b.city, b.state,
        b.zip, b.address, b.website, b.business_type
    FROM businesses b WHERE b.id = $1"#;

const LOAD_CUSTOMER: &str = r#"SELECT v.id, v.directory_id, v.name, v.email, v.phone FROM visitor_accounts v WHERE v.id = $1"#;

const DIR_LOCATION: &str = "SELECT name, city, state FROM directories WHERE id = $1";

const SET_BUSINESS_CONTACT: &str = "UPDATE businesses SET coreswift_contact_id = $1 WHERE id = $2";
const SET_CUSTOMER_CONTACT: &str =
    "UPDATE visitor_accounts SET coreswift_contact_id = $1 WHERE id = $2";

const UPSERT_STATE: &str = r#"INSERT INTO coreswift_sync_state
        (entity_kind, entity_id, directory_id, status, coreswift_contact_id, attempts,
         last_pushed_at, last_error, updated_at)
    VALUES ($1, $2, $3, $4, $5, 1,
            CASE WHEN $4::text = 'synced' THEN now() ELSE NULL END, $6, now())
    ON CONFLICT (entity_kind, entity_id) DO UPDATE SET
        directory_id = EXCLUDED.directory_id,
        status = EXCLUDED.status,
        coreswift_contact_id = COALESCE(EXCLUDED.coreswift_contact_id,
                                        coreswift_sync_state.coreswift_contact_id),
        attempts = coreswift_sync_state.attempts + 1,
        last_pushed_at = CASE WHEN EXCLUDED.status = 'synced' THEN now()
                              ELSE coreswift_sync_state.last_pushed_at END,
        last_error = EXCLUDED.last_error,
        updated_at = now()"#;

/// What one push attempt produced.
enum PushOutcome {
    Synced(Uuid),
    Error(String),
    NotConfigured(String),
}

impl PushOutcome {
    fn status(&self) -> &'static str {
        match self {
            Self::Synced(_) => "synced",
            Self::Error(_) => "error",
            Self::NotConfigured(_) => "not_configured",
        }
    }
    fn contact_id(&self) -> Option<Uuid> {
        match self {
            Self::Synced(id) => Some(*id),
            _ => None,
        }
    }
    fn message(&self) -> Option<&str> {
        match self {
            Self::Synced(_) => None,
            Self::Error(e) | Self::NotConfigured(e) => Some(e.as_str()),
        }
    }
}

async fn record_state(
    db: &PgPool,
    kind: EntityKind,
    id: Uuid,
    directory_id: Option<Uuid>,
    outcome: &PushOutcome,
) -> Result<(), AppError> {
    sqlx::query(UPSERT_STATE)
        .bind(kind.as_str())
        .bind(id)
        .bind(directory_id)
        .bind(outcome.status())
        .bind(outcome.contact_id())
        .bind(outcome.message())
        .execute(db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error recording sync state: {e}")))?;
    Ok(())
}

/// Push ONE entity through the canonical CoreSwift contact path and record the honest outcome.
/// The caller has already established that `id` exists and is readable.
async fn push_entity(
    db: &PgPool,
    tenant_id: Uuid,
    kind: EntityKind,
    id: Uuid,
) -> Result<PushOutcome, AppError> {
    // Resolve the connection once: it also tells us which hub list this kind belongs in.
    let conn = resolve_lead_conn(db, Some(tenant_id), None)
        .await
        .map_err(AppError::Internal)?;
    // A directory-scoped link still needs the entity's directory, so resolve per entity below.
    let _ = conn;

    let lead = match kind {
        EntityKind::Business | EntityKind::Supplier => {
            let row = sqlx::query_as::<
                _,
                (
                    Uuid,
                    Option<Uuid>,
                    String,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                ),
            >(LOAD_BUSINESS)
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(|e| AppError::Internal(format!("DB error loading business: {e}")))?;
            let Some((
                _id,
                directory_id,
                name,
                email,
                phone,
                city,
                state,
                zip,
                address,
                website,
                business_type,
            )) = row
            else {
                return Err(AppError::NotFound("entity not found".into()));
            };

            if kind == EntityKind::Supplier
                && !business_type
                    .as_deref()
                    .map(|b| !b.eq_ignore_ascii_case("local"))
                    .unwrap_or(false)
            {
                return Err(AppError::Validation(
                    "That business is a local listing, not a supplier.".into(),
                ));
            }

            let conn = resolve_lead_conn(db, Some(tenant_id), directory_id)
                .await
                .map_err(AppError::Internal)?;
            let Some(conn) = conn else {
                let msg = "CoreSwift is not connected for this directory — connect it in the \
                           Integration panel first."
                    .to_string();
                let outcome = PushOutcome::NotConfigured(msg);
                record_state(db, kind, id, directory_id, &outcome).await?;
                return Ok(outcome);
            };

            let list_id = match kind {
                EntityKind::Supplier => conn.suppliers_list_id,
                _ => conn.businesses_list_id,
            };
            let mut fields = serde_json::Map::new();
            fields.insert("md_entity_id".into(), json!(id.to_string()));
            fields.insert("md_entity_kind".into(), json!(kind.as_str()));
            if let Some(d) = directory_id {
                fields.insert("md_directory_id".into(), json!(d.to_string()));
            }
            LeadPayload {
                email: email.clone(),
                phone: phone.clone(),
                name: Some(name.clone()),
                first_name: None,
                last_name: None,
                company: Some(name),
                title: None,
                city,
                state,
                postal_code: zip,
                address_line1: address,
                notes: website,
                list_id,
                tags: vec![
                    format!("directory-{}", kind.as_str()),
                    "source:multidirectory".to_string(),
                ],
                fields,
            }
        }
        EntityKind::Customer => {
            let row = sqlx::query_as::<
                _,
                (Uuid, Option<Uuid>, Option<String>, String, Option<String>),
            >(LOAD_CUSTOMER)
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(|e| AppError::Internal(format!("DB error loading customer: {e}")))?;
            let Some((_id, directory_id, name, email, phone)) = row else {
                return Err(AppError::NotFound("entity not found".into()));
            };

            let conn = resolve_lead_conn(db, Some(tenant_id), directory_id)
                .await
                .map_err(AppError::Internal)?;
            let Some(conn) = conn else {
                let msg = "CoreSwift is not connected for this directory — connect it in the \
                           Integration panel first."
                    .to_string();
                let outcome = PushOutcome::NotConfigured(msg);
                record_state(db, kind, id, directory_id, &outcome).await?;
                return Ok(outcome);
            };

            // A directory gives the customer record its city/state when the account has none.
            let (city, state) = match directory_id {
                Some(did) => {
                    sqlx::query_as::<_, (String, Option<String>, Option<String>)>(DIR_LOCATION)
                        .bind(did)
                        .fetch_optional(db)
                        .await
                        .map_err(|e| {
                            AppError::Internal(format!("DB error loading directory: {e}"))
                        })?
                        .map(|(_n, c, s)| (c, s))
                        .unwrap_or((None, None))
                }
                None => (None, None),
            };

            let mut fields = serde_json::Map::new();
            fields.insert("md_entity_id".into(), json!(id.to_string()));
            fields.insert("md_entity_kind".into(), json!("customer"));
            if let Some(d) = directory_id {
                fields.insert("md_directory_id".into(), json!(d.to_string()));
            }
            LeadPayload {
                email: Some(email.clone()),
                phone: phone.clone(),
                name,
                first_name: None,
                last_name: None,
                company: None,
                title: None,
                city,
                state,
                postal_code: None,
                address_line1: None,
                notes: None,
                list_id: conn.users_list_id,
                tags: vec![
                    "directory-user".to_string(),
                    "source:multidirectory".to_string(),
                ],
                fields,
            }
        }
    };

    let directory_id = match kind {
        EntityKind::Business | EntityKind::Supplier => sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT directory_id FROM businesses WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .flatten(),
        EntityKind::Customer => sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT directory_id FROM visitor_accounts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .flatten(),
    };

    // Push through the ONE canonical path (also the idempotency guard: no identity → no push).
    let pushed = push_lead_to_coreswift(db, Some(tenant_id), directory_id, lead)
        .await
        .map_err(AppError::Internal)?;

    let outcome = match pushed {
        Some(contact_id) => {
            match kind {
                EntityKind::Business | EntityKind::Supplier => {
                    sqlx::query(SET_BUSINESS_CONTACT)
                        .bind(contact_id)
                        .bind(id)
                        .execute(db)
                        .await
                }
                EntityKind::Customer => {
                    sqlx::query(SET_CUSTOMER_CONTACT)
                        .bind(contact_id)
                        .bind(id)
                        .execute(db)
                        .await
                }
            }
            .map_err(|e| AppError::Internal(format!("DB error storing contact id: {e}")))?;
            PushOutcome::Synced(contact_id)
        }
        None => PushOutcome::Error(
            "Nothing was pushed — CoreSwift is not connected for this entity's directory, or the \
             entity has no email, phone or name to identify it."
                .to_string(),
        ),
    };

    record_state(db, kind, id, directory_id, &outcome).await?;
    Ok(outcome)
}

/// Resolve the entity's directory and confirm the caller may administer this kind of entity.
async fn assert_can_sync(
    db: &PgPool,
    claims: &crate::auth::models::Claims,
    kind: EntityKind,
    id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    match kind {
        EntityKind::Business | EntityKind::Supplier => {
            if can_admin_business(db, claims, id).await? {
                let did: Option<Uuid> =
                    sqlx::query_scalar("SELECT directory_id FROM businesses WHERE id = $1")
                        .bind(id)
                        .fetch_optional(db)
                        .await
                        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
                        .flatten();
                Ok(did)
            } else {
                Err(AppError::NotFound("entity not found".into()))
            }
        }
        EntityKind::Customer => {
            let did: Option<Uuid> =
                sqlx::query_scalar("SELECT directory_id FROM visitor_accounts WHERE id = $1")
                    .bind(id)
                    .fetch_optional(db)
                    .await
                    .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
                    .flatten();
            match did {
                Some(d) => {
                    assert_directory_admin(db, claims, d).await?;
                    Ok(Some(d))
                }
                None => {
                    if is_platform_operator(claims) {
                        Ok(None)
                    } else {
                        Err(AppError::NotFound("entity not found".into()))
                    }
                }
            }
        }
    }
}

/// GET /api/v1/integrations/coreswift/sync/kinds
pub async fn sync_kinds() -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "kinds": ["business", "supplier", "customer"],
        "statuses": ["pending", "synced", "error", "not_configured"],
    })))
}

#[derive(Debug, Deserialize)]
pub struct StateQuery {
    pub kind: String,
    pub directory_id: Uuid,
    /// Optional status filter: `synced` / `error` / `not_configured` / `pending`.
    pub status: Option<String>,
    pub limit: Option<i64>,
}

/// GET /api/v1/integrations/coreswift/sync/state?kind=&directory_id=&status=&limit=
pub async fn sync_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StateQuery>,
) -> ApiResult<Json<Value>> {
    let claims = claims_from_headers(&headers, &state.config.jwt_secret)?;
    let kind = EntityKind::parse(&q.kind)?;
    assert_directory_admin(&state.db, &claims, q.directory_id).await?;

    let sql = match kind {
        EntityKind::Business => STATE_BUSINESS,
        EntityKind::Supplier => STATE_SUPPLIER,
        EntityKind::Customer => STATE_CUSTOMER,
    };
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let rows: Vec<SyncRow> = sqlx::query_as(sql)
        .bind(q.directory_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error listing sync state: {e}")))?;

    let want = q.status.as_deref().map(|s| s.trim().to_ascii_lowercase());
    let mut counts = serde_json::Map::new();
    for s in ["pending", "synced", "error", "not_configured"] {
        counts.insert(s.to_string(), json!(0));
    }
    let mut out: Vec<Value> = Vec::new();
    for r in &rows {
        let status = r.status.clone().unwrap_or_else(|| "pending".to_string());
        if let Some(c) = counts.get_mut(&status) {
            *c = json!(c.as_i64().unwrap_or(0) + 1);
        }
        if let Some(w) = &want {
            if &status != w {
                continue;
            }
        }
        out.push(json!({
            "id": r.id,
            "label": r.label,
            "status": status,
            "coreswift_contact_id": r.coreswift_contact_id,
            "last_pushed_at": r.last_pushed_at,
            "last_error": r.last_error,
            "attempts": r.attempts.unwrap_or(0),
        }));
    }

    Ok(Json(json!({
        "kind": kind.as_str(),
        "directory_id": q.directory_id,
        "total": rows.len(),
        "counts": counts,
        "rows": out,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PushOneRequest {
    pub kind: String,
    pub id: Uuid,
    /// Re-push even when the record is already `synced`.
    #[serde(default)]
    pub force: bool,
}

/// POST /api/v1/integrations/coreswift/sync/push
pub async fn sync_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PushOneRequest>,
) -> ApiResult<Json<Value>> {
    let claims = claims_from_headers(&headers, &state.config.jwt_secret)?;
    let kind = EntityKind::parse(&req.kind)?;
    let tenant_id = caller_tenant(&claims)?;
    let directory_id = assert_can_sync(&state.db, &claims, kind, req.id).await?;

    if !req.force {
        let already = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT coreswift_contact_id FROM coreswift_sync_state \
             WHERE entity_kind = $1 AND entity_id = $2 AND status = 'synced'",
        )
        .bind(kind.as_str())
        .bind(req.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
        .flatten();
        if let Some(cid) = already {
            return Ok(Json(json!({
                "success": true,
                "pushed": false,
                "skipped": true,
                "status": "synced",
                "coreswift_contact_id": cid,
                "message": "Already synced — pass force to push again.",
            })));
        }
    }

    let outcome = push_entity(&state.db, tenant_id, kind, req.id).await?;
    Ok(Json(match outcome {
        PushOutcome::Synced(cid) => json!({
            "success": true,
            "pushed": true,
            "skipped": false,
            "status": "synced",
            "coreswift_contact_id": cid,
            "directory_id": directory_id,
        }),
        PushOutcome::Error(msg) => json!({
            "success": false,
            "pushed": false,
            "skipped": false,
            "status": "error",
            "error": msg,
            "directory_id": directory_id,
        }),
        PushOutcome::NotConfigured(msg) => json!({
            "success": false,
            "pushed": false,
            "skipped": false,
            "status": "not_configured",
            "error": msg,
            "directory_id": directory_id,
        }),
    }))
}

#[derive(Debug, Deserialize)]
pub struct PushAllRequest {
    pub kind: String,
    pub directory_id: Uuid,
    /// Re-push records already marked `synced`.
    #[serde(default)]
    pub force: bool,
    /// Cap the batch so one click cannot run unbounded (resume by clicking again).
    pub limit: Option<i64>,
}

/// POST /api/v1/integrations/coreswift/sync/push-all — resumable, idempotent batch.
pub async fn sync_push_all(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PushAllRequest>,
) -> ApiResult<Json<Value>> {
    let claims = claims_from_headers(&headers, &state.config.jwt_secret)?;
    let kind = EntityKind::parse(&req.kind)?;
    let tenant_id = caller_tenant(&claims)?;
    assert_directory_admin(&state.db, &claims, req.directory_id).await?;

    let limit = req.limit.unwrap_or(50).clamp(1, 500);
    let sql = match kind {
        EntityKind::Business => PENDING_BUSINESS,
        EntityKind::Supplier => PENDING_SUPPLIER,
        EntityKind::Customer => PENDING_CUSTOMER,
    };
    let ids: Vec<Uuid> = sqlx::query_scalar(sql)
        .bind(req.directory_id)
        .bind(req.force)
        .bind(limit)
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error listing pending entities: {e}")))?;

    let mut pushed = 0i64;
    let mut failed = 0i64;
    let mut not_configured = 0i64;
    let mut results: Vec<Value> = Vec::new();
    for id in &ids {
        match push_entity(&state.db, tenant_id, kind, *id).await {
            Ok(PushOutcome::Synced(cid)) => {
                pushed += 1;
                results.push(json!({"id": id, "status": "synced", "coreswift_contact_id": cid}));
            }
            Ok(PushOutcome::NotConfigured(msg)) => {
                not_configured += 1;
                results.push(json!({"id": id, "status": "not_configured", "error": msg}));
            }
            Ok(PushOutcome::Error(msg)) => {
                failed += 1;
                results.push(json!({"id": id, "status": "error", "error": msg}));
            }
            Err(e) => {
                failed += 1;
                results.push(json!({"id": id, "status": "error", "error": format!("{e}")}));
            }
        }
    }

    Ok(Json(json!({
        "kind": kind.as_str(),
        "directory_id": req.directory_id,
        "attempted": ids.len(),
        "pushed": pushed,
        "failed": failed,
        "not_configured": not_configured,
        "remaining_hint": "Click again to continue — a synced record is skipped unless force is set.",
        "results": results,
    })))
}
