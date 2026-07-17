use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
    Extension,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use crate::AppState;
use crate::error::{AppError, ApiResult};
use crate::auth::models::Claims;

// ── Transfer Request ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub new_owner_email: String,
    pub include_connected: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TransferredItem {
    pub id: String,
    pub name: String,
    pub kind: String,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
pub struct UserBasic {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub is_active: bool,
}

async fn find_user_by_email(db: &PgPool, email: &str) -> Result<UserBasic, AppError> {
    sqlx::query_as::<_, UserBasic>(
        "SELECT id, tenant_id, email, name, role, is_active FROM users WHERE email = $1 AND is_active = true"
    )
    .bind(email)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("User '{}' not found or inactive", email)))
}

fn map_row(row: sqlx::postgres::PgRow) -> serde_json::Value {
    use sqlx::Row;
    use sqlx::Column;
    let cols = row.columns();
    let mut map = serde_json::Map::new();
    for i in 0..cols.len() {
        let col_name = cols[i].name();
        // Try String first
        let val = row.try_get::<Option<String>, _>(i).ok()
            .map(|v| v.map(|s| serde_json::Value::String(s)).unwrap_or(serde_json::Value::Null))
            .or_else(|| row.try_get::<Option<i32>, _>(i).ok()
                .map(|v| v.map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null)))
            .or_else(|| row.try_get::<Option<i64>, _>(i).ok()
                .map(|v| v.map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null)))
            .or_else(|| row.try_get::<Option<f64>, _>(i).ok()
                .map(|v| v.map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null)))
            .or_else(|| row.try_get::<Option<bool>, _>(i).ok()
                .map(|v| v.map(|b| serde_json::json!(b)).unwrap_or(serde_json::Value::Null)))
            .or_else(|| row.try_get::<Option<Uuid>, _>(i).ok()
                .map(|v| v.map(|u| serde_json::json!(u.to_string())).unwrap_or(serde_json::Value::Null)))
            .or_else(|| row.try_get::<Option<serde_json::Value>, _>(i).ok()
                .map(|v| v.unwrap_or(serde_json::Value::Null)))
            .unwrap_or(serde_json::Value::Null);
        map.insert(col_name.to_string(), val);
    }
    serde_json::Value::Object(map)
}

async fn fetch_one(
    db: &PgPool, q: &str, bind: Uuid
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    match sqlx::query(q).bind(bind).fetch_optional(db).await? {
        Some(row) => Ok(Some(map_row(row))),
        None => Ok(None),
    }
}

async fn fetch_all(
    db: &PgPool, q: &str, bind: Uuid
) -> Result<Vec<serde_json::Value>, sqlx::Error> {
    let rows = sqlx::query(q).bind(bind).fetch_all(db).await?;
    Ok(rows.into_iter().map(map_row).collect())
}

// ── Transfer Directory ───────────────────────────────────────────────────────

pub async fn transfer_directory(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let new_owner = find_user_by_email(&s.db, &body.new_owner_email).await?;

    let dir_row = sqlx::query(
        "SELECT id, name, owner_id, network_id FROM directories WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let dir_id: Uuid = dir_row.get("id");
    let dir_name: String = dir_row.get("name");
    let dir_owner: Option<Uuid> = dir_row.get("owner_id");
    let network_id: Option<Uuid> = dir_row.get("network_id");

    if dir_owner.map(|o| o.to_string()) != Some(claims.sub.clone()) && claims.role != "superadmin" {
        return Err(AppError::Forbidden("You do not own this directory".into()));
    }

    let mut transferred = vec![TransferredItem {
        id: dir_id.to_string(),
        name: dir_name,
        kind: "directory".into(),
    }];

    sqlx::query("UPDATE directories SET owner_id = $1 WHERE id = $2")
        .bind(new_owner.id)
        .bind(dir_id)
        .execute(&s.db)
        .await?;

    if body.include_connected.unwrap_or(false) {
        if let Some(nid) = network_id {
            let siblings = sqlx::query(
                "SELECT id, name FROM directories WHERE network_id = $1 AND id != $2"
            )
            .bind(nid)
            .bind(dir_id)
            .fetch_all(&s.db)
            .await?;

            for sib in &siblings {
                let sib_id: Uuid = sib.get("id");
                let sib_name: String = sib.get("name");
                sqlx::query("UPDATE directories SET owner_id = $1 WHERE id = $2")
                    .bind(new_owner.id)
                    .bind(sib_id)
                    .execute(&s.db)
                    .await?;
                transferred.push(TransferredItem {
                    id: sib_id.to_string(),
                    name: sib_name,
                    kind: "directory".into(),
                });
            }

            if let Some(net_row) = sqlx::query("SELECT name FROM networks WHERE id = $1")
                .bind(nid)
                .fetch_optional(&s.db)
                .await?
            {
                let net_name: String = net_row.get("name");
                sqlx::query("UPDATE networks SET owner_id = $1 WHERE id = $2")
                    .bind(new_owner.id)
                    .bind(nid)
                    .execute(&s.db)
                    .await?;
                transferred.push(TransferredItem {
                    id: nid.to_string(),
                    name: net_name,
                    kind: "network".into(),
                });
            }
        }
    }

    Ok(Json(json!({
        "message": format!("Transferred {} item(s) to {}", transferred.len(), body.new_owner_email),
        "transferred": transferred
    })))
}

// ── Transfer Network ─────────────────────────────────────────────────────────

pub async fn transfer_network(
    Extension(claims): Extension<Claims>,
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let new_owner = find_user_by_email(&s.db, &body.new_owner_email).await?;

    let net_row = sqlx::query(
        "SELECT id, name, owner_id FROM networks WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Network not found".into()))?;

    let net_id: Uuid = net_row.get("id");
    let net_name: String = net_row.get("name");
    let net_owner: Option<Uuid> = net_row.get("owner_id");

    if net_owner.map(|o| o.to_string()) != Some(claims.sub.clone()) && claims.role != "superadmin" {
        return Err(AppError::Forbidden("You do not own this network".into()));
    }

    let mut transferred = vec![TransferredItem {
        id: net_id.to_string(),
        name: net_name.clone(),
        kind: "network".into(),
    }];

    sqlx::query("UPDATE networks SET owner_id = $1 WHERE id = $2")
        .bind(new_owner.id)
        .bind(net_id)
        .execute(&s.db)
        .await?;

    let dirs = sqlx::query(
        "SELECT id, name FROM directories WHERE network_id = $1"
    )
    .bind(net_id)
    .fetch_all(&s.db)
    .await?;

    for d in &dirs {
        let d_id: Uuid = d.get("id");
        let d_name: String = d.get("name");
        sqlx::query("UPDATE directories SET owner_id = $1 WHERE id = $2")
            .bind(new_owner.id)
            .bind(d_id)
            .execute(&s.db)
            .await?;
        transferred.push(TransferredItem {
            id: d_id.to_string(),
            name: d_name,
            kind: "directory".into(),
        });
    }

    Ok(Json(json!({
        "message": format!("Transferred network '{}' + {} directory(ies) to {}", net_name, dirs.len(), body.new_owner_email),
        "transferred": transferred
    })))
}

// ── Export Directory ─────────────────────────────────────────────────────────

pub async fn export_directory(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let dir = fetch_one(&s.db, "SELECT * FROM directories WHERE id = $1", id)
        .await?
        .ok_or_else(|| AppError::NotFound("Directory not found".into()))?;

    let categories = fetch_all(&s.db,
        "SELECT * FROM directory_categories WHERE directory_id = $1 ORDER BY name", id
    ).await?;

    let businesses = fetch_all(&s.db,
        "SELECT * FROM businesses WHERE directory_id = $1 ORDER BY name", id
    ).await?;

    let blog_posts = fetch_all(&s.db,
        "SELECT * FROM blog_posts WHERE directory_id = $1 ORDER BY created_at DESC", id
    ).await?;

    let reviews = fetch_all(&s.db,
        "SELECT r.* FROM reviews r JOIN businesses b ON r.business_id = b.id WHERE b.directory_id = $1", id
    ).await?;

    let deals = fetch_all(&s.db,
        "SELECT d.* FROM deals d JOIN businesses b ON d.business_id = b.id WHERE b.directory_id = $1", id
    ).await?;

    let seo_settings = sqlx::query(
        "SELECT * FROM seo_meta WHERE page_type = 'directory' AND page_id = $1::uuid"
    )
    .bind(id.to_string())
    .fetch_all(&s.db)
    .await?;
    let seo_settings: Vec<serde_json::Value> = seo_settings.into_iter().map(map_row).collect();

    let branding = fetch_one(&s.db,
        "SELECT * FROM directory_branding WHERE directory_id = $1", id
    ).await?;

    Ok(Json(json!({
        "version": "1.0",
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "directory": dir,
        "categories": categories,
        "businesses": businesses,
        "blog_posts": blog_posts,
        "reviews": reviews,
        "deals": deals,
        "public_pages": [],
        "seo_settings": seo_settings,
        "branding": branding
    })))
}

// ── Export Network ───────────────────────────────────────────────────────────

pub async fn export_network(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let network = fetch_one(&s.db, "SELECT * FROM networks WHERE id = $1", id)
        .await?
        .ok_or_else(|| AppError::NotFound("Network not found".into()))?;

    let branding = fetch_one(&s.db,
        "SELECT * FROM network_branding WHERE network_id = $1", id
    ).await?;

    let sections = fetch_all(&s.db,
        "SELECT * FROM homepage_sections WHERE network_id = $1 ORDER BY sort_order", id
    ).await?;

    let dir_ids: Vec<Uuid> = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM directories WHERE network_id = $1"
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect();

    let mut dir_exports = Vec::new();
    for did in dir_ids {
        let dir = fetch_one(&s.db, "SELECT * FROM directories WHERE id = $1", did)
            .await?.unwrap_or_default();
        let cats = fetch_all(&s.db,
            "SELECT * FROM directory_categories WHERE directory_id = $1 ORDER BY name", did
        ).await?;
        let bizs = fetch_all(&s.db,
            "SELECT * FROM businesses WHERE directory_id = $1 ORDER BY name", did
        ).await?;
        let posts = fetch_all(&s.db,
            "SELECT * FROM blog_posts WHERE directory_id = $1 ORDER BY created_at DESC", did
        ).await?;
        let revs = fetch_all(&s.db,
            "SELECT r.* FROM reviews r JOIN businesses b ON r.business_id = b.id WHERE b.directory_id = $1", did
        ).await?;
        let branding_local = fetch_one(&s.db,
            "SELECT * FROM directory_branding WHERE directory_id = $1", did
        ).await?;

        dir_exports.push(json!({
            "version": "1.0",
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "directory": dir,
            "categories": cats,
            "businesses": bizs,
            "blog_posts": posts,
            "reviews": revs,
            "deals": [],
            "public_pages": [],
            "seo_settings": [],
            "branding": branding_local
        }));
    }

    Ok(Json(json!({
        "version": "1.0",
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "network": network,
        "branding": branding,
        "homepage_sections": sections,
        "directories": dir_exports
    })))
}
