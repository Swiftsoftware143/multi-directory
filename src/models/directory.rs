//! Data models for directories, businesses, reviews, domains, and branding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ── Directory ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Directory {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub owner_id: Option<Uuid>,
    pub template: Option<String>,
    pub color_scheme: Option<serde_json::Value>,
    pub network_id: Option<Uuid>,
    pub url_type: Option<String>,
    pub url_value: Option<String>,
    pub custom_domain: Option<String>,
    pub city: Option<String>,
    pub template_config: Option<serde_json::Value>,
    pub tracking_enabled: Option<bool>,
    pub head_injection: Option<String>,
    pub body_injection: Option<String>,
    pub footer_injection: Option<String>,
    pub email_signature_html: Option<String>,
    pub email_signature_text: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDirectoryRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub template: Option<String>,
    pub color_scheme: Option<serde_json::Value>,
    pub city: Option<String>,
    pub template_config: Option<serde_json::Value>,
    /// "standalone" | "new_network" | "connect"
    pub network_mode: Option<String>,
    /// When network_mode="connect", the network UUID to join
    pub parent_network_id: Option<Uuid>,
    /// URL config — only when connecting to a network
    pub url_type: Option<String>,
    pub url_value: Option<String>,
    pub custom_domain: Option<String>,
    pub head_injection: Option<String>,
    pub body_injection: Option<String>,
    pub footer_injection: Option<String>,
    pub email_signature_html: Option<String>,
    pub email_signature_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDirectoryRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub template: Option<String>,
    pub color_scheme: Option<serde_json::Value>,
    pub city: Option<String>,
    pub template_config: Option<serde_json::Value>,
    pub network_id: Option<Uuid>,
    pub url_type: Option<String>,
    pub url_value: Option<String>,
    pub custom_domain: Option<String>,
    pub head_injection: Option<String>,
    pub body_injection: Option<String>,
    pub footer_injection: Option<String>,
    pub email_signature_html: Option<String>,
    pub email_signature_text: Option<String>,
}

// ── DirectoryCategory ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DirectoryCategory {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub sort_order: Option<i32>,
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DirectoryCategoryWithParent {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub sort_order: Option<i32>,
    pub parent_id: Option<Uuid>,
    pub parent_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub slug: String,
    pub sort_order: Option<i32>,
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCategoryRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub sort_order: Option<i32>,
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct BulkMoveRequest {
    pub category_ids: Vec<Uuid>,
    pub target_category_id: Uuid,
    pub move_businesses: bool,
    pub move_subcategories: bool,
}

#[derive(Debug, Deserialize)]
pub struct BulkDeleteCategoriesRequest {
    pub category_ids: Vec<Uuid>,
    pub reassign_to: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct CategoryVisitorSummary {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub unique_visitors: i64,
    pub total_events: i64,
    pub businesses_clicked: i64,
    pub listing_views: i64,
    pub phone_clicks: i64,
    pub website_clicks: i64,
}

#[derive(Debug, Serialize)]
pub struct CategoryDeleteCheck {
    pub business_count: i64,
    pub subcategory_count: i64,
}

#[derive(Debug, Serialize)]
pub struct CategoryBulkResult {
    pub success: bool,
    pub message: String,
    pub affected_categories: usize,
}



// ── Business ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Business {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category_id: Option<Uuid>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub rating: Option<f64>,
    pub review_count: Option<i32>,
    pub is_active: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBusinessRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub category_id: Option<Uuid>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBusinessRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub category_id: Option<Uuid>,
    pub address: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub zip: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListBusinessesQuery {
    pub q: Option<String>,
    pub category_id: Option<Uuid>,
    pub city: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub radius: Option<f64>,
    pub sort: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct BusinessSearchResult {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub city: Option<String>,
    pub state: Option<String>,
    pub category_name: Option<String>,
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for BusinessSearchResult {
    fn from_row(row: &sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            slug: row.try_get("slug")?,
            city: row.try_get("city")?,
            state: row.try_get("state")?,
            category_name: row.try_get("category_name")?,
        })
    }
}

// ---- Review ------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Review {
    pub id: Uuid,
    pub business_id: Option<Uuid>,
    pub rating: i32,
    pub title: Option<String>,
    pub content: Option<String>,
    pub reviewer_name: Option<String>,
    pub reviewer_email: Option<String>,
    pub status: Option<String>,
    pub featured: Option<bool>,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub directory_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub is_verified: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReviewRequest {
    pub business_id: Uuid,
    pub rating: i32,
    pub title: Option<String>,
    pub content: Option<String>,
    pub reviewer_name: Option<String>,
    pub reviewer_email: Option<String>,
    pub source: Option<String>,
    pub source_url: Option<String>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReviewRequest {
    pub rating: Option<i32>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub reviewer_name: Option<String>,
    pub reviewer_email: Option<String>,
    pub featured: Option<bool>,
    pub source: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ReviewStats {
    pub business_id: Uuid,
    pub average_rating: Option<f64>,
    pub total_reviews: i64,
    pub rating_1: i64,
    pub rating_2: i64,
    pub rating_3: i64,
    pub rating_4: i64,
    pub rating_5: i64,
}

// ── DomainMapping ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainMapping {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub domain: String,
    pub domain_type: String,
    pub status: Option<String>,
    pub ssl_enabled: Option<bool>,
    pub cloudflare_record_id: Option<String>,
    pub dns_records: Option<serde_json::Value>,
    pub verification_token: Option<String>,
    pub auto_configured: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for DomainMapping {
    fn from_row(row: &sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            directory_id: row.try_get("directory_id")?,
            domain: row.try_get("domain")?,
            domain_type: row.try_get("type")?,
            status: row.try_get("status")?,
            ssl_enabled: row.try_get("ssl_enabled")?,
            cloudflare_record_id: row.try_get("cloudflare_record_id")?,
            dns_records: row.try_get("dns_records")?,
            verification_token: row.try_get("verification_token")?,
            auto_configured: row.try_get("auto_configured")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct RegisterDomainRequest {
    pub domain: String,
    pub domain_type: Option<String>,
}

// ── DirectoryBranding ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DirectoryBranding {
    pub id: Uuid,
    pub directory_id: Option<Uuid>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub accent_color: Option<String>,
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    pub heading_color: Option<String>,
    pub link_color: Option<String>,
    pub button_background: Option<String>,
    pub button_text: Option<String>,
    pub heading_font: Option<String>,
    pub body_font: Option<String>,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub meta_title_template: Option<String>,
    pub meta_description_template: Option<String>,
    pub extracted_from_url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBrandingRequest {
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub accent_color: Option<String>,
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    pub heading_color: Option<String>,
    pub link_color: Option<String>,
    pub button_background: Option<String>,
    pub button_text: Option<String>,
    pub heading_font: Option<String>,
    pub body_font: Option<String>,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub meta_title_template: Option<String>,
    pub meta_description_template: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractColorsRequest {
    pub url: String,
}

// ── BusinessMeta (template-specific fields) ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BusinessMeta {
    pub id: Uuid,
    pub business_id: Option<Uuid>,
    pub template: String,
    pub meta_data: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertBusinessMetaRequest {
    pub template: Option<String>,
    pub meta_data: Option<serde_json::Value>,
}

// ── Template info ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub industries: Vec<String>,
    pub preview_image: Option<String>,
}

// ── Dashboard Stats ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DashboardStats {
    pub total_directories: i64,
    pub total_businesses: i64,
    pub total_reviews: i64,
    pub total_domains: i64,
    pub active_directories: i64,
    pub published_directories: i64,
}

// ── Paginated response wrapper ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub total_pages: i64,
}


// ── BlogPost ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BlogPost {
    pub id: uuid::Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: String,
    pub directory_id: uuid::Uuid,
    pub published: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub focus_keyword: Option<String>,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub canonical_url: Option<String>,
    pub robots_meta: Option<String>,
    pub featured_image_url: Option<String>,
    pub featured_image_alt: Option<String>,
    pub schema_type: Option<String>,
    pub author_id: Option<uuid::Uuid>,
    pub service_id: Option<uuid::Uuid>,
    pub location_id: Option<uuid::Uuid>,
    pub scheduled_at: Option<chrono::NaiveDateTime>,
    pub template_id: Option<uuid::Uuid>,
    pub template_data: Option<serde_json::Value>,
    pub is_master: Option<bool>,
    pub master_post_id: Option<uuid::Uuid>,
    pub blog_category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub feature_image: Option<String>,
    pub feature_video: Option<String>,
    pub media_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuthorProfile {
    pub id: uuid::Uuid,
    pub directory_id: uuid::Uuid,
    pub user_id: Option<uuid::Uuid>,
    pub name: String,
    pub slug: String,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub twitter_url: Option<String>,
    pub linkedin_url: Option<String>,
    pub website_url: Option<String>,
    pub role: Option<String>,
    pub is_active: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBlogPostRequest {
    pub title: String,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: String,
    pub directory_id: uuid::Uuid,
    pub published: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBlogPostRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub excerpt: Option<String>,
    pub content: Option<String>,
    pub published: Option<bool>,
}

// ── LegalPage ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LegalPage {
    pub id: uuid::Uuid,
    pub title: String,
    pub page_type: String,
    pub content: String,
    pub published: Option<bool>,
    pub is_global: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLegalPageRequest {
    pub title: String,
    pub page_type: Option<String>,
    pub content: String,
    pub published: Option<bool>,
    pub is_global: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLegalPageRequest {
    pub title: Option<String>,
    pub page_type: Option<String>,
    pub content: Option<String>,
    pub published: Option<bool>,
    pub is_global: Option<bool>,
}

// ── ImportLog ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ImportLog {
    pub id: Uuid,
    pub entity_type: String,
    pub filename: Option<String>,
    pub rows_total: Option<i32>,
    pub rows_success: Option<i32>,
    pub rows_failed: Option<i32>,
    pub errors: Option<serde_json::Value>,
    pub directory_id: Option<Uuid>,
    pub status: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ImportDataRequest {
    pub entity_type: String,
    pub data: Vec<serde_json::Value>,
    pub directory_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExportTemplate {
    pub id: Uuid,
    pub name: String,
    pub entity_type: String,
    pub fields: serde_json::Value,
    pub directory_id: Option<Uuid>,
    pub delimiter: Option<String>,
    pub include_header: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateExportTemplateRequest {
    pub name: String,
    pub entity_type: String,
    pub fields: Vec<String>,
    pub directory_id: Option<Uuid>,
    pub delimiter: Option<String>,
    pub include_header: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateExportTemplateRequest {
    pub name: Option<String>,
    pub entity_type: Option<String>,
    pub fields: Option<Vec<String>>,
    pub directory_id: Option<Uuid>,
    pub delimiter: Option<String>,
    pub include_header: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
    pub fields: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub import_log_id: Uuid,
    pub rows_total: i32,
    pub rows_success: i32,
    pub rows_failed: i32,
    pub errors: Vec<serde_json::Value>,
    pub status: String,
}
