use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::AppState;
use crate::error::AppError;
use super::models::Claims;

pub async fn auth_middleware(
    State(s): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized)?;

    let token = auth_header.strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized)?;

    let claims = verify_token(token, &s.config.jwt_secret)
        .map_err(|_| AppError::Unauthorized)?;

    let mut req = req;
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};

    let decoding_key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 30;
    validation.validate_exp = true;
    validation.set_issuer(&["multidirectory"]);
    validation.set_audience(&["multidirectory-api"]);

    let token_data = decode::<Claims>(token, &decoding_key, &validation)?;
    Ok(token_data.claims)
}

pub fn create_token(claims: &Claims, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    use jsonwebtoken::{encode, Header, EncodingKey};

    let encoding_key = EncodingKey::from_secret(secret.as_bytes());
    Ok(encode(&Header::default(), claims, &encoding_key)?)
}

/// Check if user has admin or super_admin role
pub fn is_admin(claims: &Claims) -> bool {
    claims.role == "admin" || claims.role == "super_admin"
}

/// Check if user is super_admin
pub fn is_super_admin(claims: &Claims) -> bool {
    claims.role == "super_admin"
}
