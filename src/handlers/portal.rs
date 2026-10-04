//! Portal handlers for Business Owner Dashboard and Visitor Accounts.
//! BL13 — Memberships & Subscriber Dashboard for ZaarHub.

use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::{create_token, is_super_admin, verify_token};
use crate::auth::models::Claims;
use crate::error::{ApiResult, AppError};
use crate::AppState;

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

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BusinessVerificationRow {
    pub status: Option<String>,
    pub verified_at: Option<chrono::DateTime<Utc>>,
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
    pub verification: Option<BusinessVerificationRow>,
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
    /// Card B48 — an optional referral code from `?ref=CODE`. When present and live, the
    /// pending referral row for that code is attached to this new account.
    #[serde(default)]
    pub referral_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VisitorLoginRequest {
    pub email: String,
    pub password: String,
}

/// PUT /api/v1/visitor/profile body. Every field is optional; a missing or blank field is
/// left unchanged, so the Save Profile button can send just what it edits. (t_4fd9fd2a.)
#[derive(Debug, Deserialize)]
pub struct UpdateVisitorProfileRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
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
    // ZaarHub network visibility per-directory
    #[serde(default)]
    pub network_visible: Option<bool>,
    #[serde(default)]
    pub homepage_featured: Option<bool>,
    #[serde(default)]
    pub show_deals: Option<bool>,
    #[serde(default)]
    pub show_events: Option<bool>,
    #[serde(default)]
    pub show_reviews: Option<bool>,
    #[serde(default)]
    pub show_activity: Option<bool>,
    // B90: per-directory hiring / quality guarantee (Angie's-List style public trust statement).
    #[serde(default)]
    pub guarantee_enabled: Option<bool>,
    #[serde(default)]
    pub guarantee_title: Option<String>,
    #[serde(default)]
    pub guarantee_text: Option<String>,
    // Generic feature_config passthrough. The admin B2B-toggles card PUTs the whole
    // `{feature_config:{...}}` object; without this field serde dropped it and the save was a
    // silent no-op, so b2b_marketplace etc. were never persisted.
    #[serde(default)]
    pub feature_config: Option<serde_json::Map<String, Value>>,
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
    // We need the user's email from the claims (or from visitor_accounts lookup)
    let visitor_email: Option<String> =
        sqlx::query_scalar("SELECT email FROM visitor_accounts WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&s.db)
            .await?
            .flatten();

    let claims_rows = sqlx::query_as::<_, ClaimedBusinessRow>(
        r#"SELECT id, business_id, owner_email, owner_name, owner_phone, user_id, is_active, created_at
           FROM claimed_businesses
           WHERE user_id = $1
              OR ($2 IS NOT NULL AND owner_email = $2)
           ORDER BY created_at DESC"#
    )
    .bind(user_id)
    .bind(&visitor_email)
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
                id,
                name,
                category,
                city,
                state,
                phone,
                website,
                images,
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

        let subscription = sub.map(
            |(
                id,
                tier_id,
                tier_name,
                status,
                billing_cycle,
                price_paid,
                start_date,
                end_date,
                auto_renew,
            )| {
                BusinessSubscriptionInfo {
                    id,
                    tier_id,
                    tier_name,
                    status,
                    billing_cycle,
                    price_paid,
                    start_date,
                    end_date,
                    auto_renew,
                }
            },
        );

        // Fetch verification status
        let verification = sqlx::query_as::<_, BusinessVerificationRow>(
            r#"SELECT status, verified_at FROM business_verifications WHERE business_id = $1"#,
        )
        .bind(claim.business_id)
        .fetch_optional(&s.db)
        .await?;

        businesses_with_subs.push(BusinessWithSubscription {
            claim: claim.clone(),
            business: business_profile,
            subscription,
            verification,
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
        return Err(AppError::Validation(
            "Email and password are required".to_string(),
        ));
    }
    if req.password.len() < 6 {
        return Err(AppError::Validation(
            "Password must be at least 6 characters".to_string(),
        ));
    }

    // Trim + lowercase + refuse anything that is not an address, BEFORE the first SELECT: this is
    // the value the duplicate check must see and the value the INSERT will store (t_01f183b1).
    let email = crate::security::email_addr::normalize(&req.email).map_err(AppError::Validation)?;

    // Check if visitor already exists
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM visitor_accounts WHERE lower(email) = $1",
    )
    .bind(&email)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    if existing > 0 {
        return Err(AppError::Duplicate(
            "A visitor account with this email already exists".to_string(),
        ));
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
        let dir_exists =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = $1")
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
    .bind(&email)
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

    // Card B48 — attach a referral when the signup carried a code. The code's own row is
    // adopted first; once that is used, each further signup writes its own referral row, so a
    // member can refer more than one person. A bad/expired code never fails the registration.
    if let Some(code) = req
        .referral_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        let attached = sqlx::query(
            "UPDATE referrals SET referee_type = 'visitor', referee_id = $1, referee_email = $2, \
             referee_name = $3, status = 'pending', updated_at = now() \
             WHERE referral_code = $4 AND referee_id IS NULL AND status = 'pending' \
             RETURNING id",
        )
        .bind(visitor.id)
        .bind(&email)
        .bind(&req.name)
        .bind(code)
        .fetch_optional(&s.db)
        .await;

        match attached {
            Ok(Some(_)) => {}
            Ok(None) => {
                use sqlx::Row as _;
                let referrer = sqlx::query(
                    "SELECT referrer_type, referrer_id, referrer_email FROM referrals \
                     WHERE referral_code = $1 AND status <> 'expired' ORDER BY created_at ASC LIMIT 1",
                )
                .bind(code)
                .fetch_optional(&s.db)
                .await
                .ok()
                .flatten();
                match referrer {
                    Some(r) => {
                        let rtype: String = r
                            .try_get("referrer_type")
                            .unwrap_or_else(|_| "visitor".to_string());
                        let rid: Option<Uuid> = r.try_get("referrer_id").ok();
                        let remail: Option<String> = r.try_get("referrer_email").unwrap_or(None);
                        if let Some(rid) = rid {
                            if let Err(e) = sqlx::query(
                                "INSERT INTO referrals (referrer_type, referrer_id, referrer_email, \
                                 referee_type, referee_id, referee_email, referee_name, referral_code, \
                                 direction, status) \
                                 VALUES ($1, $2, $3, 'visitor', $4, $5, $6, $7, 'inbound', 'pending')",
                            )
                            .bind(&rtype)
                            .bind(rid)
                            .bind(&remail)
                            .bind(visitor.id)
                            .bind(&email)
                            .bind(&req.name)
                            .bind(code)
                            .execute(&s.db)
                            .await
                            {
                                eprintln!("[referral] signup attach failed for code {code}: {e}");
                            }
                        }
                    }
                    None => {
                        eprintln!(
                            "[referral] signup code {code} matched no live referral — skipped"
                        )
                    }
                }
            }
            Err(e) => eprintln!("[referral] signup attach failed for code {code}: {e}"),
        }
    }

    // Fire cross-platform tag sync for visitor signup (fire-and-forget)
    {
        let ts_db = s.db.clone();
        let ts_email = visitor.email.clone();
        let ts_name = visitor.name.clone();
        let ts_phone = visitor.phone.clone();
        let ts_dir_id = visitor.directory_id;
        tokio::spawn(async move {
            let (dir_slug, city) = if let Some(did) = ts_dir_id {
                let slug: Option<String> =
                    sqlx::query_scalar("SELECT slug FROM directories WHERE id = $1")
                        .bind(did)
                        .fetch_optional(&ts_db)
                        .await
                        .unwrap_or(None)
                        .flatten();
                let s = slug.unwrap_or_default();
                let c = s.replace("-", " ");
                (s, c)
            } else {
                (String::new(), String::new())
            };

            let tags = vec!["Customer".to_string(), city.clone()];
            let city_list = if city.is_empty() {
                None
            } else {
                Some(format!("{} - Subscribers", city))
            };

            crate::handlers::tag_sync::fire_tag_sync(
                &ts_db,
                ts_email,
                ts_name,
                None,
                ts_phone,
                tags,
                city_list,
                Some("subscribers".to_string()),
                Some(dir_slug),
                Some("visitor_signup".to_string()),
                None,
                None,
            );
        });
    }

    // Enrol the new shopper in the network-wide loyalty programme — native Multi-Directory code.
    // (This replaces a call into IncentiveSwift at a hardcoded localhost:8083. Loyalty is now ours;
    // the only external seam left is the onboarding/IQS survey proxy.)
    {
        let db = s.db.clone();
        let vid = visitor.id;
        tokio::spawn(async move {
            crate::handlers::loyalty_native::enroll_visitor_in_network_loyalty(&db, &vid).await;
        });
    }

    // Card B49 — award the directory's CUSTOMER signup reward (once, only if the admin has
    // switched it on). Best-effort: a failure is logged and can never fail the signup.
    {
        let db = s.db.clone();
        let vid = visitor.id;
        let dir = visitor.directory_id;
        tokio::spawn(async move {
            match crate::handlers::loyalty_native::award_signup_reward(&db, dir, &vid, "visitor")
                .await
            {
                Ok(Some(a)) if a.units > 0 => tracing::info!(
                    "[loyalty] signup reward: credited {} {} to visitor {vid}",
                    a.units,
                    a.currency_name
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("[loyalty] signup reward failed for visitor {vid}: {e}"),
            }
        });
    }

    // Generate JWT with role=visitor
    let now_ts = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: visitor.id.to_string(),
        tid: Uuid::nil().to_string(),
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
            "visitor": VisitorAccountResponse {
                id: visitor.id,
                email: visitor.email,
                name: visitor.name,
                phone: visitor.phone,
                directory_id: visitor.directory_id,
                is_active: visitor.is_active,
                created_at: visitor.created_at,
            },
        })),
    ))
}

/// POST /api/v1/visitor/login
pub async fn visitor_login(
    State(s): State<AppState>,
    Json(req): Json<VisitorLoginRequest>,
) -> ApiResult<impl IntoResponse> {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    if req.email.is_empty() || req.password.is_empty() {
        return Err(AppError::Validation(
            "Email and password are required".to_string(),
        ));
    }

    // Same trim+lowercase as the writers, with lower(email) on the column so rows stored before
    // the normalisation existed still resolve (t_01f183b1).
    let email_key = crate::security::email_addr::lookup_key(&req.email);

    let visitor = sqlx::query_as::<_, VisitorAccount>(
        "SELECT * FROM visitor_accounts WHERE lower(email) = $1",
    )
    .bind(&email_key)
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
    let parsed_hash =
        PasswordHash::new(&visitor.password_hash).map_err(|e| AppError::Hash(e.to_string()))?;
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
        tid: Uuid::nil().to_string(),
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
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    // Manually verify visitor JWT from Authorization header (route is before auth_guard)
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, &s.config.jwt_secret).map_err(|_| AppError::Unauthorized)?;

    let visitor_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let visitor =
        sqlx::query_as::<_, VisitorAccount>("SELECT * FROM visitor_accounts WHERE id = $1")
            .bind(visitor_id)
            .fetch_optional(&s.db)
            .await?
            .ok_or(AppError::NotFound("Visitor not found".to_string()))?;

    // Get saved deals — deals where this visitor claimed/flagged
    let saved_deals =
        sqlx::query_as::<_, (Uuid, String, Option<String>, Option<String>, Option<String>)>(
            // deal_claims links a claim to a visitor by email only (it has no visitor_account_id)
            // and stamps claimed_at, not created_at — the old WHERE/ORDER BY named three columns
            // that do not exist, so this read errored out on every page load and the swallow turned
            // it into a silently empty list (t_4f883b9a).
            r#"SELECT d.id, d.title, d.description, d.discount_value::text AS discount_value, d.image_url
           FROM deals d
           JOIN deal_claims dc ON dc.deal_id = d.id
           WHERE dc.visitor_email = $1
           ORDER BY dc.claimed_at DESC
           LIMIT 20"#,
        )
        .bind(&visitor.email)
        .fetch_all(&s.db)
        .await
        .unwrap_or_else(|e| {
            // Secondary read: never take the visitor dashboard down with it, but never swallow it
            // silently either — the decode mismatch this used to hide is t_4f883b9a.
            eprintln!("portal saved_deals query failed: {e}");
            Vec::new()
        });

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

/// PUT /api/v1/visitor/profile — the signed-in visitor edits their own name / email / phone.
///
/// t_4fd9fd2a: the Save Profile button in frontend/user-saved.html PUTs this path, but the route
/// was registered GET-only, so every save returned 405 and the page showed "Failed to update".
/// The visitor routes sit BEFORE the auth guard (see create_router), so this repeats the same
/// manual JWT check `visitor_profile` uses instead of leaning on an auth layer that is not there.
pub async fn update_visitor_profile(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateVisitorProfileRequest>,
) -> ApiResult<impl IntoResponse> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AppError::Unauthorized)?;
    let claims = verify_token(token, &s.config.jwt_secret).map_err(|_| AppError::Unauthorized)?;
    let visitor_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    // Blank / absent fields mean "leave unchanged" (COALESCE below) — the Save button sends only
    // name + email. An email that IS supplied must be a real address (same normaliser as register,
    // so a saved address still matches the lower(email) lookup used at login).
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let phone = req
        .phone
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let email = match req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(raw) => {
            Some(crate::security::email_addr::normalize(raw).map_err(AppError::Validation)?)
        }
        None => None,
    };

    // Friendly conflict before the UPDATE; the unique index remains the hard backstop underneath.
    if let Some(ref new_email) = email {
        let taken = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM visitor_accounts WHERE lower(email) = $1 AND id <> $2",
        )
        .bind(new_email)
        .bind(visitor_id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(0);
        if taken > 0 {
            return Err(AppError::Duplicate(
                "A visitor account with this email already exists".to_string(),
            ));
        }
    }

    let affected = sqlx::query(
        "UPDATE visitor_accounts SET name = COALESCE($1, name), email = COALESCE($2, email), \
         phone = COALESCE($3, phone), updated_at = NOW() WHERE id = $4",
    )
    .bind(&name)
    .bind(&email)
    .bind(&phone)
    .bind(visitor_id)
    .execute(&s.db)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::Duplicate("A visitor account with this email already exists".to_string())
        }
        _ => AppError::Database(e),
    })?;

    if affected.rows_affected() == 0 {
        return Err(AppError::NotFound("Visitor not found".to_string()));
    }

    let visitor =
        sqlx::query_as::<_, VisitorAccount>("SELECT * FROM visitor_accounts WHERE id = $1")
            .bind(visitor_id)
            .fetch_one(&s.db)
            .await?;

    Ok(Json(json!({
        "status": "updated",
        "visitor": {
            "id": visitor.id,
            "email": visitor.email,
            "name": visitor.name,
            "phone": visitor.phone,
            "directory_id": visitor.directory_id,
            "is_active": visitor.is_active,
            "created_at": visitor.created_at,
        }
    })))
}

// ── Directory Feature Config ──

/// GET /api/v1/directories/:id/features
/// Public — returns the feature config for a directory
pub async fn get_directory_features(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let feature_config: Option<Value> =
        sqlx::query_scalar(r#"SELECT feature_config FROM directories WHERE id = $1"#)
            .bind(id)
            .fetch_optional(&s.db)
            .await?
            .flatten();

    let zaarhub_config: Value = sqlx::query_scalar(
        r#"SELECT COALESCE(zaarhub_config, '{}'::jsonb) FROM directories WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(json!({}));

    match feature_config {
        Some(config) => Ok(Json(json!({
            "directory_id": id,
            "feature_config": config,
            "zaarhub_config": zaarhub_config,
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
    if !is_super_admin(&claims) {
        return Err(AppError::Forbidden("Admin access required".to_string()));
    }

    // Check directory exists
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM directories WHERE id = $1")
        .bind(id)
        .fetch_one(&s.db)
        .await?;

    if exists == 0 {
        return Err(AppError::NotFound("Directory not found".to_string()));
    }

    // Build the new config from current + updates
    let current_config: Value = sqlx::query_scalar(
        r#"SELECT COALESCE(feature_config, '{}'::jsonb) FROM directories WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(json!({}));

    let mut config = current_config.as_object().cloned().unwrap_or_default();

    if let Some(v) = req.deals {
        config.insert("deals".to_string(), json!(v));
    }
    if let Some(v) = req.blogging {
        config.insert("blogging".to_string(), json!(v));
    }
    if let Some(v) = req.community_posts {
        config.insert("community_posts".to_string(), json!(v));
    }
    if let Some(v) = req.b2b_marketplace {
        config.insert("b2b_marketplace".to_string(), json!(v));
    }
    if let Some(v) = req.visitor_accounts {
        config.insert("visitor_accounts".to_string(), json!(v));
    }
    if let Some(v) = req.gamification {
        config.insert("gamification".to_string(), json!(v));
    }
    // Generic passthrough: the admin card sends the whole feature_config object.
    if let Some(fc) = &req.feature_config {
        for (k, v) in fc.iter() {
            config.insert(k.clone(), v.clone());
        }
    }

    let new_config = Value::Object(config);

    sqlx::query(r#"UPDATE directories SET feature_config = $1, updated_at = NOW() WHERE id = $2"#)
        .bind(&new_config)
        .bind(id)
        .execute(&s.db)
        .await?;

    // Also handle ZaarHub-specific config in zaarhub_config column
    let any_zh = [
        req.network_visible.is_some(),
        req.homepage_featured.is_some(),
        req.show_deals.is_some(),
        req.show_events.is_some(),
        req.show_reviews.is_some(),
        req.show_activity.is_some(),
        req.guarantee_enabled.is_some(),
        req.guarantee_title.is_some(),
        req.guarantee_text.is_some(),
    ]
    .iter()
    .any(|&x| x);

    if any_zh {
        // Current ZaarHub config, so a partial guarantee update preserves the other keys.
        let current_zh: Value = sqlx::query_scalar(
            r#"SELECT COALESCE(zaarhub_config, '{}'::jsonb) FROM directories WHERE id = $1"#,
        )
        .bind(id)
        .fetch_one(&s.db)
        .await
        .unwrap_or(json!({}));

        // The patch set is a JSON OBJECT bound as a parameter, so the statement itself is one
        // compile-time literal and nothing is built as text at run time (gate rule 5d). Semantics
        // are unchanged: `jsonb || jsonb` merges the same keys (jsonb is order-insensitive), and
        // the leading `|| '{}'::jsonb` that the old interpolated literal carried is kept verbatim.
        let mut zh_patch = serde_json::Map::new();
        if let Some(v) = req.network_visible {
            zh_patch.insert("network_visible".to_string(), json!(v));
        }
        if let Some(v) = req.homepage_featured {
            zh_patch.insert("homepage_featured".to_string(), json!(v));
        }
        if let Some(v) = req.show_deals {
            zh_patch.insert("show_deals".to_string(), json!(v));
        }
        if let Some(v) = req.show_events {
            zh_patch.insert("show_events".to_string(), json!(v));
        }
        if let Some(v) = req.show_reviews {
            zh_patch.insert("show_reviews".to_string(), json!(v));
        }
        if let Some(v) = req.show_activity {
            zh_patch.insert("show_activity".to_string(), json!(v));
        }

        // B90: hiring / quality guarantee. Merge into the existing {enabled,title,text}
        // object so a partial update (e.g. only the toggle) keeps title/text intact.
        if req.guarantee_enabled.is_some()
            || req.guarantee_title.is_some()
            || req.guarantee_text.is_some()
        {
            let mut g = current_zh
                .get("guarantee")
                .cloned()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({}));
            if let Some(obj) = g.as_object_mut() {
                if let Some(v) = req.guarantee_enabled {
                    obj.insert("enabled".to_string(), json!(v));
                }
                if let Some(v) = &req.guarantee_title {
                    obj.insert("title".to_string(), json!(v));
                }
                if let Some(v) = &req.guarantee_text {
                    obj.insert("text".to_string(), json!(v));
                }
            }
            zh_patch.insert("guarantee".to_string(), g);
        }

        if !zh_patch.is_empty() {
            const ZH_PATCH_SQL: &str = "UPDATE directories SET zaarhub_config = zaarhub_config || '{}'::jsonb || $2::jsonb, updated_at = NOW() WHERE id = $1";
            sqlx::query(ZH_PATCH_SQL)
                .bind(id)
                .bind(Value::Object(zh_patch))
                .execute(&s.db)
                .await?;
        }
    }

    // Fetch the full updated zaarhub_config to return
    let zaarhub_config: Value = sqlx::query_scalar(
        r#"SELECT COALESCE(zaarhub_config, '{}'::jsonb) FROM directories WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .unwrap_or(json!({}));

    Ok(Json(json!({
        "directory_id": id,
        "feature_config": new_config,
        "zaarhub_config": zaarhub_config,
    })))
}
