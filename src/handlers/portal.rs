//! Portal handlers for Business Owner Dashboard and Visitor Accounts.
//! BL13 — Memberships & Subscriber Dashboard for ZaarHub.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::AppState;
use crate::auth::models::Claims;
use crate::auth::middleware::{create_token, is_admin};
use crate::error::{AppError, ApiResult};

// ── Data Types ──

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ClaimedBusinessRow {
    pub id: Uuid,
    pub business_id: Uuid,
    pub owner_email: String,
    pub owner_name: Option<String>,
    pub owner_phone: Option<String>,
    pub user_id: Option<Uuid>,
    pub is_active: Option<bool>,
    pub created_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct BusinessProfile {
    pub id: Uuid,
    pub name: String,
    pub category: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub images: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct BusinessPortalResponse {
    pub claimed_businesses: Vec<BusinessWithSubscription>,
}

#[derive(Debug, Serialize)]
pub struct BusinessWithSubscription {
    pub claim: ClaimedBusinessRow,
    pub business: BusinessProfile,
    pub subscription: Option<BusinessSubscriptionInfo>,
}

#[derive(Debug, Serialize)]
pub struct BusinessSubscriptionInfo {
    pub id: Option<Uuid>,
    pub tier_id: Option<Uuid>,
    pub tier_name: Option<String>,
    pub status: Option<String>,
    pub billing_cycle: Option<String>,
    pub price_paid: Option<rust_decimal::Decimal>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub auto_renew: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct VisitorRegisterRequest {
    pub email: String,
    pub password: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct VisitorLoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VisitorAccount {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub directory_id: Option<Uuid>,
    pub is_active: bool,
    pub last_login_at: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct VisitorAccountResponse {
    pub id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub directory_id: Option<Uuid>,
    pub is_active: bool,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct FeatureConfigUpdate {
    #[serde(default)]
    pub deals: Option<bool>,
    #[serde(default)]
    pub blogging: Option<bool>,
    #[serde(default)]
    pub community_posts: Option<bool>,
    #[serde(default)]
    pub b2b_marketplace: Option<bool>,
    #[serde(default)]
    pub visitor_accounts: Option<bool>,
    #[serde(default)]
    pub gamification: Option<bool>,
}

// ── Portal: Business Profile ──

/// GET /api/v1/portal/business/profile
/// Returns the logged-in business owner's claimed businesses + subscription status
pub async fn business_profile(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // Find claimed businesses for this user_id OR owner_email
    let claims_rows = sqlx::query_as::<_, ClaimedBusinessRow>(
        r#"SELECT id, business_id, owner_email, owner_name, owner_phone, user_id, is_active, created_at
           FROM claimed_businesses
           WHERE user_id = $1
           ORDER BY created_at DESC"#
    )
    .bind(user_id)
    .fetch_all(&s.db)
    .await?;

    let mut businesses_with_subs = Vec::new();

    for claim in &claims_rows {
        // Fetch business details (with category name from join)
        let biz = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<Value>)>(
            r#"SELECT b.id, b.name, dc.name as category, b.city, b.state, b.phone, b.website, b.images
               FROM businesses b
               LEFT JOIN directory_categories dc ON dc.id = b.category_id
               WHERE b.id = $1"#
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        let business_profile = match biz {
            Some((id, name, category, city, state, phone, website, images)) => BusinessProfile {
                id, name, category, city, state, phone, website, images,
            },
            None => continue,
        };

        // Fetch subscription info
        let sub = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<String>, Option<String>, Option<String>, Option<rust_decimal::Decimal>, Option<chrono::NaiveDate>, Option<chrono::NaiveDate>, Option<bool>)>(
            r#"SELECT bs.id, bs.tier_id, pt.name, bs.status, bs.billing_cycle, bs.price_paid, bs.start_date, bs.end_date, bs.auto_renew
               FROM business_subscriptions bs
               LEFT JOIN plan_tiers pt ON pt.id = bs.tier_id
               WHERE bs.business_id = $1
               ORDER BY bs.created_at DESC
               LIMIT 1"#
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        let subscription = sub.map(|(id, tier_id, tier_name, status, billing_cycle, price_paid, start_date, end_date, auto_renew)| {
            BusinessSubscriptionInfo {
                id, tier_id, tier_name,
                status, billing_cycle, price_paid, start_date, end_date, auto_renew,
            }
        });

        businesses_with_subs.push(BusinessWithSubscription {
            claim: claim.clone(),
            business: business_profile,
            subscription,
        });
    }

    Ok(Json(json!(BusinessPortalResponse {
        claimed_businesses: businesses_with_subs,
    })))
}

// ── Visitor Account Routes ──

/// POST /api/v1/visitor/register
pub async fn visitor_register(
    State(s): State<AppState>,
    Json(req): Json<VisitorRegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.email.is_empty() || req.password.is_empty() {
        return Err(AppError::Validation("Email and password are required".to_string()));
    }
    if req.password.len() < 6 {
        return Err(AppError::Validation("Password must be at least 6 characters".to_string()));
    }

    // Check if visitor already exists
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_accounts WHERE email = $1"
    )
    .bind(&req.email)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    if existing > 0 {
        return Err(AppError::Duplicate("A visitor account with this email already exists".to_string()));
    }

    // Hash password with argon2
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

    // If directory_id is provided, validate it exists
    if let Some(dir_id) = req.directory_id {
        let dir_exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM directories WHERE id = $1"
        )
        .bind(dir_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);

        if dir_exists == 0 {
            return Err(AppError::Validation("Directory not found".to_string()));
        }
    }

    // Create visitor account
    let visitor = sqlx::query_as::<_, VisitorAccount>(
        "INSERT INTO visitor_accounts (email, password_hash, name, phone, directory_id) VALUES ($1, $2, $3, $4, $5) RETURNING *"
    )
    .bind(&req.email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind(&req.phone)
    .bind(req.directory_id)
    .fetch_one(&s.db)
    .await?;

    // Update last_login
    sqlx::query("UPDATE visitor_accounts SET last_login_at = NOW() WHERE id = $1")
        .bind(visitor.id)
        .execute(&s.db)
        .await?;

    // Generate JWT with role=visitor
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

    Ok((StatusCode::CREATED, Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": s.config.jwt_access_expiry,
        "visitor": VisitorAccountResponse {
            id: visitor.id,
            email: visitor.email,
            name: visitor.name,
            phone: visitor.phone,
            directory_id: visitor.directory_id,
            is_active: visitor.is_active,
            created_at: visitor.created_at,
        },
    }))))
}

/// POST /api/v1/visitor/login
pub async fn visitor_login(
    State(s): State<AppState>,
    Json(req): Json<VisitorLoginRequest>,
) -> ApiResult<impl IntoResponse> {
    use argon2::{
        Argon2, PasswordHash, PasswordVerifier,
    };

    if req.email.is_empty() || req.password.is_empty() {
        return Err(AppError::Validation("Email and password are required".to_string()));
    }

    let visitor = sqlx::query_as::<_, VisitorAccount>(
        "SELECT * FROM visitor_accounts WHERE email = $1"
    )
    .bind(&req.email)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| {
        tracing::warn!("Visitor login failed: user not found for {}", &req.email);
        AppError::InvalidCredentials
    })?;

    if !visitor.is_active {
        return Err(AppError::Forbidden("Account is deactivated".to_string()));
    }

    // Verify password
    let parsed_hash = PasswordHash::new(&visitor.password_hash)
        .map_err(|e| AppError::Hash(e.to_string()))?;
    let argon2 = Argon2::default();
    argon2
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::InvalidCredentials)?;

    // Update last_login
    sqlx::query("UPDATE visitor_accounts SET last_login_at = NOW() WHERE id = $1")
        .bind(visitor.id)
        .execute(&s.db)
        .await?;

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

    Ok(Json(json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": s.config.jwt_access_expiry,
        "visitor": VisitorAccountResponse {
            id: visitor.id,
            email: visitor.email,
            name: visitor.name,
            phone: visitor.phone,
            directory_id: visitor.directory_id,
            is_active: visitor.is_active,
            created_at: visitor.created_at,
        },
    })))
}

/// GET /api/v1/visitor/profile
/// Returns visitor profile with saved deals, favorites, badges
pub async fn visitor_profile(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let visitor_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let visitor = sqlx::query_as::<_, VisitorAccount>(
        "SELECT * FROM visitor_accounts WHERE id = $1"
    )
    .bind(visitor_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("Visitor not found".to_string()))?;

    // Get saved deals — deals where this visitor claimed/flagged
    let saved_deals = sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>)>(
        r#"SELECT d.id, d.title, d.description, d.discount_value, d.image_url
           FROM deals d
           JOIN deal_claims dc ON dc.deal_id = d.id
           WHERE dc.visitor_account_id = $1 OR dc.email = $2
           ORDER BY dc.created_at DESC
           LIMIT 20"#
    )
    .bind(visitor_id)
    .bind(&visitor.email)
    .fetch_all(&s.db)
    .await
    .unwrap_or_default();

    Ok(Json(json!({
        "visitor": {
            "id": visitor.id,
            "email": visitor.email,
            "name": visitor.name,
            "phone": visitor.phone,
            "directory_id": visitor.directory_id,
            "is_active": visitor.is_active,
            "created_at": visitor.created_at,
        },
        "saved_deals": saved_deals.into_iter().map(|(id, title, desc, discount, img)| {
            json!({
                "id": id,
                "title": title,
                "description": desc,
                "discount_value": discount,
                "image_url": img,
            })
        }).collect::<Vec<_>>(),
        "favorites": [],
        "badges": [],
    })))
}

// ── Directory Feature Config ──

/// GET /api/v1/directories/:id/features
/// Public — returns the feature config for a directory
pub async fn get_directory_features(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let feature_config: Option<Value> = sqlx::query_scalar(
        r#"SELECT feature_config FROM directories WHERE id = $1"#
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await?
    .flatten();

    match feature_config {
        Some(config) => Ok(Json(json!({
            "directory_id": id,
            "feature_config": config,
        }))),
        None => Err(AppError::NotFound("Directory not found".to_string())),
    }
}

/// PUT /api/v1/directories/:id/features
/// Admin-only — updates the feature config for a directory
pub async fn update_directory_features(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<FeatureConfigUpdate>,
) -> ApiResult<impl IntoResponse> {
    if !is_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Check directory exists
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM directories WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&s.db)
    .await?;

    if exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    // Build the new config from current + updates
    let current_config: Value = sqlx::query_scalar(
        r#"SELECT COALESCE(feature_config, '{}'::jsonb) FROM directories WHERE id = $1"#
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(json!({}));

    let mut config = current_config.as_object().cloned().unwrap_or_default();

    if let Some(v) = req.deals { config.insert("deals".to_string(), json!(v)); }
    if let Some(v) = req.blogging { config.insert("blogging".to_string(), json!(v)); }
    if let Some(v) = req.community_posts { config.insert("community_posts".to_string(), json!(v)); }
    if let Some(v) = req.b2b_marketplace { config.insert("b2b_marketplace".to_string(), json!(v)); }
    if let Some(v) = req.visitor_accounts { config.insert("visitor_accounts".to_string(), json!(v)); }
    if let Some(v) = req.gamification { config.insert("gamification".to_string(), json!(v)); }

    let new_config = Value::Object(config);

    sqlx::query(
        r#"UPDATE directories SET feature_config = $1, updated_at = NOW() WHERE id = $2"#
    )
    .bind(&new_config)
    .bind(id)
    .execute(&s.db)
    .await?;

    Ok(Json(json!({
        "directory_id": id,
        "feature_config": new_config,
    })))
}
