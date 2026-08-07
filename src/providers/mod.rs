//! Event provider trait and shared types.
//! Each provider (Eventbrite, Meetup, ICS Feed, n8n Webhook) implements
//! the EventProvider trait, enabling a unified sync pipeline.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod eventbrite;

/// Configuration passed to a provider's fetch_events method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: String,
    pub city: String,
    pub state: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub radius_miles: Option<i32>,
    pub categories: Option<Vec<String>>,
}

/// Normalized event representation returned by all providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub source_id: String,
    pub title: String,
    pub description: Option<String>,
    pub start_time: String,
    pub end_time: Option<String>,
    pub venue_name: Option<String>,
    pub venue_address: Option<String>,
    pub venue_city: Option<String>,
    pub venue_state: Option<String>,
    pub url: Option<String>,
    pub image_url: Option<String>,
    pub is_free: bool,
    pub price_text: Option<String>,
    pub category: Option<String>,
    pub organizer_name: Option<String>,
}

/// Every event provider must implement this trait.
#[async_trait]
pub trait EventProvider: Send + Sync {
    /// Returns a short string identifying the provider (e.g. "eventbrite").
    fn provider_type(&self) -> String;

    /// Quick connectivity check using the provider's API key.
    async fn test_connection(&self, api_key: &str) -> Result<bool, String>;

    /// Fetch events from the provider for the given configuration.
    async fn fetch_events(&self, config: &ProviderConfig) -> Result<Vec<RawEvent>, String>;
}
