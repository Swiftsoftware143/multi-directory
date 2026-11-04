use crate::email::send_reset_email;
// Auth handler functions.

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use super::middleware::create_token;
use super::models::*;
use crate::error::{ApiResult, AppError};
use crate::AppState;

/// The business a counter/portal caller administers, in the shape `/auth/me` reports it.
///
/// The scanner page holds nothing but a bearer token. It used to display the caller's email
/// prefix as the business name (its login response is nested, so every top-level `name` lookup
/// missed), and it posted that string with each scan — which the scan ledger then recorded as the
/// business name. The claim is the only ownership link both account tables share, so resolve the
/// business from it once here and let the page show and post the real thing.
pub(crate) async fn claimed_business_for(
    db: &sqlx::PgPool,
    account_id: Uuid,
) -> Option<(Uuid, String)> {
    sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT b.id, b.name
             FROM claimed_businesses c
             JOIN businesses b ON b.id = c.business_id
            WHERE c.is_active
              AND (c.user_id = $1 OR c.visitor_account_id = $1)
            ORDER BY c.created_at DESC
            LIMIT 1"#,
    )
    .bind(account_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// `/auth/me` body: the account itself plus, when it administers a business, that business's
/// id and name at the TOP level — the shape the counter scanner reads.
fn me_payload(user: User, business: Option<(Uuid, String)>) -> serde_json::Value {
    let (business_id, business_name) = match business {
        Some((id, name)) => (Some(id.to_string()), Some(name)),
        None => (None, None),
    };
    json!({
        "user": UserResponse::from(user),
        "business_id": business_id,
        "business_name": business_name,
    })
}

/// POST /api/v1/auth/register
pub async fn register(
    State(s): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.email.is_empty() || req.password.is_empty() || req.name.is_empty() {
        return Err(AppError::Validation(
            "Name, email, and password are required".to_string(),
        ));
    }
    if req.password.len() < 6 {
        return Err(AppError::Validation(
            "Password must be at least 6 characters".to_string(),
        ));
    }

    // The login identity: trim + lowercase + refuse anything that is not an address, BEFORE the
    // first SELECT. `users.email` is the only address a credentials mail can ever reach, and the
    // duplicate check below must see the same value the INSERT will store (t_01f183b1).
    let email = crate::security::email_addr::normalize(&req.email).map_err(AppError::Validation)?;

    // Check if user already exists
    let existing =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_one(&s.db)
            .await
            .unwrap_or(0);

    if existing > 0 {
        return Err(AppError::Duplicate(
            "A user with this email already exists".to_string(),
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

    // Create tenant
    let tenant_name = req
        .tenant_name
        .unwrap_or_else(|| format!("{}'s Directory", req.name));
    let tenant_slug = req.tenant_slug.unwrap_or_else(|| {
        req.name
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .take(30)
            .collect()
    });

    // `tenants.slug` is UNIQUE, but it is derived from the display name (or taken from the
    // request), so the SECOND person to sign up under a name already in use got a bare 500
    // "Database error" and could not create an account at all. Walk to the next free slug.
    let tenant_id = Uuid::new_v4();
    let slug_base = match tenant_slug.trim_matches('-') {
        "" => "directory".to_string(),
        s => s.chars().take(40).collect(),
    };
    let mut slug = slug_base.clone();
    // The walk must not run dry. The old fixed `base-2 … base-8` candidate list was exhausted
    // by the ninth tenant that shared a display name and the signup got the bare 500 right back
    // (measured 2026-10-14: tenants `probe-webhooks` … `probe-webhooks-8`; every signup after
    // that failed). Try the readable `-2 … -9` first, then a random suffix, so the loop
    // converges instead of running out of candidates.
    for attempt in 0..16u32 {
        let inserted = sqlx::query(
            "INSERT INTO tenants (id, name, slug, is_active) VALUES (\x241, \x242, \x243, true)",
        )
        .bind(tenant_id)
        .bind(&tenant_name)
        .bind(&slug)
        .execute(&s.db)
        .await;
        match inserted {
            Ok(_) => break,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() && attempt < 15 => {
                slug = if attempt < 8 {
                    format!("{slug_base}-{}", attempt + 2)
                } else {
                    format!(
                        "{}-{}",
                        slug_base,
                        &Uuid::new_v4().simple().to_string()[..8]
                    )
                };
            }
            Err(e) => return Err(e.into()),
        }
    }

    // Create user
    let user_id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active, created_at, updated_at) VALUES (\x241, \x242, \x243, \x244, \x245, 'admin', true, \x246, \x246)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind(now)
    .execute(&s.db)
    .await?;

    // Loyalty is native Multi-Directory code — there is no per-user IncentiveSwift account to
    // provision here. The only external seam is the onboarding/IQS survey proxy, which the app
    // authenticates as the platform. (This block used to POST to a hardcoded localhost:8083 for a
    // programme that did not exist there, so it failed on every signup.)

    // Create JWT
    let now_ts = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        tid: tenant_id.to_string(),
        role: "admin".to_string(),
        exp: now_ts + s.config.jwt_access_expiry as usize,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };
    let token = create_token(&claims, &s.config.jwt_secret)?;

    let refresh_claims = Claims {
        sub: user_id.to_string(),
        tid: tenant_id.to_string(),
        role: "admin".to_string(),
        exp: now_ts + s.config.jwt_refresh_expiry as usize,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };
    let refresh_token = create_token(&refresh_claims, &s.config.jwt_secret)?;

    let user_response = UserResponse {
        id: user_id,
        tenant_id,
        email,
        name: req.name,
        role: "admin".to_string(),
        is_active: true,
        last_login_at: None,
        created_at: now,
    };

    Ok((
        StatusCode::CREATED,
        Json(json!(RegisterResponse {
            access_token: token,
            refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: s.config.jwt_access_expiry,
            user: user_response,
            tenant: TenantResponse {
                id: tenant_id,
                name: tenant_name,
                slug: tenant_slug,
                is_active: true,
            },
        })),
    ))
}

/// POST /api/v1/auth/login
pub async fn login(
    State(s): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> ApiResult<impl IntoResponse> {
    // Try users table first, then fall back to visitor_accounts
    use sqlx::Row;

    // Same trim+lowercase as the writers, paired with lower(email) on the column so rows stored
    // before the normalisation existed still resolve (t_01f183b1).
    let email_key = crate::security::email_addr::lookup_key(&req.email);

    let row = sqlx::query(
        "SELECT id, tenant_id, email, password_hash, name, role, is_active, last_login_at, created_at, updated_at FROM users WHERE lower(email) = $1"
    )
    .bind(&email_key)
    .fetch_optional(&s.db)
    .await?;

    let (user, is_visitor) = if let Some(r) = row {
        (
            User {
                id: r.try_get("id")?,
                tenant_id: r.try_get("tenant_id")?,
                email: r.try_get("email")?,
                password_hash: r.try_get("password_hash")?,
                name: r.try_get("name")?,
                role: r.try_get("role")?,
                is_active: r.try_get("is_active")?,
                last_login_at: r.try_get("last_login_at")?,
                created_at: r.try_get("created_at")?,
                updated_at: r.try_get("updated_at")?,
            },
            false,
        )
    } else {
        // Fall back to visitor_accounts (ZaarHub business portal users)
        let vrow = sqlx::query(
            "SELECT id, email, password_hash, name, is_active FROM visitor_accounts WHERE lower(email) = $1"
        )
        .bind(&email_key)
        .fetch_optional(&s.db)
        .await?
        .ok_or_else(|| {
            tracing::warn!("Login failed: user not found for {}", &req.email);
            AppError::InvalidCredentials
        })?;

        let is_active: bool = vrow.try_get("is_active")?;
        if !is_active {
            tracing::warn!("Login failed: account deactivated for {}", &req.email);
            return Err(AppError::Forbidden("Account is deactivated".to_string()));
        }

        (
            User {
                id: vrow.try_get("id")?,
                // Use the SwiftSoftware system tenant for visitor/business portal accounts
                tenant_id: crate::system_tenant::system_tenant_uuid(),
                email: vrow.try_get("email")?,
                password_hash: vrow.try_get("password_hash")?,
                name: vrow.try_get("name")?,
                role: "company_admin".to_string(),
                is_active: true,
                last_login_at: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            true,
        )
    };

    if !user.is_active {
        tracing::warn!("Login failed: account deactivated for {}", &user.email);
        return Err(AppError::Forbidden("Account is deactivated".to_string()));
    }

    // Verify password. ONE failure mode: a wrong password AND a stored hash that cannot be parsed
    // both answer "invalid credentials". This line used to map a parse failure to
    // `AppError::Hash`, so a damaged row answered `500 Password hashing error` to an anonymous
    // caller and told it which accounts were broken (measured live 2026-10-30).
    if !crate::security::password_check::matches(&user.password_hash, &req.password) {
        return Err(AppError::InvalidCredentials);
    }

    // Update last_login. `table` is one of two hardcoded names, so the statement is one of two
    // COMPLETE compile-time literals — no statement text is built at run time (gate rule 5d).
    const TOUCH_LAST_LOGIN_USERS: &str = "UPDATE users SET last_login_at = NOW() WHERE id = \x241";
    const TOUCH_LAST_LOGIN_VISITORS: &str =
        "UPDATE visitor_accounts SET last_login_at = NOW() WHERE id = \x241";
    let update_query = if is_visitor {
        TOUCH_LAST_LOGIN_VISITORS
    } else {
        TOUCH_LAST_LOGIN_USERS
    };
    sqlx::query(update_query)
        .bind(user.id)
        .execute(&s.db)
        .await?;

    // Get tenant info (skip for visitors — use a placeholder)
    let tenant = if is_visitor {
        None
    } else {
        let tenant_row =
            sqlx::query("SELECT id, name, slug, is_active FROM tenants WHERE id = \x241")
                .bind(user.tenant_id)
                .fetch_optional(&s.db)
                .await?;
        tenant_row
            .map(|r| -> Result<TenantResponse, sqlx::Error> {
                Ok(TenantResponse {
                    id: r.try_get("id")?,
                    name: r.try_get("name")?,
                    slug: r.try_get("slug")?,
                    is_active: r.try_get("is_active")?,
                })
            })
            .transpose()?
    };

    // Generate JWT
    let now_ts = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user.id.to_string(),
        tid: user.tenant_id.to_string(),
        role: user.role.clone(),
        exp: now_ts + s.config.jwt_access_expiry as usize,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };
    let token = create_token(&claims, &s.config.jwt_secret)?;

    let refresh_claims = Claims {
        sub: user.id.to_string(),
        tid: user.tenant_id.to_string(),
        role: user.role.clone(),
        exp: now_ts + s.config.jwt_refresh_expiry as usize,
        iat: now_ts,
        aud: Some("multidirectory-api".to_string()),
        iss: Some("multidirectory".to_string()),
        impersonating: None,
    };
    let refresh_token = create_token(&refresh_claims, &s.config.jwt_secret)?;

    Ok(Json(json!(LoginResponse {
        access_token: token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: s.config.jwt_access_expiry,
        user: user.into(),
        tenant,
    })))
}

/// GET /api/v1/auth/me
pub async fn me(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> ApiResult<impl IntoResponse> {
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let row = sqlx::query(
        "SELECT id, tenant_id, email, password_hash, name, role, is_active, last_login_at, created_at, updated_at FROM users WHERE id = \x241"
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await?;

    use sqlx::Row;
    if let Some(row) = row {
        let user = User {
            id: row.try_get("id")?,
            tenant_id: row.try_get("tenant_id")?,
            email: row.try_get("email")?,
            password_hash: row.try_get("password_hash")?,
            name: row.try_get("name")?,
            role: row.try_get("role")?,
            is_active: row.try_get("is_active")?,
            last_login_at: row.try_get("last_login_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        };

        let business = claimed_business_for(&s.db, user.id).await;
        return Ok(Json(me_payload(user, business)));
    }

    // Fall back to visitor_accounts (ZaarHub business-portal and counter-scanner accounts),
    // mirroring the login path. Before this, /auth/me answered 404 "User not found" for every
    // visitor account, so the scanner's saved-session arm (scanner.html init) treated a still
    // valid token as dead, wiped the session and forced a re-login on every visit — even though
    // /auth/login and /loyalty/scan both accept these accounts. Same account type login accepts,
    // same role login assigns ("company_admin"), so the two endpoints agree.
    let vrow = sqlx::query(
        "SELECT id, email, name, is_active, last_login_at, created_at, updated_at FROM visitor_accounts WHERE id = \x241"
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::NotFound("User not found".to_string()))?;

    let user = User {
        id: vrow.try_get("id")?,
        // Visitor/business accounts live on the SwiftSoftware system tenant, exactly as login sets it.
        tenant_id: crate::system_tenant::system_tenant_uuid(),
        email: vrow.try_get("email")?,
        // Never surfaced: UserResponse omits password_hash. Kept empty so no secret leaves the DB.
        password_hash: String::new(),
        name: vrow.try_get("name")?,
        role: "company_admin".to_string(),
        is_active: vrow.try_get("is_active")?,
        last_login_at: vrow.try_get("last_login_at")?,
        created_at: vrow.try_get("created_at")?,
        updated_at: vrow.try_get("updated_at")?,
    };

    let business = claimed_business_for(&s.db, user_id).await;
    Ok(Json(me_payload(user, business)))
}

/// PUT /api/v1/auth/password
pub async fn change_password(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    if req.new_password.len() < 6 {
        return Err(AppError::Validation(
            "New password must be at least 6 characters".to_string(),
        ));
    }

    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let row = sqlx::query("SELECT password_hash FROM users WHERE id = \x241")
        .bind(user_id)
        .fetch_optional(&s.db)
        .await?
        .ok_or(AppError::Unauthorized)?;

    use sqlx::Row;
    let password_hash: String = row.try_get("password_hash")?;

    // Verify current password. A stored hash that cannot be parsed is a refusal, not a 500.
    if !crate::security::password_check::matches(&password_hash, &req.current_password) {
        return Err(AppError::InvalidCredentials);
    }

    // Hash new password
    let salt = SaltString::generate(&mut OsRng);
    let new_hash = Argon2::default()
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Hash(e.to_string()))?
        .to_string();

    // kanban t_dd9d99c9 — a password CHANGE must end the sessions that predate it. `NOW()` lands
    // in `users.sessions_valid_from`, which `routes::session_gate` / `routes::auth_guard` compare
    // against every token's `iat`; without this stamp the old bearer kept working for its full
    // 24 h life, which is exactly the "I changed my password because I thought I was compromised"
    // case. A session minted in this same second still works on purpose (`iat` is
    // second-granularity, so it cannot be told apart from the re-login that follows).
    sqlx::query(
        "UPDATE users SET password_hash = \x241, sessions_valid_from = NOW(), updated_at = NOW() \
         WHERE id = \x242",
    )
    .bind(&new_hash)
    .bind(user_id)
    .execute(&s.db)
    .await?;

    Ok((
        StatusCode::OK,
        Json(json!({"message": "Password updated successfully"})),
    ))
}

/// POST /api/v1/auth/forgot-password
pub async fn forgot_password(
    State(s): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    // The user must never be told a reset link was sent when nothing can leave. Check the mail
    // transport BEFORE looking the account up, so the answer is byte-for-byte identical whether or
    // not the address is registered (no account-existence oracle), and refuse honestly when the
    // platform has no email provider configured. Config-only: a provider outage is not reported
    // here (email::mail_transport_ready).
    if let Err(reason) = crate::email::mail_transport_ready(&s.db).await {
        tracing::error!(
            "forgot-password refused: no email could be delivered ({}) — save an Email/SMTP \
             transport in the admin panel (Integration Center / Email) and retry",
            reason
        );
        return Err(AppError::ServiceUnavailable(
            "We can't email a reset link yet: this site's email provider is not set up. \
             Please contact support so they can enable password resets."
                .to_string(),
        ));
    }

    // Read-side: the same trim+lowercase as the writers, so an address stored normalised resolves
    // no matter how the caller typed it (t_01f183b1). A malformed value matches nothing and the
    // endpoint keeps its generic response — never an account-existence oracle.
    let email_key = crate::security::email_addr::lookup_key(&req.email);

    let row_opt = sqlx::query("SELECT id FROM users WHERE lower(email) = $1")
        .bind(&email_key)
        .fetch_optional(&s.db)
        .await?;

    if let Some(row) = row_opt {
        use sqlx::Row;
        let user_id: Uuid = row.try_get("id")?;
        let token = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + chrono::Duration::hours(24);

        sqlx::query(
            "UPDATE password_resets SET used = true WHERE user_id = \x241 AND used = false",
        )
        .bind(user_id)
        .execute(&s.db)
        .await
        .ok();

        sqlx::query(
            "INSERT INTO password_resets (user_id, token, expires_at) VALUES (\x241, \x242, \x243)",
        )
        .bind(user_id)
        .bind(&token)
        .bind(expires_at)
        .execute(&s.db)
        .await?;

        match send_reset_email(&s.db, &email_key, &token).await {
            Ok(_) => tracing::info!("Password reset email sent to {}", email_key),
            Err(e) => tracing::error!(
                "Failed to send password reset email to {}: {}",
                email_key,
                e
            ),
        }
        // Send password reset email via SMTP
    }

    Ok((
        StatusCode::OK,
        Json(json!({"message": "If the email exists, a password reset link has been sent"})),
    ))
}

/// POST /api/v1/auth/reset-password
pub async fn reset_password(
    State(s): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    if req.new_password.len() < 6 {
        return Err(AppError::Validation(
            "New password must be at least 6 characters".to_string(),
        ));
    }

    let reset_row = sqlx::query(
        "SELECT id, user_id FROM password_resets WHERE token = \x241 AND used = false AND expires_at > NOW()",
    )
    .bind(&req.token)
    .fetch_optional(&s.db)
    .await?
    .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".to_string()))?;

    use sqlx::Row;
    let reset_id: Uuid = reset_row.try_get("id")?;
    let reset_user_id: Uuid = reset_row.try_get("user_id")?;

    let salt = SaltString::generate(&mut OsRng);
    let new_hash = Argon2::default()
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Hash(e.to_string()))?
        .to_string();

    // kanban t_dd9d99c9 — a RESET is a password change too: it is the door a user takes when they
    // believe the account is compromised, so the sessions that existed before it must die with it.
    // Same stamp, same comparison (`iat` vs `sessions_valid_from`) as the change arm above.
    sqlx::query(
        "UPDATE users SET password_hash = \x241, sessions_valid_from = NOW(), updated_at = NOW() \
         WHERE id = \x242",
    )
    .bind(&new_hash)
    .bind(reset_user_id)
    .execute(&s.db)
    .await?;

    sqlx::query("UPDATE password_resets SET used = true WHERE id = \x241")
        .bind(reset_id)
        .execute(&s.db)
        .await?;

    Ok((
        StatusCode::OK,
        Json(json!({"message": "Password has been reset successfully"})),
    ))
}
