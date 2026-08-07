//! Co-op Buying Groups handlers — BL26
//! Businesses form buying groups to negotiate better pricing with suppliers.
//! Group purchasing + collective bargaining = a marketplace Google can't touch.

use axum::{
    extract::{Path, State, Query},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::auth::middleware::verify_token;
use crate::error::{AppError, ApiResult};

// ── Auth helpers ──

fn extract_user_id(headers: &HeaderMap, state: &AppState) -> ApiResult<Uuid> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;
    let claims = verify_token(token, &state.config.jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

async fn resolve_business_id(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Uuid> {
    let biz_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT cb.business_id
           FROM claimed_businesses cb
           WHERE cb.visitor_account_id = $1
           ORDER BY cb.created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    if let Some(bid) = biz_id {
        return Ok(bid);
    }

    let biz_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT cb.business_id
           FROM claimed_businesses cb
           WHERE cb.user_id = $1
           ORDER BY cb.created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    if let Some(bid) = biz_id {
        return Ok(bid);
    }

    let email = sqlx::query_scalar::<_, String>("SELECT email FROM visitor_accounts WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await?;

    if let Some(ref em) = email {
        let biz_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM businesses WHERE email = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(em)
        .fetch_optional(db)
        .await?;

        if let Some(bid) = biz_id {
            return Ok(bid);
        }
    }

    Err(AppError::NotFound(
        "No business linked to your account. Claim a business first.".into(),
    ))
}

// ── Types ──

#[derive(Debug, FromRow, Serialize)]
struct BuyingGroupRow {
    id: Uuid,
    name: String,
    description: Option<String>,
    category: Option<String>,
    founder_business_id: Uuid,
    status: String,
    member_count: Option<i32>,
    min_members: Option<i32>,
    max_members: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_deals: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

#[derive(Debug, FromRow, Serialize)]
struct GroupMemberRow {
    id: Uuid,
    group_id: Uuid,
    business_id: Uuid,
    role: String,
    joined_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow, Serialize)]
struct GroupDealRow {
    id: Uuid,
    group_id: Uuid,
    title: String,
    description: Option<String>,
    supplier_business_id: Uuid,
    product_name: String,
    normal_price: Option<Decimal>,
    group_price: Decimal,
    min_quantity: i32,
    current_quantity: Option<i32>,
    unit: Option<String>,
    deadline: Option<NaiveDate>,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    committed_quantity: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GroupQuery {
    pub category: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub min_members: Option<i32>,
    pub max_members: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDealRequest {
    pub title: String,
    pub description: Option<String>,
    pub product_name: String,
    pub normal_price: Option<Decimal>,
    pub group_price: Decimal,
    pub min_quantity: i32,
    pub unit: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommitRequest {
    pub quantity: i32,
}

// ── API: GET /b2b/co-op/groups ──

pub async fn list_groups(
    State(state): State<AppState>,
    Query(params): Query<GroupQuery>,
) -> ApiResult<impl IntoResponse> {
    let per_page = params.per_page.unwrap_or(20).min(100);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let groups = sqlx::query_as::<_, BuyingGroupRow>(
        "SELECT g.id, g.name, g.description, g.category, g.founder_business_id, g.status, \
         g.member_count, g.min_members, g.max_members, g.created_at, g.updated_at, \
         COALESCE((SELECT COUNT(*) FROM buying_group_deals WHERE group_id = g.id AND status = 'active'), 0) as active_deals, \
         NULL::text as role \
         FROM buying_groups g \
         WHERE g.status IN ('recruiting', 'active') \
         ORDER BY g.created_at DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM buying_groups WHERE status IN ('recruiting', 'active')",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    Ok(Json(json!({
        "groups": groups,
        "total": total,
        "page": page,
        "per_page": per_page,
    })))
}

// ── API: POST /b2b/co-op/groups ──

pub async fn create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateGroupRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.name.trim().is_empty() {
        return Err(AppError::Validation("Group name is required".into()));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let group_id = Uuid::new_v4();
    let min_members = req.min_members.unwrap_or(2);
    let max_members = req.max_members;

    sqlx::query(
        "INSERT INTO buying_groups (id, name, description, category, founder_business_id, min_members, max_members) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(group_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.category)
    .bind(biz_id)
    .bind(min_members)
    .bind(max_members)
    .execute(&state.db)
    .await?;

    sqlx::query(
        "INSERT INTO buying_group_members (group_id, business_id, role) VALUES ($1, $2, 'founder')",
    )
    .bind(group_id)
    .bind(biz_id)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "id": group_id,
        "name": req.name,
        "founder_business_id": biz_id,
        "status": "recruiting",
    })))
}

// ── API: GET /b2b/co-op/groups/:id ──

pub async fn get_group(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let group = sqlx::query_as::<_, BuyingGroupRow>(
        "SELECT g.id, g.name, g.description, g.category, g.founder_business_id, g.status, \
         g.member_count, g.min_members, g.max_members, g.created_at, g.updated_at, \
         NULL::bigint as active_deals, NULL::text as role \
         FROM buying_groups g WHERE g.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;

    let group = match group {
        Some(g) => g,
        None => return Err(AppError::NotFound("Buying group not found".into())),
    };

    let members = sqlx::query_as::<_, GroupMemberRow>(
        "SELECT id, group_id, business_id, role, joined_at \
         FROM buying_group_members WHERE group_id = $1 ORDER BY joined_at ASC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let deals = sqlx::query_as::<_, GroupDealRow>(
        "SELECT id, group_id, title, description, supplier_business_id, \
         product_name, normal_price, group_price, min_quantity, current_quantity, \
         unit, deadline, status, created_at, updated_at, \
         NULL::text as group_name, NULL::text as group_category, NULL::bigint as committed_quantity \
         FROM buying_group_deals \
         WHERE group_id = $1 AND status = 'active' ORDER BY created_at DESC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    Ok(Json(json!({
        "group": group,
        "members": members,
        "active_deals": deals,
    })))
}

// ── API: POST /b2b/co-op/groups/:id/join ──

pub async fn join_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let group_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM buying_groups WHERE id = $1",
    )
    .bind(group_id)
    .fetch_optional(&state.db)
    .await?;

    match group_status {
        Some(s) if s == "recruiting" || s == "active" => {}
        Some(s) => {
            return Err(AppError::Validation(format!(
                "Cannot join group with status: {}",
                s
            )))
        }
        None => return Err(AppError::NotFound("Buying group not found".into())),
    }

    let already: bool = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM buying_group_members WHERE group_id = $1 AND business_id = $2",
    )
    .bind(group_id)
    .bind(biz_id)
    .fetch_one(&state.db)
    .await
    .map(|c| c > 0)
    .unwrap_or(false);

    if already {
        return Err(AppError::Duplicate(
            "You are already a member of this group".into(),
        ));
    }

    sqlx::query(
        "INSERT INTO buying_group_members (group_id, business_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(group_id)
    .bind(biz_id)
    .execute(&state.db)
    .await?;

    let _ = sqlx::query(
        "UPDATE buying_groups SET member_count = (SELECT COUNT(*) FROM buying_group_members WHERE group_id = $1), updated_at = now() WHERE id = $1",
    )
    .bind(group_id)
    .execute(&state.db)
    .await;

    let _ = sqlx::query(
        "UPDATE buying_groups SET status = 'active', updated_at = now() WHERE id = $1 AND status = 'recruiting' AND member_count >= min_members",
    )
    .bind(group_id)
    .execute(&state.db)
    .await;

    Ok(Json(json!({
        "group_id": group_id,
        "business_id": biz_id,
        "status": "joined",
    })))
}

// ── API: GET /b2b/co-op/my-groups ──

pub async fn my_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let groups = sqlx::query_as::<_, BuyingGroupRow>(
        "SELECT g.id, g.name, g.description, g.category, g.founder_business_id, g.status, \
         g.member_count, g.min_members, g.max_members, g.created_at, g.updated_at, \
         NULL::bigint as active_deals, m.role \
         FROM buying_groups g \
         JOIN buying_group_members m ON m.group_id = g.id AND m.business_id = $1 \
         ORDER BY g.created_at DESC",
    )
    .bind(biz_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({ "my_groups": groups })))
}

// ── API: POST /b2b/co-op/groups/:id/deals ──

pub async fn create_deal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateDealRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.title.trim().is_empty() {
        return Err(AppError::Validation("Deal title is required".into()));
    }
    if req.min_quantity <= 0 {
        return Err(AppError::Validation(
            "Minimum quantity must be greater than zero".into(),
        ));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let member_role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM buying_group_members WHERE group_id = $1 AND business_id = $2",
    )
    .bind(group_id)
    .bind(biz_id)
    .fetch_optional(&state.db)
    .await?;

    match member_role {
        Some(r) if r == "founder" || r == "admin" => {}
        Some(_) => {
            return Err(AppError::Forbidden(
                "Only group founders/admins can create deals".into(),
            ))
        }
        None => {
            return Err(AppError::Forbidden(
                "You must be a member of this group to create deals".into(),
            ))
        }
    }

    let deadline = req
        .deadline
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());

    let deal_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO buying_group_deals (id, group_id, title, description, supplier_business_id, \
         product_name, normal_price, group_price, min_quantity, unit, deadline) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(deal_id)
    .bind(group_id)
    .bind(&req.title)
    .bind(&req.description)
    .bind(biz_id)
    .bind(&req.product_name)
    .bind(req.normal_price)
    .bind(req.group_price)
    .bind(req.min_quantity)
    .bind(req.unit.as_deref().unwrap_or("each"))
    .bind(deadline)
    .execute(&state.db)
    .await?;

    Ok(Json(json!({
        "id": deal_id,
        "group_id": group_id,
        "status": "active",
        "group_price": req.group_price,
        "min_quantity": req.min_quantity,
    })))
}

// ── API: GET /b2b/co-op/deals/active ──

pub async fn active_deals(
    State(state): State<AppState>,
    Query(params): Query<GroupQuery>,
) -> ApiResult<impl IntoResponse> {
    let per_page = params.per_page.unwrap_or(20).min(100);
    let page = params.page.unwrap_or(1).max(1);
    let offset = (page - 1) * per_page;

    let deals = sqlx::query_as::<_, GroupDealRow>(
        "SELECT d.id, d.group_id, d.title, d.description, d.supplier_business_id, \
         d.product_name, d.normal_price, d.group_price, d.min_quantity, d.current_quantity, \
         d.unit, d.deadline, d.status, d.created_at, d.updated_at, \
         g.name as group_name, g.category as group_category, \
         COALESCE((SELECT SUM(c.quantity) FROM group_deal_commitments c WHERE c.deal_id = d.id), 0) as committed_quantity \
         FROM buying_group_deals d \
         JOIN buying_groups g ON g.id = d.group_id \
         WHERE d.status = 'active' AND (d.deadline IS NULL OR d.deadline >= CURRENT_DATE) \
         ORDER BY d.created_at DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM buying_group_deals WHERE status = 'active' AND (deadline IS NULL OR deadline >= CURRENT_DATE)",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    Ok(Json(json!({
        "deals": deals,
        "total": total,
        "page": page,
        "per_page": per_page,
    })))
}

// ── API: POST /b2b/co-op/deals/:id/commit ──

pub async fn commit_to_deal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(deal_id): Path<Uuid>,
    Json(req): Json<CommitRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.quantity <= 0 {
        return Err(AppError::Validation(
            "Quantity must be greater than zero".into(),
        ));
    }

    let user_id = extract_user_id(&headers, &state)?;
    let biz_id = resolve_business_id(&state.db, user_id).await?;

    let deal = sqlx::query_as::<_, (Uuid, String, Decimal, i32)>(
        "SELECT group_id, status, group_price, min_quantity FROM buying_group_deals WHERE id = $1",
    )
    .bind(deal_id)
    .fetch_optional(&state.db)
    .await?;

    let (group_id, deal_status, group_price, _min_qty) = match deal {
        Some(d) => d,
        None => return Err(AppError::NotFound("Deal not found".into())),
    };

    if deal_status != "active" {
        return Err(AppError::Validation(format!(
            "Deal is not active (status: {})",
            deal_status
        )));
    }

    let is_member: bool = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM buying_group_members WHERE group_id = $1 AND business_id = $2",
    )
    .bind(group_id)
    .bind(biz_id)
    .fetch_one(&state.db)
    .await
    .map(|c| c > 0)
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden(
            "You must be a member of the buying group to commit to deals".into(),
        ));
    }

    let total_amount = group_price * Decimal::from(req.quantity);

    let commit_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO group_deal_commitments (id, deal_id, business_id, quantity, total_amount) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(commit_id)
    .bind(deal_id)
    .bind(biz_id)
    .bind(req.quantity)
    .bind(total_amount)
    .execute(&state.db)
    .await?;

    let _ = sqlx::query(
        "UPDATE buying_group_deals SET current_quantity = (SELECT COALESCE(SUM(quantity), 0) FROM group_deal_commitments WHERE deal_id = $1), updated_at = now() WHERE id = $1",
    )
    .bind(deal_id)
    .execute(&state.db)
    .await;

    Ok(Json(json!({
        "id": commit_id,
        "deal_id": deal_id,
        "business_id": biz_id,
        "quantity": req.quantity,
        "total_amount": total_amount,
        "status": "committed",
    })))
}
