//! B2B Marketplace handlers — BL23
//! Suppliers list products. Businesses search, browse, and connect.
//! Distinct from regular business listings with supplier-specific features.
//!
//! Auth: Product CRUD requires JWT and is scoped to the user's claimed supplier business.
//!       Search and list endpoints are public (no auth needed).

use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::{create_token, verify_token};
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

// ── B2B Feature Config Helpers ──

use serde_json::Value;

/// Default B2B feature flags — all enabled by default
pub(crate) fn default_b2b_config() -> Value {
    json!({
        "b2b_marketplace": true,
        "b2b_orders": true,
        "b2b_messages": true,
        "b2b_discover": true,
    })
}

pub(crate) async fn get_b2b_config(db: &sqlx::PgPool, directory_id: Option<Uuid>) -> Value {
    if let Some(dir_id) = directory_id {
        let fc: Option<Value> =
            sqlx::query_scalar("SELECT feature_config FROM directories WHERE id = $1")
                .bind(dir_id)
                .fetch_optional(db)
                .await
                .ok()
                .flatten();

        if let Some(mut fc) = fc {
            // Merge with defaults so missing keys get default values
            let defaults = default_b2b_config();
            if let (Value::Object(ref mut map), Value::Object(def_map)) = (&mut fc, &defaults) {
                for (k, v) in def_map {
                    if !map.contains_key(k) {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
            return fc;
        }
    }
    default_b2b_config()
}

/// Returns Err if a specific B2B feature is disabled
pub(crate) fn require_b2b_feature(config: &Value, feature: &str) -> ApiResult<()> {
    let enabled = config
        .get(feature)
        .and_then(|v| v.as_bool())
        .unwrap_or(true); // default: enabled
    if !enabled {
        return Err(AppError::Forbidden(format!(
            "{} is not enabled for this directory",
            feature
        )));
    }
    Ok(())
}

/// Get the first active directory ID — since B2B is global (not directory-scoped in v1),
/// we use the first directory's feature config as the network-wide toggle.
pub(crate) async fn get_first_directory_id(db: &sqlx::PgPool) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM directories WHERE status = 'active' ORDER BY created_at, id LIMIT 1",
    )
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

#[derive(Debug, Deserialize)]
pub struct ProductQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    pub business_id: Option<Uuid>,
    pub delivery_area: Option<String>,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductRequest {
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub price: Option<f64>,
    pub unit: Option<String>,
    pub min_order: Option<i32>,
    pub delivery_areas: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProductRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub price: Option<f64>,
    pub unit: Option<String>,
    pub min_order: Option<i32>,
    pub delivery_areas: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SupplierProduct {
    pub id: Uuid,
    pub business_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub price: Option<rust_decimal::Decimal>,
    pub unit: Option<String>,
    pub min_order: Option<i32>,
    pub currency: Option<String>,
    pub delivery_areas: Option<Vec<String>>,
    pub is_active: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct B2bRegisterRequest {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub business_type: String,
}

/// POST /api/v1/b2b/register — distributor/B2B supplier registration (network-wide)
///
/// Creates a visitor_account AND a matching business record. The link is established
/// via email matching (resolve_supplier_business looks up by email for visitor accounts).
pub async fn b2b_register(
    State(s): State<AppState>,
    Json(req): Json<B2bRegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.email.is_empty() || req.password.is_empty() || req.business_type.is_empty() {
        return Err(AppError::Validation(
            "Email, password, and business type are required".to_string(),
        ));
    }
    if req.password.len() < 6 {
        return Err(AppError::Validation(
            "Password must be at least 6 characters".to_string(),
        ));
    }

    let valid_types = [
        "association",
        "farm",
        "wholesaler",
        "distributor",
        "manufacturer",
        "other",
    ];
    let bt_lower = req.business_type.to_lowercase();
    if !valid_types.contains(&bt_lower.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid business_type '{}'. Must be one of: {}",
            req.business_type,
            valid_types.join(", ")
        )));
    }

    let existing =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM visitor_accounts WHERE email = $1")
            .bind(&req.email)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);

    if existing > 0 {
        return Err(AppError::Duplicate(
            "An account with this email already exists".to_string(),
        ));
    }

    // Hash password
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| AppError::Hash(e.to_string()))?
        .to_string();

    // Create visitor_account
    let visitor = sqlx::query_as::<_, crate::handlers::portal::VisitorAccount>(
        "INSERT INTO visitor_accounts (email, password_hash, name, phone, directory_id, business_type) \
         VALUES ($1, $2, $3, $4, NULL, $5) RETURNING *"
    )
    .bind(&req.email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind(&req.phone)
    .bind(&bt_lower)
    .fetch_one(&s.db)
    .await?;

    // Create a business record for the supplier (linked via email)
    let business_id = Uuid::new_v4();
    let biz_name = req.name.as_deref().unwrap_or("Unnamed Supplier");
    let biz_slug = format!(
        "{}-{}",
        biz_name
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "-")
            .trim_matches('-'),
        &business_id.to_string()[..8]
    );
    let biz_desc = format!(
        "{} supplier on ZaarHub Network",
        bt_lower
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            + &bt_lower[1..]
    );

    sqlx::query(
        "INSERT INTO businesses (id, name, email, phone, slug, business_type, description, is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, true, NOW(), NOW())"
    )
    .bind(business_id)
    .bind(biz_name)
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&biz_slug)
    .bind(&bt_lower)
    .bind(&biz_desc)
    .execute(&s.db)
    .await?;

    // Auto-claim: link the visitor account to the newly created business
    sqlx::query(
        "INSERT INTO claimed_businesses (business_id, owner_email, owner_name, owner_phone, verified_at, is_active, visitor_account_id) \
         VALUES ($1, $2, $3, $4, NOW(), true, $5) \
         ON CONFLICT (business_id) DO NOTHING"
    )
    .bind(business_id)
    .bind(&req.email)
    .bind(&req.name)
    .bind(&req.phone)
    .bind(visitor.id)
    .execute(&s.db)
    .await?;

    // Update last_login
    sqlx::query("UPDATE visitor_accounts SET last_login_at = NOW() WHERE id = $1")
        .bind(visitor.id)
        .execute(&s.db)
        .await?;

    // Fire tag sync in background
    let ts_db = s.db.clone();
    let ts_email = visitor.email.clone();
    let ts_name = visitor.name.clone();
    let ts_phone = visitor.phone.clone();
    let ts_business_type = bt_lower.clone();
    tokio::spawn(async move {
        let cap = format!(
            "{}{}",
            ts_business_type
                .chars()
                .next()
                .unwrap_or('S')
                .to_uppercase(),
            &ts_business_type[1..]
        );
        crate::handlers::tag_sync::fire_tag_sync(
            &ts_db,
            ts_email,
            ts_name,
            None,
            ts_phone,
            vec!["Supplier".to_string(), cap],
            None,
            Some("suppliers".to_string()),
            None,
            Some("b2b_register".to_string()),
            Some("2944af81-2086-44b8-93c1-d83e93a5dec1".to_string()),
            Some("043fb15c-0874-4f41-b81a-4f324ce98b23".to_string()),
        );
    });

    // Generate JWT
    let now_ts = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: visitor.id.to_string(),
        tid: "00000000-0000-0000-0000-000000000000".to_string(),
        role: "visitor".to_string(),
        exp: now_ts + s.config.jwt_access_expiry as usize,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
    };
    let token = create_token(&claims, &s.config.jwt_secret)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "access_token": token,
            "token_type": "Bearer",
            "expires_in": s.config.jwt_access_expiry,
            "visitor": {
                "id": visitor.id,
                "email": visitor.email,
                "name": visitor.name,
                "phone": visitor.phone,
                "business_type": bt_lower,
                "business_id": business_id,
                "directory_id": serde_json::Value::Null,
                "is_active": visitor.is_active,
                "created_at": visitor.created_at,
            },
        })),
    ))
}

/// Extract user_id from Authorization header JWT (used by handlers outside auth_guard)
fn extract_user_id(headers: &HeaderMap, state: &AppState) -> ApiResult<Uuid> {
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;
    let claims =
        verify_token(token, &state.config.jwt_secret).map_err(|_| AppError::Unauthorized)?;
    Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)
}

/// POST /api/v1/b2b/products — supplier adds a product (auth required, scoped to claimed business)
pub async fn create_product(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateProductRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO supplier_products (id, business_id, name, description, category, price, unit, min_order, delivery_areas)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
    )
    .bind(id)
    .bind(biz_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.category)
    .bind(req.price)
    .bind(&req.unit)
    .bind(req.min_order)
    .bind(&req.delivery_areas)
    .execute(&s.db)
    .await?;

    Ok(Json(
        json!({"id": id, "business_id": biz_id, "status": "created"}),
    ))
}

/// GET /api/v1/b2b/products — search products (public, no auth needed)
pub async fn search_products(
    State(s): State<AppState>,
    Query(qs): Query<ProductQuery>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let mut wheres = vec!["sp.is_active = true".to_string()];

    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            wheres.push(format!("(sp.name ILIKE '%' || $1 || '%' OR COALESCE(sp.description,'') ILIKE '%' || $1 || '%')"));
        }
    }
    if let Some(ref _cat) = qs.category {
        if !_cat.is_empty() {
            wheres.push(format!("sp.category = $2"));
        }
    }
    if qs.business_id.is_some() {
        wheres.push(format!("sp.business_id = $3"));
    }
    if let Some(ref _area) = qs.delivery_area {
        if !_area.is_empty() {
            wheres.push(format!("$4 = ANY(sp.delivery_areas)"));
        }
    }
    if qs.max_price.is_some() {
        wheres.push(format!("COALESCE(sp.price, 0) <= $5"));
    }

    let where_clause = if wheres.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wheres.join(" AND "))
    };

    // Count query
    let count_sql = format!("SELECT COUNT(*) FROM supplier_products sp {}", where_clause);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            count_q = count_q.bind(q);
        }
    }
    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            count_q = count_q.bind(cat);
        }
    }
    if let Some(bid) = qs.business_id {
        count_q = count_q.bind(bid);
    }
    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            count_q = count_q.bind(area);
        }
    }
    if let Some(mp) = qs.max_price {
        count_q = count_q.bind(mp);
    }
    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    // Data query
    let data_sql = format!(
        "SELECT sp.id, sp.business_id, sp.name, sp.description, sp.category, sp.price, sp.unit, sp.min_order, \
                sp.currency, sp.delivery_areas, sp.is_active, sp.created_at, sp.updated_at, \
                b.name as business_name, b.city, b.state \
         FROM supplier_products sp \
         LEFT JOIN businesses b ON b.id = sp.business_id \
         {} ORDER BY sp.name ASC LIMIT 20 OFFSET {}",
        where_clause, offset
    );
    let mut data_q = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<rust_decimal::Decimal>,
            Option<String>,
            Option<i32>,
            Option<String>,
            Option<Vec<String>>,
            Option<bool>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<chrono::DateTime<chrono::Utc>>,
            String,
            Option<String>,
            Option<String>,
        ),
    >(&data_sql);
    if let Some(ref q) = qs.q {
        if !q.is_empty() {
            data_q = data_q.bind(q);
        }
    }
    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            data_q = data_q.bind(cat);
        }
    }
    if let Some(bid) = qs.business_id {
        data_q = data_q.bind(bid);
    }
    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            data_q = data_q.bind(area);
        }
    }
    if let Some(mp) = qs.max_price {
        data_q = data_q.bind(mp);
    }

    let rows = data_q.fetch_all(&s.db).await?;
    let results: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "business_id": r.1, "name": r.2, "description": r.3,
                "category": r.4, "price": r.5, "unit": r.6, "min_order": r.7,
                "currency": r.8, "delivery_areas": r.9, "is_active": r.10, "created_at": r.11,
                "business_name": r.13, "city": r.14, "state": r.15
            })
        })
        .collect();

    Ok(Json(
        json!({"products": results, "total": total, "page": page, "per_page": per_page}),
    ))
}

/// GET /api/v1/b2b/products/my — list the authenticated supplier's own products
pub async fn my_products(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let products = sqlx::query_as::<_, (Uuid, Uuid, String, Option<String>, Option<String>, Option<rust_decimal::Decimal>, Option<String>, Option<i32>, Option<String>, Option<Vec<String>>, Option<bool>, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT id, business_id, name, description, category, price, unit, min_order, currency, delivery_areas, is_active, created_at, updated_at \
         FROM supplier_products WHERE business_id = $1 ORDER BY created_at DESC"
    )
    .bind(biz_id)
    .fetch_all(&s.db)
    .await?;

    let results: Vec<serde_json::Value> = products
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "business_id": r.1, "name": r.2, "description": r.3,
                "category": r.4, "price": r.5, "unit": r.6, "min_order": r.7,
                "currency": r.8, "delivery_areas": r.9, "is_active": r.10, "created_at": r.11,
            })
        })
        .collect();

    Ok(Json(json!({"products": results, "total": results.len()})))
}

/// GET /api/v1/b2b/products/:id — get single product (public)
pub async fn get_product(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let product = sqlx::query_as::<_, SupplierProduct>(
        "SELECT id, business_id, name, description, category, price, unit, min_order, currency, \
                delivery_areas, is_active, created_at, updated_at \
         FROM supplier_products WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Product not found".into()))?;

    Ok(Json(json!(&product)))
}

/// PUT /api/v1/b2b/products/:id — update product (auth required, scoped to claimed business)
pub async fn update_product(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProductRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let owner_check =
        sqlx::query_scalar::<_, Uuid>("SELECT business_id FROM supplier_products WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Product not found".into()))?;

    if owner_check != biz_id {
        return Err(AppError::Forbidden(
            "You can only update your own products".into(),
        ));
    }

    sqlx::query(
        "UPDATE supplier_products SET name=COALESCE($1,name), description=COALESCE($2,description), \
         category=COALESCE($3,category), price=COALESCE($4,price), unit=COALESCE($5,unit), \
         min_order=COALESCE($6,min_order), delivery_areas=COALESCE($7,delivery_areas), \
         is_active=COALESCE($8,is_active), updated_at=NOW() WHERE id=$9"
    )
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.category)
    .bind(req.price)
    .bind(&req.unit)
    .bind(req.min_order)
    .bind(&req.delivery_areas)
    .bind(req.is_active)
    .bind(id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"status": "updated", "id": id})))
}

/// DELETE /api/v1/b2b/products/:id — delete product (auth required, scoped to claimed business)
pub async fn delete_product(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let owner_check =
        sqlx::query_scalar::<_, Uuid>("SELECT business_id FROM supplier_products WHERE id = $1")
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Product not found".into()))?;

    if owner_check != biz_id {
        return Err(AppError::Forbidden(
            "You can only delete your own products".into(),
        ));
    }

    sqlx::query("DELETE FROM supplier_products WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({"status": "deleted", "id": id})))
}

/// GET /api/v1/b2b/suppliers — list all supplier-type businesses (public)
pub async fn list_suppliers(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let suppliers = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>)>(
        "SELECT id, name, city, state, phone, website FROM businesses \
         WHERE business_type IN ('supplier','distributor','wholesaler','farm','association') AND COALESCE(is_active, true) = true \
         ORDER BY name ASC"
    )
    .fetch_all(&s.db)
    .await?;

    let result: Vec<serde_json::Value> = suppliers
        .into_iter()
        .map(|s| {
            json!({
                "id": s.0, "name": s.1, "city": s.2, "state": s.3, "phone": s.4, "website": s.5
            })
        })
        .collect();

    Ok(Json(json!({"suppliers": result, "total": result.len()})))
}

// ── Phase 2: B2B Orders ──

#[derive(Debug, Deserialize)]
pub struct PlaceOrderRequest {
    pub product_id: Uuid,
    pub quantity: Option<i32>,
    pub buyer_notes: Option<String>,
    pub delivery_area: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OrderListQuery {
    pub role: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOrderStatusRequest {
    pub status: String,
}

/// Row struct for B2B order queries (maps to SQL row with joins)
#[derive(Debug, sqlx::FromRow)]
struct B2bOrderRow {
    id: Uuid,
    buyer_business_id: Uuid,
    supplier_business_id: Uuid,
    product_id: Uuid,
    quantity: i32,
    unit_price: Option<rust_decimal::Decimal>,
    total_amount: Option<rust_decimal::Decimal>,
    status: String,
    buyer_notes: Option<String>,
    delivery_area: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
    shipped_at: Option<chrono::DateTime<chrono::Utc>>,
    delivered_at: Option<chrono::DateTime<chrono::Utc>>,
    product_name: String,
    buyer_name: String,
    supplier_name: String,
}

/// POST /api/v1/b2b/orders — place an order (authenticated buyer)
pub async fn place_order(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PlaceOrderRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let buyer_biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let quantity = req.quantity.unwrap_or(1).max(1);

    // Look up the product
    let product = sqlx::query_as::<_, (Uuid, Option<rust_decimal::Decimal>)>(
        "SELECT sp.business_id, sp.price FROM supplier_products sp WHERE sp.id = $1 AND COALESCE(sp.is_active, true) = true"
    )
    .bind(req.product_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Product not found or is inactive".into()))?;

    let supplier_biz_id = product.0;
    let unit_price = product.1;
    let total_amount = unit_price.map(|p| p * rust_decimal::Decimal::from(quantity));

    let order_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO b2b_orders (id, buyer_business_id, supplier_business_id, product_id, quantity, unit_price, total_amount, buyer_notes, delivery_area) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
    )
    .bind(order_id)
    .bind(buyer_biz_id)
    .bind(supplier_biz_id)
    .bind(req.product_id)
    .bind(quantity)
    .bind(unit_price)
    .bind(total_amount)
    .bind(&req.buyer_notes)
    .bind(&req.delivery_area)
    .execute(&s.db)
    .await?;

    // Get buyer business name for notification
    let buyer_name = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(name, 'Unknown Business') FROM businesses WHERE id = $1",
    )
    .bind(buyer_biz_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or_else(|_| "Unknown Business".to_string());

    // Get product name for notification body
    let product_name =
        sqlx::query_scalar::<_, String>("SELECT name FROM supplier_products WHERE id = $1")
            .bind(req.product_id)
            .fetch_one(&s.db)
            .await
            .unwrap_or_else(|_| "a product".to_string());

    // Notify the supplier
    let title = format!("New Order from {}", buyer_name);
    let body_text = format!(
        "Order for {} — {} units, ${}",
        product_name,
        quantity,
        total_amount.map_or("0.00".to_string(), |t| format!("{:.2}", t))
    );
    let _ = create_notification(
        &s.db,
        supplier_biz_id,
        "new_order",
        &title,
        Some(&body_text),
        Some(order_id),
        None,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": order_id,
            "buyer_business_id": buyer_biz_id,
            "supplier_business_id": supplier_biz_id,
            "product_id": req.product_id,
            "quantity": quantity,
            "unit_price": unit_price,
            "total_amount": total_amount,
            "status": "pending",
            "created_at": Utc::now()
        })),
    ))
}

/// GET /api/v1/b2b/orders — list orders for the authenticated business
pub async fn my_orders(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(qs): Query<OrderListQuery>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let role = qs.role.as_deref().unwrap_or("both");

    let mut wheres: Vec<String> = Vec::new();

    match role {
        "buyer" => wheres.push(format!("buyer_business_id = $1")),
        "supplier" => wheres.push(format!("supplier_business_id = $1")),
        _ => wheres.push(format!(
            "(buyer_business_id = $1 OR supplier_business_id = $1)"
        )),
    }

    if let Some(ref st) = qs.status {
        if !st.is_empty() {
            wheres.push(format!("status = $2"));
        }
    }

    let where_clause = format!("WHERE {}", wheres.join(" AND "));

    // Count
    let count_sql = format!("SELECT COUNT(*) FROM b2b_orders {}", where_clause);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(biz_id);
    if qs.status.as_ref().map_or(false, |s| !s.is_empty()) {
        count_q = count_q.bind(qs.status.as_ref().unwrap());
    }
    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    // Data query
    let data_sql = format!(
        "SELECT o.*, sp.name as product_name, bb.name as buyer_name, sb.name as supplier_name \
         FROM b2b_orders o \
         LEFT JOIN supplier_products sp ON sp.id = o.product_id \
         LEFT JOIN businesses bb ON bb.id = o.buyer_business_id \
         LEFT JOIN businesses sb ON sb.id = o.supplier_business_id \
         {} ORDER BY o.created_at DESC LIMIT {} OFFSET {}",
        where_clause, per_page, offset
    );

    let mut data_q = sqlx::query_as::<_, B2bOrderRow>(&data_sql).bind(biz_id);

    if qs.status.as_ref().map_or(false, |s| !s.is_empty()) {
        data_q = data_q.bind(qs.status.as_ref().unwrap());
    }

    let rows = data_q.fetch_all(&s.db).await?;
    let results: Vec<serde_json::Value> = rows.into_iter().map(|r| json!({
        "id": r.id, "buyer_business_id": r.buyer_business_id, "supplier_business_id": r.supplier_business_id,
        "product_id": r.product_id, "quantity": r.quantity, "unit_price": r.unit_price,
        "total_amount": r.total_amount, "status": r.status, "buyer_notes": r.buyer_notes,
        "delivery_area": r.delivery_area, "created_at": r.created_at, "updated_at": r.updated_at,
        "confirmed_at": r.confirmed_at, "shipped_at": r.shipped_at, "delivered_at": r.delivered_at,
        "product_name": r.product_name, "buyer_name": r.buyer_name, "supplier_name": r.supplier_name
    })).collect();

    Ok(Json(
        json!({"orders": results, "total": total, "page": page, "per_page": per_page}),
    ))
}

/// GET /api/v1/b2b/orders/:id — get single order (must be buyer or supplier)
pub async fn get_order(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let row = sqlx::query_as::<_, B2bOrderRow>(
        "SELECT o.*, sp.name as product_name, bb.name as buyer_name, sb.name as supplier_name \
         FROM b2b_orders o \
         LEFT JOIN supplier_products sp ON sp.id = o.product_id \
         LEFT JOIN businesses bb ON bb.id = o.buyer_business_id \
         LEFT JOIN businesses sb ON sb.id = o.supplier_business_id \
         WHERE o.id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Order not found".into()))?;

    // Authorization: must be buyer or supplier of this order
    if biz_id != row.buyer_business_id && biz_id != row.supplier_business_id {
        return Err(AppError::Forbidden(
            "You can only view your own orders".into(),
        ));
    }

    Ok(Json(json!({
        "id": row.id, "buyer_business_id": row.buyer_business_id, "supplier_business_id": row.supplier_business_id,
        "product_id": row.product_id, "quantity": row.quantity, "unit_price": row.unit_price, "total_amount": row.total_amount,
        "status": row.status, "buyer_notes": row.buyer_notes, "delivery_area": row.delivery_area,
        "created_at": row.created_at, "updated_at": row.updated_at,
        "confirmed_at": row.confirmed_at, "shipped_at": row.shipped_at, "delivered_at": row.delivered_at,
        "product_name": row.product_name, "buyer_name": row.buyer_name, "supplier_name": row.supplier_name
    })))
}

/// PUT /api/v1/b2b/orders/:id/status — update order status (supplier only)
pub async fn update_order_status(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateOrderStatusRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let valid_statuses = ["confirmed", "shipped", "delivered", "cancelled"];
    if !valid_statuses.contains(&req.status.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid status '{}'. Must be one of: {}",
            req.status,
            valid_statuses.join(", ")
        )));
    }

    // Verify the supplier owns this order
    let order = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT supplier_business_id, buyer_business_id, status FROM b2b_orders WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Order not found".into()))?;

    if order.0 != biz_id {
        return Err(AppError::Forbidden(
            "Only the supplier can update order status".into(),
        ));
    }

    let buyer_biz_id = order.1;

    // Build the SET clause with appropriate timestamp
    let now = Utc::now();
    match req.status.as_str() {
        "confirmed" => {
            sqlx::query("UPDATE b2b_orders SET status = 'confirmed', confirmed_at = $1, updated_at = $1 WHERE id = $2")
                .bind(now).bind(id).execute(&s.db).await?;
        }
        "shipped" => {
            sqlx::query("UPDATE b2b_orders SET status = 'shipped', shipped_at = $1, updated_at = $1 WHERE id = $2")
                .bind(now).bind(id).execute(&s.db).await?;
        }
        "delivered" => {
            sqlx::query("UPDATE b2b_orders SET status = 'delivered', delivered_at = $1, updated_at = $1 WHERE id = $2")
                .bind(now).bind(id).execute(&s.db).await?;
        }
        "cancelled" => {
            sqlx::query(
                "UPDATE b2b_orders SET status = 'cancelled', updated_at = $1 WHERE id = $2",
            )
            .bind(now)
            .bind(id)
            .execute(&s.db)
            .await?;
        }
        _ => unreachable!(),
    }

    // Notify the buyer about the status change
    let order_id_prefix = &id.to_string()[..8];
    let title = format!("Order #{} is now {}", order_id_prefix, req.status);
    let body_text = format!("Your order status has been updated to: {}", req.status);
    let notif_type = format!("order_{}", req.status);
    let _ = create_notification(
        &s.db,
        buyer_biz_id,
        &notif_type,
        &title,
        Some(&body_text),
        Some(id),
        None,
    )
    .await;

    Ok(Json(
        json!({"id": id, "status": req.status, "updated_at": now}),
    ))
}

// ── Phase 2: B2B Messaging ──

#[derive(Debug, Deserialize)]
pub struct SendB2bMessageRequest {
    pub to_business_id: Uuid,
    pub subject: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct B2bMessageListQuery {
    pub is_read: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// POST /api/v1/b2b/messages — send a message to another business
pub async fn send_b2b_message(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SendB2bMessageRequest>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_messages")?;

    let user_id = extract_user_id(&headers, &s)?;
    let sender_biz_id = resolve_buyer_business(&s.db, user_id).await?;

    // Get sender business details
    let sender = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT name, email FROM businesses WHERE id = $1",
    )
    .bind(sender_biz_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Your business profile not found".into()))?;

    // Verify recipient exists
    let recipient_exists =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM businesses WHERE id = $1")
            .bind(req.to_business_id)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);

    if recipient_exists == 0 {
        return Err(AppError::NotFound("Recipient business not found".into()));
    }

    let msg_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO business_messages (id, business_id, to_business_id, sender_business_id, sender_name, sender_email, subject, message) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(msg_id)
    .bind(req.to_business_id)  // business_id = recipient (legacy field)
    .bind(req.to_business_id)
    .bind(sender_biz_id)
    .bind(&sender.0)
    .bind(&sender.1)
    .bind(&req.subject)
    .bind(&req.message)
    .execute(&s.db)
    .await?;

    // Notify the recipient
    let sender_name = &sender.0;
    let title = format!("New message from {}", sender_name);
    let truncated_body = if req.subject.len() > 100 {
        &req.subject[..100]
    } else {
        &req.subject
    };
    let _ = create_notification(
        &s.db,
        req.to_business_id,
        "new_message",
        &title,
        Some(truncated_body),
        None,
        Some(msg_id),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": msg_id, "status": "sent"})),
    ))
}

/// GET /api/v1/b2b/messages — list messages received by the authenticated business
pub async fn my_b2b_messages(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(qs): Query<B2bMessageListQuery>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_messages")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let mut wheres = vec!["to_business_id = $1".to_string()];
    if let Some(read) = qs.is_read {
        wheres.push(format!("is_read = $2"));
    }
    let where_clause = format!("WHERE {}", wheres.join(" AND "));

    // Count
    let count_sql = format!("SELECT COUNT(*) FROM business_messages {}", where_clause);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(biz_id);
    if qs.is_read.is_some() {
        count_q = count_q.bind(qs.is_read.unwrap());
    }
    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    // Data query — messages sent TO this business via B2B
    let data_sql = format!(
        "SELECT id, business_id, sender_business_id, sender_name, sender_email, subject, message, is_read, created_at \
         FROM business_messages {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        where_clause, per_page, offset
    );

    let mut data_q = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Option<Uuid>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            bool,
            chrono::DateTime<chrono::Utc>,
        ),
    >(&data_sql)
    .bind(biz_id);

    if qs.is_read.is_some() {
        data_q = data_q.bind(qs.is_read.unwrap());
    }

    let rows = data_q.fetch_all(&s.db).await?;
    let results: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0, "to_business_id": r.1, "sender_business_id": r.2,
                "sender_name": r.3, "sender_email": r.4, "subject": r.5,
                "message": r.6, "is_read": r.7, "created_at": r.8
            })
        })
        .collect();

    Ok(Json(
        json!({"messages": results, "total": total, "page": page, "per_page": per_page}),
    ))
}

/// PUT /api/v1/b2b/messages/:id/read — mark a message as read
pub async fn mark_message_read(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_messages")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    // Verify this message is for this business
    let msg = sqlx::query_scalar::<_, Uuid>(
        "SELECT to_business_id FROM business_messages WHERE id = $1 AND to_business_id IS NOT NULL",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Message not found".into()))?;

    if msg != biz_id {
        return Err(AppError::Forbidden(
            "You can only mark your own messages as read".into(),
        ));
    }

    sqlx::query("UPDATE business_messages SET is_read = true WHERE id = $1")
        .bind(id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({"id": id, "is_read": true})))
}

// ── Phase 3b: Public B2B Marketplace ──

/// Query params for the public marketplace endpoint.
#[derive(Debug, Deserialize)]
pub struct MarketplaceQuery {
    pub category: Option<String>,
    pub search: Option<String>,
    pub delivery_area: Option<String>,
    pub min_rating: Option<f64>,
    pub sort_by: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// Row struct for marketplace product queries (joins supplier_products with businesses).
#[derive(Debug, sqlx::FromRow)]
struct MarketplaceProductRow {
    id: Uuid,
    business_id: Uuid,
    name: String,
    description: Option<String>,
    category: Option<String>,
    price: Option<rust_decimal::Decimal>,
    unit: Option<String>,
    min_order: Option<i32>,
    delivery_areas: Option<Vec<String>>,
    is_active: Option<bool>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    business_name: Option<String>,
    #[sqlx(default)]
    supplier_rating: Option<rust_decimal::Decimal>,
}

/// GET /api/v1/b2b/marketplace — PUBLIC. Browse active supplier products with filters and sorting.
pub async fn marketplace(
    State(s): State<AppState>,
    Query(qs): Query<MarketplaceQuery>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_marketplace")?;

    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let supplier_types = "'supplier','distributor','wholesaler','farm','association'";
    let mut wheres: Vec<String> = vec![
        "sp.is_active = true".to_string(),
        format!("b.business_type IN ({})", supplier_types),
        "b.is_active = COALESCE(b.is_active, true)".to_string(),
    ];

    // Dynamic parameter binding — track param index
    let mut param_idx = 0u32;

    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            param_idx += 1;
            wheres.push(format!("sp.category ILIKE '%' || ${} || '%'", param_idx));
        }
    }

    if let Some(ref q) = qs.search {
        if !q.is_empty() {
            param_idx += 1;
            wheres.push(format!(
                "(sp.name ILIKE '%' || ${} || '%' OR COALESCE(sp.description,'') ILIKE '%' || ${} || '%' OR b.name ILIKE '%' || ${} || '%')",
                param_idx, param_idx, param_idx
            ));
        }
    }

    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            param_idx += 1;
            wheres.push(format!("${} = ANY(sp.delivery_areas)", param_idx));
        }
    }

    if qs.min_rating.is_some() {
        param_idx += 1;
        wheres.push(format!("COALESCE(sos.avg_rating, 0) >= ${}", param_idx));
    }

    let where_clause = format!("WHERE {}", wheres.join(" AND "));

    // Determine sort order
    let order = match qs.sort_by.as_deref() {
        Some("price_desc") => "COALESCE(sp.price, 0) DESC",
        Some("rating") => "COALESCE(sos.avg_rating, 0) DESC",
        Some("newest") => "sp.created_at DESC",
        _ => "COALESCE(sp.price, 0) ASC", // price_asc (default)
    };

    // Count total
    let mut count_sql = format!(
        r#"SELECT COUNT(*) FROM supplier_products sp
           JOIN businesses b ON b.id = sp.business_id
           LEFT JOIN supplier_order_stats sos ON sos.supplier_business_id = sp.business_id
           {}"#,
        where_clause
    );
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            count_q = count_q.bind(cat);
        }
    }
    if let Some(ref q) = qs.search {
        if !q.is_empty() {
            count_q = count_q.bind(q);
        }
    }
    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            count_q = count_q.bind(area);
        }
    }
    if let Some(mr) = qs.min_rating {
        count_q = count_q.bind(mr);
    }
    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    // Fetch products
    let data_sql = format!(
        r#"SELECT sp.id, sp.business_id, sp.name, sp.description, sp.category,
                  sp.price, sp.unit, sp.min_order, sp.delivery_areas, sp.is_active, sp.created_at,
                  b.name as business_name,
                  COALESCE(sos.avg_rating, 0) as supplier_rating
           FROM supplier_products sp
           JOIN businesses b ON b.id = sp.business_id
           LEFT JOIN supplier_order_stats sos ON sos.supplier_business_id = sp.business_id
           {} ORDER BY {} LIMIT {} OFFSET {}"#,
        where_clause, order, per_page, offset
    );

    let mut data_q = sqlx::query_as::<_, MarketplaceProductRow>(&data_sql);
    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            data_q = data_q.bind(cat);
        }
    }
    if let Some(ref q) = qs.search {
        if !q.is_empty() {
            data_q = data_q.bind(q);
        }
    }
    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            data_q = data_q.bind(area);
        }
    }
    if let Some(mr) = qs.min_rating {
        data_q = data_q.bind(mr);
    }

    let rows = data_q.fetch_all(&s.db).await?;

    let products: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "name": r.name,
                "description": r.description,
                "price": r.price,
                "unit": r.unit,
                "min_order": r.min_order,
                "category": r.category,
                "delivery_areas": r.delivery_areas,
                "supplier_business_id": r.business_id,
                "supplier_name": r.business_name,
                "supplier_rating": r.supplier_rating,
                "created_at": r.created_at,
            })
        })
        .collect();

    Ok(Json(
        json!({"products": products, "page": page, "per_page": per_page, "total": total}),
    ))
}

/// GET /api/v1/b2b/suppliers/:id/detail — PUBLIC. View supplier profile with products.
pub async fn supplier_detail(
    State(s): State<AppState>,
    Path(supplier_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_marketplace")?;

    let supplier = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            String,
            Option<Uuid>,
            Option<String>,
        ),
    >(
        r#"SELECT id, name, description, business_type, featured_product_id, featured_product_cta
           FROM businesses
           WHERE id = $1
             AND business_type IN ('supplier','distributor','wholesaler','farm','association')
           LIMIT 1"#,
    )
    .bind(supplier_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Supplier not found".into()))?;

    let (biz_id, name, description, business_type, featured_id, featured_cta) = supplier;

    // Get delivery_areas from supplier_fields JSONB
    let supplier_fields: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT supplier_fields FROM businesses WHERE id = $1")
            .bind(biz_id)
            .fetch_optional(&s.db)
            .await?
            .flatten();

    let delivery_areas: Vec<String> = supplier_fields
        .as_ref()
        .and_then(|v| v.get("delivery_areas"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Active product count
    let product_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM supplier_products WHERE business_id = $1 AND is_active = true",
    )
    .bind(biz_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Average rating from supplier_order_stats
    let avg_rating: rust_decimal::Decimal = sqlx::query_scalar(
        "SELECT COALESCE(avg_rating, 0) FROM supplier_order_stats WHERE supplier_business_id = $1",
    )
    .bind(biz_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(rust_decimal::Decimal::ZERO);

    // Featured product info if set
    let featured_product = if let Some(fid) = featured_id {
        let fp = sqlx::query_as::<_, (Uuid, String, Option<rust_decimal::Decimal>, Option<String>)>(
            "SELECT id, name, price, unit FROM supplier_products WHERE id = $1 AND is_active = true"
        )
        .bind(fid)
        .fetch_optional(&s.db)
        .await?;
        fp.map(|(pid, pname, pprice, punit)| {
            json!({
                "id": pid,
                "name": pname,
                "price": pprice,
                "unit": punit,
                "cta_text": featured_cta.unwrap_or_else(|| "Featured Product".to_string()),
            })
        })
    } else {
        None
    };

    // Top 12 active products
    let products = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Option<String>,
            Option<String>,
            Option<rust_decimal::Decimal>,
            Option<String>,
            Option<i32>,
        ),
    >(
        "SELECT id, name, description, category, price, unit, min_order \
         FROM supplier_products WHERE business_id = $1 AND is_active = true \
         ORDER BY created_at DESC LIMIT 12",
    )
    .bind(biz_id)
    .fetch_all(&s.db)
    .await?;

    let product_list: Vec<serde_json::Value> = products
        .into_iter()
        .map(|p| {
            json!({
                "id": p.0,
                "name": p.1,
                "description": p.2,
                "category": p.3,
                "price": p.4,
                "unit": p.5,
                "min_order": p.6,
            })
        })
        .collect();

    Ok(Json(json!({
        "supplier": {
            "id": biz_id,
            "name": name,
            "description": description,
            "business_type": business_type,
            "delivery_areas": delivery_areas,
            "featured_product": featured_product,
        },
        "products": product_list,
        "product_count": product_count,
        "avg_rating": avg_rating,
    })))
}

// ── Phase 3c: Product CSV Import/Export ──

/// GET /api/v1/b2b/products/export — export supplier's products as CSV
pub async fn export_products(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let rows = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<rust_decimal::Decimal>,
            Option<String>,
            Option<i32>,
            Option<Vec<String>>,
            Option<bool>,
        ),
    >(
        "SELECT name, description, category, price, unit, min_order, delivery_areas, is_active \
         FROM supplier_products WHERE business_id = $1 ORDER BY name ASC",
    )
    .bind(biz_id)
    .fetch_all(&s.db)
    .await?;

    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(&[
        "name",
        "description",
        "category",
        "price",
        "unit",
        "min_order",
        "delivery_areas",
        "is_active",
    ])
    .map_err(|e| AppError::Internal(format!("CSV write error: {}", e)))?;

    for row in &rows {
        let areas = row.6.as_ref().map(|a| a.join("; ")).unwrap_or_default();
        let price_str = row.3.map(|p| p.to_string()).unwrap_or_default();
        let min_order_str = row.5.map(|m| m.to_string()).unwrap_or_default();
        let is_active_str = if row.7.unwrap_or(true) {
            "true"
        } else {
            "false"
        };

        wtr.write_record(&[
            &row.0,
            row.1.as_deref().unwrap_or(""),
            row.2.as_deref().unwrap_or(""),
            &price_str,
            row.4.as_deref().unwrap_or(""),
            &min_order_str,
            &areas,
            is_active_str,
        ])
        .map_err(|e| AppError::Internal(format!("CSV write error: {}", e)))?;
    }

    let data = String::from_utf8(
        wtr.into_inner()
            .map_err(|e| AppError::Internal(format!("CSV flush error: {}", e)))?,
    )
    .map_err(|e| AppError::Internal(format!("CSV encoding error: {}", e)))?;

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=products.csv",
            ),
        ],
        data,
    ))
}

/// POST /api/v1/b2b/products/import — import products from CSV file upload
pub async fn import_products(
    State(s): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_supplier_business(&s.db, user_id).await?;

    let mut csv_content: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|n| n.to_string()).unwrap_or_default();
        if name == "file" {
            let bytes = field.bytes().await.map_err(|e| {
                AppError::BadRequest(format!("Failed to read uploaded file: {}", e))
            })?;
            csv_content = Some(String::from_utf8_lossy(&bytes).to_string());
            break;
        }
    }

    let csv_data = csv_content.ok_or_else(|| {
        AppError::BadRequest("No file uploaded. Use field name 'file'.".to_string())
    })?;

    let mut reader = csv::Reader::from_reader(csv_data.as_bytes());
    let headers_csv = reader
        .headers()
        .map_err(|e| AppError::BadRequest(format!("Failed to read CSV headers: {}", e)))?;

    // Validate expected headers
    let expected = [
        "name",
        "description",
        "category",
        "price",
        "unit",
        "min_order",
        "delivery_areas",
        "is_active",
    ];
    let header_strings: Vec<&str> = headers_csv.iter().collect();
    if header_strings != expected {
        return Err(AppError::BadRequest(format!(
            "Invalid CSV headers. Expected: {:?}, got: {:?}",
            expected, header_strings
        )));
    }

    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut errors: Vec<String> = Vec::new();
    let mut row_num = 1u32; // header is row 0

    for result in reader.records() {
        row_num += 1;
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                skipped += 1;
                errors.push(format!("row {}: parse error: {}", row_num, e));
                continue;
            }
        };

        if record.len() < 8 {
            skipped += 1;
            errors.push(format!("row {}: not enough columns", row_num));
            continue;
        }

        let name = record.get(0).unwrap_or("").trim().to_string();
        if name.is_empty() {
            skipped += 1;
            errors.push(format!("row {}: missing name", row_num));
            continue;
        }

        let description = record
            .get(1)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let category = record
            .get(2)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let price: Option<f64> = record.get(3).and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                s.parse::<f64>().ok()
            }
        });

        let unit = record
            .get(4)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let min_order: Option<i32> = record.get(5).and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                s.parse::<i32>().ok()
            }
        });

        let delivery_areas: Option<Vec<String>> = record.get(6).and_then(|s| {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(
                    s.split(';')
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect(),
                )
            }
        });

        let is_active: bool = record
            .get(7)
            .map(|s| s.trim().to_lowercase())
            .map(|s| s == "true" || s == "1" || s == "yes")
            .unwrap_or(true);

        let id = Uuid::new_v4();
        let result = sqlx::query(
            "INSERT INTO supplier_products (id, business_id, name, description, category, price, unit, min_order, delivery_areas, is_active) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        )
        .bind(id)
        .bind(biz_id)
        .bind(&name)
        .bind(&description)
        .bind(&category)
        .bind(price)
        .bind(&unit)
        .bind(min_order)
        .bind(&delivery_areas)
        .bind(is_active)
        .execute(&s.db)
        .await;

        match result {
            Ok(_) => imported += 1,
            Err(e) => {
                skipped += 1;
                errors.push(format!("row {}: db error: {}", row_num, e));
            }
        }
    }

    Ok(Json(json!({
        "imported": imported,
        "skipped": skipped,
        "errors": errors
    })))
}

// ── Phase 3d: Supplier Discovery ──

#[derive(Debug, Deserialize)]
pub struct DiscoverSuppliersQuery {
    pub category: Option<String>,
    pub business_type: Option<String>,
    pub delivery_area: Option<String>,
    pub search: Option<String>,
    pub min_products: Option<i64>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
struct SupplierDiscoveryRow {
    business_id: Uuid,
    name: String,
    description: Option<String>,
    business_type: String,
    delivery_areas: Option<String>,
    product_count: i64,
    avg_rating: Option<rust_decimal::Decimal>,
    review_count: i64,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GET /api/v1/b2b/discover — PUBLIC. Discover suppliers with filters and pagination.
/// Returns supplier businesses with product count, ratings, and delivery area info.
pub async fn discover_suppliers(
    State(s): State<AppState>,
    Query(qs): Query<DiscoverSuppliersQuery>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_marketplace")?;

    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let supplier_types = "'supplier','distributor','wholesaler','farm','association'";
    let mut wheres: Vec<String> = vec![
        format!("b.business_type IN ({})", supplier_types),
        "b.is_active = COALESCE(b.is_active, true)".to_string(),
    ];
    let mut having_clauses: Vec<String> = Vec::new();
    let mut param_idx = 0u32;

    if let Some(ref bt) = qs.business_type {
        if !bt.is_empty() {
            param_idx += 1;
            wheres.push(format!("b.business_type = ${}", param_idx));
        }
    }

    if let Some(ref search) = qs.search {
        if !search.is_empty() {
            param_idx += 1;
            wheres.push(format!(
                "(b.name ILIKE '%' || ${0} || '%' OR COALESCE(b.description,'') ILIKE '%' || ${0} || '%')",
                0
            ));
            // Replace the $0 placeholder with the actual param index
            wheres
                .last_mut()
                .map(|w| *w = w.replace("$0", &param_idx.to_string()));
        }
    }

    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            param_idx += 1;
            // Check both supplier_fields->>'delivery_areas' and any text array columns
            wheres.push(format!(
                "(COALESCE(b.supplier_fields->>'delivery_areas','') ILIKE '%' || ${0} || '%')",
                0
            ));
            wheres
                .last_mut()
                .map(|w| *w = w.replace("$0", &param_idx.to_string()));
        }
    }

    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            param_idx += 1;
            // Filter via subquery on supplier_products category
            having_clauses.push(format!("COALESCE(product_count, 0) > 0"));
            wheres.push(format!(
                "EXISTS (SELECT 1 FROM supplier_products sp WHERE sp.business_id = b.id AND sp.is_active = true AND sp.category ILIKE '%' || ${0} || '%')",
                0
            ));
            wheres
                .last_mut()
                .map(|w| *w = w.replace("$0", &param_idx.to_string()));
        }
    }

    if let Some(min_p) = qs.min_products {
        if min_p > 0 {
            having_clauses.push(format!("COALESCE(product_count, 0) >= {}", min_p));
        }
    }

    let where_clause = format!("WHERE {}", wheres.join(" AND "));
    let having_clause = if having_clauses.is_empty() {
        String::new()
    } else {
        format!("HAVING {}", having_clauses.join(" AND "))
    };

    // Count total matching suppliers
    let count_sql = format!(
        r#"SELECT COUNT(*) FROM (
            SELECT b.id,
                   COUNT(sp.id) FILTER (WHERE sp.is_active = true) as product_count
            FROM businesses b
            LEFT JOIN supplier_products sp ON sp.business_id = b.id
            {}
            GROUP BY b.id
            {}
        ) sub"#,
        where_clause, having_clause
    );

    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(ref bt) = qs.business_type {
        if !bt.is_empty() {
            count_q = count_q.bind(bt);
        }
    }
    if let Some(ref search) = qs.search {
        if !search.is_empty() {
            count_q = count_q.bind(search);
        }
    }
    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            count_q = count_q.bind(area);
        }
    }
    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            count_q = count_q.bind(cat);
        }
    }
    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    // Fetch suppliers with product count and ratings
    let data_sql = format!(
        r#"SELECT
            b.id as business_id,
            b.name,
            b.description,
            COALESCE(b.business_type, 'supplier') as business_type,
            COALESCE(b.supplier_fields->>'delivery_areas', '[]') as delivery_areas,
            COUNT(sp.id) FILTER (WHERE sp.is_active = true) as product_count,
            COALESCE(sos.avg_rating, 0) as avg_rating,
            COALESCE(sos.total_orders, 0) as review_count,
            b.created_at
        FROM businesses b
        LEFT JOIN supplier_products sp ON sp.business_id = b.id
        LEFT JOIN supplier_order_stats sos ON sos.supplier_business_id = b.id
        {}
        GROUP BY b.id, sos.avg_rating, sos.total_orders
        {}
        ORDER BY product_count DESC, avg_rating DESC
        LIMIT {} OFFSET {}"#,
        where_clause, having_clause, per_page, offset
    );

    let mut data_q = sqlx::query_as::<_, SupplierDiscoveryRow>(&data_sql);
    if let Some(ref bt) = qs.business_type {
        if !bt.is_empty() {
            data_q = data_q.bind(bt);
        }
    }
    if let Some(ref search) = qs.search {
        if !search.is_empty() {
            data_q = data_q.bind(search);
        }
    }
    if let Some(ref area) = qs.delivery_area {
        if !area.is_empty() {
            data_q = data_q.bind(area);
        }
    }
    if let Some(ref cat) = qs.category {
        if !cat.is_empty() {
            data_q = data_q.bind(cat);
        }
    }

    let rows = data_q.fetch_all(&s.db).await?;
    let suppliers: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let areas: Vec<String> = r
                .delivery_areas
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            let avg = r
                .avg_rating
                .map(|d| d.to_string().parse::<f64>().unwrap_or(0.0))
                .unwrap_or(0.0);

            json!({
                "business_id": r.business_id,
                "name": r.name,
                "description": r.description,
                "business_type": r.business_type,
                "delivery_areas": areas,
                "product_count": r.product_count,
                "avg_rating": avg,
                "review_count": r.review_count,
                "created_at": r.created_at,
            })
        })
        .collect();

    Ok(Json(json!({
        "suppliers": suppliers,
        "page": page,
        "per_page": per_page,
        "total": total,
    })))
}

// ── Notifications (Phase 3e) ──

#[derive(Debug, Deserialize)]
pub struct NotificationListQuery {
    pub is_read: Option<bool>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// GET /api/v1/b2b/notifications — list notifications for the authenticated business
pub async fn my_notifications(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(qs): Query<NotificationListQuery>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let page = qs.page.unwrap_or(1).max(1);
    let per_page = qs.per_page.unwrap_or(20).min(100);
    let offset = (page - 1) * per_page;

    let mut wheres = vec!["business_id = $1".to_string()];
    let mut param_idx = 1u32;

    if let Some(read) = qs.is_read {
        param_idx += 1;
        wheres.push(format!("is_read = ${}", param_idx));
    }

    let where_clause = format!("WHERE {}", wheres.join(" AND "));

    // Count
    let count_sql = format!("SELECT COUNT(*) FROM b2b_notifications {}", where_clause);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(biz_id);
    if qs.is_read.is_some() {
        count_q = count_q.bind(qs.is_read.unwrap());
    }
    let total = count_q.fetch_one(&s.db).await.unwrap_or(0);

    // Unread count
    let unread_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM b2b_notifications WHERE business_id = $1 AND is_read = false",
    )
    .bind(biz_id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    // Fetch notifications
    let data_sql = format!(
        "SELECT id, business_id, type, title, body, related_order_id, related_message_id, is_read, created_at \
         FROM b2b_notifications {} ORDER BY is_read ASC, created_at DESC LIMIT {} OFFSET {}",
        where_clause, per_page, offset
    );

    let mut data_q = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            String,
            Option<String>,
            Option<Uuid>,
            Option<Uuid>,
            bool,
            chrono::DateTime<chrono::Utc>,
        ),
    >(&data_sql)
    .bind(biz_id);
    if qs.is_read.is_some() {
        data_q = data_q.bind(qs.is_read.unwrap());
    }

    let rows = data_q.fetch_all(&s.db).await?;
    let notifications: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.0,
                "business_id": r.1,
                "type": r.2,
                "title": r.3,
                "body": r.4,
                "related_order_id": r.5,
                "related_message_id": r.6,
                "is_read": r.7,
                "created_at": r.8
            })
        })
        .collect();

    Ok(Json(json!({
        "notifications": notifications,
        "page": page,
        "per_page": per_page,
        "total": total,
        "unread_count": unread_count
    })))
}

/// PUT /api/v1/b2b/notifications/:id/read — mark a single notification as read
pub async fn mark_notification_read(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(notification_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let owner =
        sqlx::query_scalar::<_, Uuid>("SELECT business_id FROM b2b_notifications WHERE id = $1")
            .bind(notification_id)
            .fetch_optional(&s.db)
            .await?
            .ok_or_else(|| AppError::NotFound("Notification not found".into()))?;

    if owner != biz_id {
        return Err(AppError::Forbidden(
            "You can only mark your own notifications as read".into(),
        ));
    }

    sqlx::query("UPDATE b2b_notifications SET is_read = true WHERE id = $1")
        .bind(notification_id)
        .execute(&s.db)
        .await?;

    Ok(Json(json!({"success": true})))
}

/// PUT /api/v1/b2b/notifications/read-all — mark all notifications as read
pub async fn mark_all_read(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let b2b_config = get_b2b_config(&s.db, get_first_directory_id(&s.db).await).await;
    require_b2b_feature(&b2b_config, "b2b_orders")?;

    let user_id = extract_user_id(&headers, &s)?;
    let biz_id = resolve_buyer_business(&s.db, user_id).await?;

    let result = sqlx::query(
        "UPDATE b2b_notifications SET is_read = true WHERE business_id = $1 AND is_read = false",
    )
    .bind(biz_id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({"marked_read": result.rows_affected()})))
}

// ── Notification Helpers ──

/// Create a notification for a business. Used internally by other handlers.
async fn create_notification(
    db: &sqlx::PgPool,
    business_id: Uuid,
    ntype: &str,
    title: &str,
    body: Option<&str>,
    order_id: Option<Uuid>,
    message_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO b2b_notifications (business_id, type, title, body, related_order_id, related_message_id) \
         VALUES ($1, $2, $3, $4, $5, $6)"
    )
    .bind(business_id)
    .bind(ntype)
    .bind(title)
    .bind(body)
    .bind(order_id)
    .bind(message_id)
    .execute(db)
    .await?;
    Ok(())
}

// ── Helpers ──

/// Resolve the supplier's business_id from the authenticated user's identity.
/// For visitor accounts (supplier portal), looks up the business by matching email.
/// For regular users with claimed_businesses, looks up via that join.
pub async fn resolve_supplier_business(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Uuid> {
    // Try visitor_account_id in claimed_businesses (supplier portal registrations)
    let biz_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT cb.business_id
           FROM claimed_businesses cb
           JOIN businesses b ON b.id = cb.business_id
           WHERE cb.visitor_account_id = $1
             AND b.business_type IN ('supplier','distributor','wholesaler','farm','association')
           ORDER BY cb.created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    if let Some(bid) = biz_id {
        return Ok(bid);
    }

    // Try claimed_businesses join (for users table accounts with user_id)
    let biz_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT cb.business_id
           FROM claimed_businesses cb
           JOIN businesses b ON b.id = cb.business_id
           WHERE cb.user_id = $1
             AND b.business_type IN ('supplier','distributor','wholesaler','farm','association')
           ORDER BY cb.created_at DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    if let Some(bid) = biz_id {
        return Ok(bid);
    }

    // Fallback: find the visitor_account and match by email in businesses table
    let email = sqlx::query_scalar::<_, String>("SELECT email FROM visitor_accounts WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await?;

    if let Some(ref em) = email {
        let biz_id = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM businesses
               WHERE email = $1
                 AND business_type IN ('supplier','distributor','wholesaler','farm','association')
               ORDER BY created_at DESC
               LIMIT 1"#,
        )
        .bind(em)
        .fetch_optional(db)
        .await?;

        if let Some(bid) = biz_id {
            return Ok(bid);
        }
    }

    Err(AppError::NotFound(
        "No supplier business linked to your account. Register as a supplier first.".into(),
    ))
}

/// Resolve ANY claimed business for the authenticated user (no business_type filter).
/// Used for buyers who may be any business type placing orders or sending messages.
pub async fn resolve_buyer_business(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<Uuid> {
    // Try visitor_account_id in claimed_businesses (any business type)
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

    // Try claimed_businesses join via user_id (for users table accounts)
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

    // Fallback: find the visitor_account and match by email in businesses table
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
