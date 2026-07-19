use crate::email::send_reset_email;
// Auth handler functions.

use axum::{
    extract::{State, Extension},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::AppState;
use crate::error::{AppError, ApiResult};
use super::models::*;
use super::middleware::{create_token, verify_token};

/// POST /api/v1/auth/register
pub async fn register(
    State(s): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    if req.email.is_empty() || req.password.is_empty() || req.name.is_empty() {
        return Err(AppError::Validation("Name, email, and password are required".to_string()));
    }
    if req.password.len() < 6 {
        return Err(AppError::Validation("Password must be at least 6 characters".to_string()));
    }

    // Check if user already exists
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE email = \x241"
    )
    .bind(&req.email)
    .fetch_one(&s.db)
    .await
    .unwrap_or(0);

    if existing > 0 {
        return Err(AppError::Duplicate("A user with this email already exists".to_string()));
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
    let tenant_name = req.tenant_name.unwrap_or_else(|| format!("{}'s Directory", req.name));
    let tenant_slug = req.tenant_slug.unwrap_or_else(|| {
        req.name.to_lowercase().replace(' ', "-").chars().take(30).collect()
    });

    let tenant_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tenants (id, name, slug, is_active) VALUES (\x241, \x242, \x243, true)",
    )
    .bind(tenant_id)
    .bind(&tenant_name)
    .bind(&tenant_slug)
    .execute(&s.db)
    .await?;

    // Create user
    let user_id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, email, password_hash, name, role, is_active, created_at, updated_at) VALUES (\x241, \x242, \x243, \x244, \x245, 'admin', true, \x246, \x246)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(&req.email)
    .bind(&password_hash)
    .bind(&req.name)
    .bind(now)
    .execute(&s.db)
    .await?;

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
    };
    let refresh_token = create_token(&refresh_claims, &s.config.jwt_secret)?;

    let user_response = UserResponse {
        id: user_id,
        tenant_id,
        email: req.email,
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
    use argon2::{
        Argon2, PasswordHash, PasswordVerifier,
    };

    // Manually query user row
    let row = sqlx::query(
        "SELECT id, tenant_id, email, password_hash, name, role, is_active, last_login_at, created_at, updated_at FROM users WHERE email = \x241"
    )
    .bind(&req.email)
    .fetch_optional(&s.db)
    .await?;
    
    let row = row
        .ok_or_else(|| {
            tracing::warn!("Login failed: user not found for {}", &req.email);
            AppError::InvalidCredentials
        })?;

    use sqlx::Row;
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

    if !user.is_active {
        tracing::warn!("Login failed: account deactivated for {}", &user.email);
        return Err(AppError::Forbidden("Account is deactivated".to_string()));
    }

    // Verify password
    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|e| AppError::Hash(e.to_string()))?;
    let argon2 = Argon2::default();
    argon2
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::InvalidCredentials)?;

    // Update last_login
    sqlx::query("UPDATE users SET last_login_at = NOW() WHERE id = \x241")
        .bind(user.id)
        .execute(&s.db)
        .await?;

    // Get tenant info
    let tenant_row = sqlx::query("SELECT id, name, slug, is_active FROM tenants WHERE id = \x241")
        .bind(user.tenant_id)
        .fetch_optional(&s.db)
        .await?;

    let tenant = tenant_row.map(|r| -> Result<TenantResponse, sqlx::Error> {
        Ok(TenantResponse {
            id: r.try_get("id")?,
            name: r.try_get("name")?,
            slug: r.try_get("slug")?,
            is_active: r.try_get("is_active")?,
        })
    }).transpose()?;

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
    .await?
    .ok_or(AppError::NotFound("User not found".to_string()))?;

    use sqlx::Row;
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

    Ok(Json(json!({ "user": UserResponse::from(user) })))
}

/// PUT /api/v1/auth/password
pub async fn change_password(
    State(s): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher, PasswordHash, PasswordVerifier,
    };

    if req.new_password.len() < 6 {
        return Err(AppError::Validation("New password must be at least 6 characters".to_string()));
    }

    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| AppError::Unauthorized)?;

    let row = sqlx::query(
        "SELECT password_hash FROM users WHERE id = \x241"
    )
    .bind(user_id)
    .fetch_optional(&s.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    use sqlx::Row;
    let password_hash: String = row.try_get("password_hash")?;

    // Verify current password
    let parsed_hash = PasswordHash::new(&password_hash)
        .map_err(|e| AppError::Hash(e.to_string()))?;
    let argon2 = Argon2::default();
    argon2
        .verify_password(req.current_password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::InvalidCredentials)?;

    // Hash new password
    let salt = SaltString::generate(&mut OsRng);
    let new_hash = Argon2::default()
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|e| AppError::Hash(e.to_string()))?
        .to_string();

    sqlx::query("UPDATE users SET password_hash = \x241, updated_at = NOW() WHERE id = \x242")
        .bind(&new_hash)
        .bind(user_id)
        .execute(&s.db)
        .await?;

    Ok((StatusCode::OK, Json(json!({"message": "Password updated successfully"}))))
}

/// POST /api/v1/auth/forgot-password
pub async fn forgot_password(
    State(s): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    let row_opt = sqlx::query("SELECT id FROM users WHERE email = \x241")
        .bind(&req.email)
        .fetch_optional(&s.db)
        .await?;

    if let Some(row) = row_opt {
        use sqlx::Row;
        let user_id: Uuid = row.try_get("id")?;
        let token = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + chrono::Duration::hours(24);

        sqlx::query("UPDATE password_resets SET used = true WHERE user_id = \x241 AND used = false")
            .bind(user_id)
            .execute(&s.db)
            .await.ok();

        sqlx::query(
            "INSERT INTO password_resets (user_id, token, expires_at) VALUES (\x241, \x242, \x243)",
        )
        .bind(user_id)
        .bind(&token)
        .bind(expires_at)
        .execute(&s.db)
        .await?;

        match send_reset_email(&s.db, &req.email, &token).await {
            Ok(_) => tracing::info!("Password reset email sent to {}", req.email),
            Err(e) => tracing::error!("Failed to send password reset email to {}: {}", req.email, e),
        }
        // Send password reset email via SMTP
    }

    Ok((StatusCode::OK, Json(json!({"message": "If the email exists, a password reset link has been sent"}))))
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
        return Err(AppError::Validation("New password must be at least 6 characters".to_string()));
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

    sqlx::query("UPDATE users SET password_hash = \x241, updated_at = NOW() WHERE id = \x242")
        .bind(&new_hash)
        .bind(reset_user_id)
        .execute(&s.db)
        .await?;

    sqlx::query("UPDATE password_resets SET used = true WHERE id = \x241")
        .bind(reset_id)
        .execute(&s.db)
        .await?;

    Ok((StatusCode::OK, Json(json!({"message": "Password has been reset successfully"}))))
}
