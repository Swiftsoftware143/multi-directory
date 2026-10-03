//! Complete per-entity export (B112).
//!
//! David's binding principle: Multi-Directory is a DATA COMPANY that presents as a
//! directory. Every entity — a business, a supplier (a business with a non-local
//! `business_type`) and a customer/user (`visitor_accounts`) — must be downloadable as a
//! COMPLETE record: the entity's own row plus every record that hangs off it, gathered
//! together rather than scattered across the schema.
//!
//! Two surfaces, both admin-authenticated, both operable from the admin panel with no SQL
//! and no script:
//!   * `GET /export/entity/:kind/:id?format=json|csv` — ONE entity, complete.
//!   * `GET /export/bulk/:kind?directory_id=…&format=csv|json` — every entity of a kind in
//!     one directory, in bulk.
//!
//! "Complete" is not hand-maintained: the child table list below is the exact set of
//! single-column foreign keys the live catalog shows pointing at each root. Each query is a
//! COMPLETE compile-time literal (the `child_q!` macro expands `concat!` from literals), so
//! no statement text is built at run time and no row is left behind by a runtime join table.
//!
//! Scoping follows the fleet IDOR rules: business/supplier via `can_admin_business`
//! (platform operator, owner, claimant or directory operator); a customer account via its
//! `directory_id` (or platform operator when it has none). A row the caller may not see is a
//! 404, never a 403 that would confirm the id exists.

use axum::http::HeaderMap;
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::Response,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::handlers::tenant_scope::{
    assert_directory_admin, can_admin_business, claims_from_headers, is_platform_operator,
};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct EntityExportQuery {
    /// `json` (default) or `csv`.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkExportQuery {
    pub directory_id: Option<Uuid>,
    /// `csv` (default) or `json`.
    pub format: Option<String>,
}

/// One compile-time statement per (child table, fk column) pair. `concat!` needs literals, so
/// the macro is invoked at each call site with a literal table and column — never a runtime
/// `format!` (gate rule 5d).
macro_rules! child_q {
    ($t:literal, $c:literal) => {
        concat!(
            "SELECT row_to_json(t)::text AS j FROM ",
            $t,
            " t WHERE t.",
            $c,
            " = $1"
        )
    };
}

// Root rows, as a single JSON text column (sqlx never needs the column types).
const ENTITY_BUSINESS: &str = "SELECT row_to_json(t)::text AS j FROM businesses t WHERE t.id = $1";
const ENTITY_CUSTOMER: &str =
    "SELECT row_to_json(t)::text AS j FROM visitor_accounts t WHERE t.id = $1";
const BUSINESS_DIRECTORY: &str = "SELECT directory_id FROM businesses WHERE id = $1";
const BUSINESS_TYPE: &str = "SELECT business_type FROM businesses WHERE id = $1";
const CUSTOMER_DIRECTORY: &str = "SELECT directory_id FROM visitor_accounts WHERE id = $1";

// Bulk: every root row of a kind in one directory.
const BULK_BUSINESS: &str =
    "SELECT row_to_json(t)::text AS j FROM businesses t WHERE t.directory_id = $1 ORDER BY t.name";
const BULK_SUPPLIER: &str = "SELECT row_to_json(t)::text AS j FROM businesses t \
     WHERE t.directory_id = $1 AND (t.business_type IS NOT NULL AND t.business_type <> 'local') \
     ORDER BY t.name";
const BULK_CUSTOMER: &str = "SELECT row_to_json(t)::text AS j FROM visitor_accounts t \
     WHERE t.directory_id = $1 ORDER BY t.created_at";

/// Every single-column FK in the live catalog pointing at `public.businesses`, as
/// `(label, statement)`. `label` is `table.column` when one table points here through more
/// than one column, otherwise just `table`.
const BUSINESS_CHILDREN: &[(&str, &str)] = &[
    (
        "b2b_notifications",
        child_q!("b2b_notifications", "business_id"),
    ),
    (
        "b2b_orders.buyer_business_id",
        child_q!("b2b_orders", "buyer_business_id"),
    ),
    (
        "b2b_orders.supplier_business_id",
        child_q!("b2b_orders", "supplier_business_id"),
    ),
    (
        "business_articles",
        child_q!("business_articles", "business_id"),
    ),
    (
        "business_categories",
        child_q!("business_categories", "business_id"),
    ),
    (
        "business_listings",
        child_q!("business_listings", "business_id"),
    ),
    (
        "business_messages.business_id",
        child_q!("business_messages", "business_id"),
    ),
    (
        "business_messages.sender_business_id",
        child_q!("business_messages", "sender_business_id"),
    ),
    (
        "business_messages.to_business_id",
        child_q!("business_messages", "to_business_id"),
    ),
    ("business_meta", child_q!("business_meta", "business_id")),
    (
        "business_services",
        child_q!("business_services", "business_id"),
    ),
    (
        "business_transfer_events",
        child_q!("business_transfer_events", "business_id"),
    ),
    (
        "business_transfer_fees",
        child_q!("business_transfer_fees", "business_id"),
    ),
    (
        "business_transfers",
        child_q!("business_transfers", "business_id"),
    ),
    (
        "business_verifications",
        child_q!("business_verifications", "business_id"),
    ),
    (
        "buying_group_deals",
        child_q!("buying_group_deals", "supplier_business_id"),
    ),
    (
        "buying_group_members",
        child_q!("buying_group_members", "business_id"),
    ),
    (
        "buying_groups",
        child_q!("buying_groups", "founder_business_id"),
    ),
    (
        "category_requests",
        child_q!("category_requests", "business_id"),
    ),
    (
        "checkout_sessions",
        child_q!("checkout_sessions", "business_id"),
    ),
    (
        "claimed_businesses.business_id",
        child_q!("claimed_businesses", "business_id"),
    ),
    (
        "community_events",
        child_q!("community_events", "business_id"),
    ),
    (
        "data_enrichment_logs",
        child_q!("data_enrichment_logs", "business_id"),
    ),
    (
        "deal_redemptions",
        child_q!("deal_redemptions", "business_id"),
    ),
    (
        "grandfathered_pricing",
        child_q!("grandfathered_pricing", "business_id"),
    ),
    (
        "group_deal_commitments",
        child_q!("group_deal_commitments", "business_id"),
    ),
    (
        "lead_share_transactions.from_business_id",
        child_q!("lead_share_transactions", "from_business_id"),
    ),
    (
        "lead_share_transactions.to_business_id",
        child_q!("lead_share_transactions", "to_business_id"),
    ),
    ("loyalty_scans", child_q!("loyalty_scans", "business_id")),
    ("pay_per_call", child_q!("pay_per_call", "business_id")),
    (
        "plan_slot_bookings",
        child_q!("plan_slot_bookings", "business_id"),
    ),
    ("public_pages", child_q!("public_pages", "business_id")),
    ("reviews", child_q!("reviews", "business_id")),
    ("rfq_bids", child_q!("rfq_bids", "bidder_business_id")),
    (
        "rfq_messages",
        child_q!("rfq_messages", "sender_business_id"),
    ),
    ("rfqs.awarded_to", child_q!("rfqs", "awarded_to")),
    (
        "rfqs.poster_business_id",
        child_q!("rfqs", "poster_business_id"),
    ),
    (
        "service_bookings",
        child_q!("service_bookings", "business_id"),
    ),
    (
        "settlement_invoices",
        child_q!("settlement_invoices", "business_id"),
    ),
    (
        "settlement_payouts",
        child_q!("settlement_payouts", "business_id"),
    ),
    (
        "shared_leads.claimed_by",
        child_q!("shared_leads", "claimed_by"),
    ),
    (
        "shared_leads.poster_business_id",
        child_q!("shared_leads", "poster_business_id"),
    ),
    (
        "sponsored_listings",
        child_q!("sponsored_listings", "business_id"),
    ),
    ("sponsors", child_q!("sponsors", "business_id")),
    (
        "supplier_products",
        child_q!("supplier_products", "business_id"),
    ),
    ("visitor_events", child_q!("visitor_events", "business_id")),
    (
        "visitor_favorites.business_id",
        child_q!("visitor_favorites", "business_id"),
    ),
];

/// Every single-column FK pointing at `public.visitor_accounts`.
const CUSTOMER_CHILDREN: &[(&str, &str)] = &[
    (
        "account_links",
        child_q!("account_links", "visitor_account_id"),
    ),
    (
        "claimed_businesses.visitor_account_id",
        child_q!("claimed_businesses", "visitor_account_id"),
    ),
    (
        "deal_redemptions",
        child_q!("deal_redemptions", "visitor_id"),
    ),
    ("event_rsvps", child_q!("event_rsvps", "visitor_account_id")),
    (
        "loyalty_members",
        child_q!("loyalty_members", "visitor_account_id"),
    ),
    ("poll_votes", child_q!("poll_votes", "visitor_account_id")),
    (
        "service_bookings",
        child_q!("service_bookings", "visitor_account_id"),
    ),
    (
        "survey_responses",
        child_q!("survey_responses", "visitor_account_id"),
    ),
    (
        "visitor_favorites.visitor_account_id",
        child_q!("visitor_favorites", "visitor_account_id"),
    ),
];

/// The three entities the card names — `user/customer` is the `customer` (visitor account).
fn kind_root(kind: &str) -> Result<RootKind, AppError> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "business" => Ok(RootKind::Business {
            supplier_only: false,
        }),
        "supplier" => Ok(RootKind::Business {
            supplier_only: true,
        }),
        "customer" | "user" => Ok(RootKind::Customer),
        other => Err(AppError::Validation(format!(
            "Unknown entity kind '{other}' — expected business, supplier or customer."
        ))),
    }
}

#[derive(Clone, Copy)]
enum RootKind {
    Business { supplier_only: bool },
    Customer,
}

async fn fetch_json_rows(db: &PgPool, sql: &str, id: Uuid) -> Result<Vec<Value>, AppError> {
    let rows = sqlx::query_scalar::<_, String>(sql)
        .bind(id)
        .fetch_all(db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error exporting entity: {e}")))?;
    rows.into_iter()
        .map(|s| {
            serde_json::from_str::<Value>(&s)
                .map_err(|e| AppError::Internal(format!("DB JSON encode error: {e}")))
        })
        .collect()
}

/// Confirm the caller may read this entity; 404 otherwise (never confirm existence).
async fn assert_can_read(
    db: &PgPool,
    claims: &crate::auth::models::Claims,
    root: RootKind,
    id: Uuid,
) -> Result<(), AppError> {
    match root {
        RootKind::Business { .. } => {
            if can_admin_business(db, claims, id).await? {
                Ok(())
            } else {
                Err(AppError::NotFound("business not found".into()))
            }
        }
        RootKind::Customer => {
            let directory_id: Option<Uuid> = sqlx::query_scalar(CUSTOMER_DIRECTORY)
                .bind(id)
                .fetch_optional(db)
                .await
                .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
                .flatten();
            match directory_id {
                Some(did) => assert_directory_admin(db, claims, did).await,
                None => {
                    if is_platform_operator(claims) {
                        Ok(())
                    } else {
                        Err(AppError::NotFound("customer not found".into()))
                    }
                }
            }
        }
    }
}

fn json_response(body: Value, filename: &str) -> ApiResult<Response> {
    let text = serde_json::to_string_pretty(&body)
        .map_err(|e| AppError::Internal(format!("JSON encode error: {e}")))?;
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename={filename}.json"),
        )
        .body(axum::body::Body::from(text))
        .map_err(|e| AppError::Internal(format!("Response build error: {e}")))
}

/// One CSV scalar: scalars verbatim, nested values as compact JSON, NULL as empty.
fn csv_field(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Ordered union of the keys across a table's rows (so nothing is dropped when rows differ).
fn union_keys(rows: &[Value]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for k in obj.keys() {
                if !keys.iter().any(|x| x == k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    keys
}

/// Build a multi-section CSV: a `# section` banner, a header row, then one row per record.
/// A complete record is nested, so each related table is its own section rather than a
/// pretend-flat join.
fn csv_response(sections: &[(String, Vec<Value>)], filename: &str) -> ApiResult<Response> {
    // Sections have different widths (the entity row is wide, a blank separator is one field),
    // so the writer must allow records of differing field counts.
    let mut w = csv::WriterBuilder::new()
        .flexible(true)
        .from_writer(Vec::new());
    for (label, rows) in sections {
        w.write_record([format!("# {label}")])
            .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
        if rows.is_empty() {
            w.write_record(["(no rows)"])
                .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
            w.write_record([""])
                .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
            continue;
        }
        let keys = union_keys(rows);
        w.write_record(&keys)
            .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
        for row in rows {
            let rec: Vec<String> = keys
                .iter()
                .map(|k| row.get(k).map(csv_field).unwrap_or_default())
                .collect();
            w.write_record(&rec)
                .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
        }
        w.write_record([""])
            .map_err(|e| AppError::Internal(format!("CSV write error: {e}")))?;
    }
    let data = String::from_utf8(
        w.into_inner()
            .map_err(|e| AppError::Internal(format!("CSV flush error: {e}")))?,
    )
    .map_err(|e| AppError::Internal(format!("CSV encoding error: {e}")))?;

    Response::builder()
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename={filename}.csv"),
        )
        .body(axum::body::Body::from(data))
        .map_err(|e| AppError::Internal(format!("CSV response build error: {e}")))
}

/// GET /api/v1/export/entity/:kind/:id?format=json|csv
pub async fn export_entity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, Uuid)>,
    Query(q): Query<EntityExportQuery>,
) -> ApiResult<Response> {
    let claims = claims_from_headers(&headers, &state.config.jwt_secret)?;
    let root = kind_root(&kind)?;
    assert_can_read(&state.db, &claims, root, id).await?;

    let (entity_sql, children): (&str, &[(&str, &str)]) = match root {
        RootKind::Business { .. } => (ENTITY_BUSINESS, BUSINESS_CHILDREN),
        RootKind::Customer => (ENTITY_CUSTOMER, CUSTOMER_CHILDREN),
    };

    let entity_text: Option<String> = sqlx::query_scalar(entity_sql)
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Internal(format!("DB error exporting entity: {e}")))?;
    let entity_text = entity_text.ok_or_else(|| AppError::NotFound("entity not found".into()))?;
    let entity: Value = serde_json::from_str(&entity_text)
        .map_err(|e| AppError::Internal(format!("DB JSON encode error: {e}")))?;

    // A supplier must actually be a supplier (non-local business_type).
    if let RootKind::Business {
        supplier_only: true,
    } = root
    {
        let bt: Option<String> = sqlx::query_scalar(BUSINESS_TYPE)
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Internal(format!("DB error: {e}")))?
            .flatten();
        let is_supplier = bt
            .as_deref()
            .map(|b| !b.eq_ignore_ascii_case("local"))
            .unwrap_or(false);
        if !is_supplier {
            return Err(AppError::Validation(
                "That business is a local listing, not a supplier.".into(),
            ));
        }
    }

    let mut section_labels: Vec<String> = vec!["entity".to_string()];
    let mut related = Map::new();
    let mut counts = Map::new();
    let mut csv_sections: Vec<(String, Vec<Value>)> =
        vec![("entity".to_string(), vec![entity.clone()])];

    for (label, sql) in children {
        let rows = fetch_json_rows(&state.db, sql, id).await?;
        counts.insert(label.to_string(), json!(rows.len()));
        section_labels.push((*label).to_string());
        csv_sections.push((format!("table: {label}"), rows.clone()));
        related.insert(label.to_string(), Value::Array(rows));
    }

    if q.format.as_deref() == Some("csv") {
        let name = format!("{}_entity_{}", kind.trim().to_ascii_lowercase(), id);
        return csv_response(&csv_sections, &name);
    }

    Ok(json_response(
        json!({
            "kind": kind.trim().to_ascii_lowercase(),
            "id": id,
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "entity": entity,
            "related": Value::Object(related),
            "counts": counts,
            "sections": section_labels,
        }),
        &format!("{}_entity_{}", kind.trim().to_ascii_lowercase(), id),
    )?)
}

/// GET /api/v1/export/bulk/:kind?directory_id=…&format=csv|json
pub async fn export_bulk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    Query(q): Query<BulkExportQuery>,
) -> ApiResult<Response> {
    let claims = claims_from_headers(&headers, &state.config.jwt_secret)?;
    let root = kind_root(&kind)?;
    let directory_id = q.directory_id.ok_or_else(|| {
        AppError::Validation("directory_id is required for a bulk export.".into())
    })?;
    assert_directory_admin(&state.db, &claims, directory_id).await?;

    let sql = match root {
        RootKind::Business {
            supplier_only: false,
        } => BULK_BUSINESS,
        RootKind::Business {
            supplier_only: true,
        } => BULK_SUPPLIER,
        RootKind::Customer => BULK_CUSTOMER,
    };
    let rows = fetch_json_rows(&state.db, sql, directory_id).await?;
    let k = kind.trim().to_ascii_lowercase();

    if q.format.as_deref() == Some("json") {
        return json_response(
            json!({
                "kind": k,
                "directory_id": directory_id,
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "count": rows.len(),
                "rows": rows,
            }),
            &format!("bulk_{k}_{directory_id}"),
        );
    }
    csv_response(
        &[(format!("{k} (directory {directory_id})"), rows)],
        &format!("bulk_{k}_{directory_id}"),
    )
}

/// Small helper used by the admin panel to know what it can ask for.
pub async fn export_entity_kinds(State(_state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "kinds": ["business", "supplier", "customer"],
        "formats": ["json", "csv"],
    })))
}
