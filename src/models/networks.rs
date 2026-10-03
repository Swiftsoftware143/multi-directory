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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryWithNetwork {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub template: Option<String>,
    pub color_scheme: Option<serde_json::Value>,
    pub network_id: Option<Uuid>,
    pub network_name: Option<String>,
    pub network_slug: Option<String>,
    pub url_type: Option<String>,
    pub url_value: Option<String>,
    pub custom_domain: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// ── CreateDirectoryRequest (extended with network option) ────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateDirectoryRequestV2 {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub template: Option<String>,
    pub color_scheme: Option<serde_json::Value>,
    /// "standalone" | "new_network" | "connect:<network_id>"
    pub network_mode: Option<String>,
    /// Used when network_mode="connect" — network_id to join
    pub parent_network_id: Option<Uuid>,
    /// URL config — only meaningful when connecting to a network
    pub url_type: Option<String>, // "subdomain" | "subfolder" | "custom"
    pub url_value: Option<String>, // the slug for subdomain/subfolder
    pub custom_domain: Option<String>, // only for url_type="custom"
}
