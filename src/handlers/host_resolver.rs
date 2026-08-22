//! Simple Host → Directory lookup in the fallback service.
//! This file exists to keep the module import happy.
//! The actual logic is in routes.rs fallback_service.

use crate::AppState;
use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Placeholder - actual logic moved to fallback_service in routes.rs
pub async fn resolve_host(_req: Request, _next: Next) -> Response {
    _next.run(_req).await
}
