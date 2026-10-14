//! Data models for networks, branding, and homepage sections.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ── Network ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Network {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub root_domain: Option<String>,
    pub status: Option<String>,
    pub owner_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNetworkRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub root_domain: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNetworkRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub root_domain: Option<String>,
    pub status: Option<String>,
}

// ── Network Branding ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NetworkBranding {
    pub id: Uuid,
    pub network_id: Uuid,
    pub logo_url: Option<String>,
    pub logo_footer_url: Option<String>,
    pub favicon_url: Option<String>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub accent_color: Option<String>,
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    pub heading_color: Option<String>,
    pub heading_font: Option<String>,
    pub body_font: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNetworkBrandingRequest {
    pub logo_url: Option<String>,
    pub logo_footer_url: Option<String>,
    pub favicon_url: Option<String>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub accent_color: Option<String>,
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    pub heading_color: Option<String>,
    pub heading_font: Option<String>,
    pub body_font: Option<String>,
}

// ── Extended Directory with network info ─────────────────────────────────────

// ── CreateDirectoryRequest (extended with network option) ────────────────────
