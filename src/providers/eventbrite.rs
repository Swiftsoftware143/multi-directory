//! Eventbrite provider implementation.
//!
//! Uses the Eventbrite v3 REST API (`www.eventbriteapi.com/v3`) with
//! Bearer-token authentication.

use async_trait::async_trait;
use serde::Deserialize;

use super::{EventProvider, ProviderConfig, RawEvent};

const EVENTBRITE_API_BASE: &str = "https://www.eventbriteapi.com/v3";

/// The Eventbrite provider — stateless, constructed once.
pub struct EventbriteProvider;

#[derive(Debug, Deserialize)]
struct EventbriteUserResponse {
    #[allow(dead_code)]
    id: String,
}

#[derive(Debug, Deserialize)]
struct EventbriteEventsResponse {
    events: Vec<EventbriteEvent>,
    pagination: EventbritePagination,
}

#[derive(Debug, Deserialize)]
struct EventbritePagination {
    page_count: i32,
    #[allow(dead_code)]
    page_number: i32,
}

#[derive(Debug, Deserialize)]
struct EventbriteEvent {
    id: String,
    name: EventbriteName,
    description: Option<EventbriteDescription>,
    start: EventbriteTime,
    end: EventbriteTime,
    url: Option<String>,
    logo: Option<EventbriteLogo>,
    venue: Option<EventbriteVenue>,
    is_free: Option<bool>,
    ticket_availability: Option<EventbriteTicketAvailability>,
    category: Option<EventbriteCategory>,
    organizer: Option<EventbriteOrganizer>,
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventbriteName {
    text: String,
}

#[derive(Debug, Deserialize)]
struct EventbriteDescription {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventbriteTime {
    local: String,
    #[allow(dead_code)]
    timezone: String,
}

#[derive(Debug, Deserialize)]
struct EventbriteLogo {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventbriteVenue {
    name: Option<String>,
    address: Option<EventbriteAddress>,
}

#[derive(Debug, Deserialize)]
struct EventbriteAddress {
    localized_address_display: Option<String>,
    city: Option<String>,
    region: Option<String>,
    postal_code: Option<String>,
    country: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventbriteTicketAvailability {
    minimum_ticket_price: Option<EventbritePrice>,
    maximum_ticket_price: Option<EventbritePrice>,
}

#[derive(Debug, Deserialize)]
struct EventbritePrice {
    major_value: Option<String>,
    #[allow(dead_code)]
    currency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventbriteCategory {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventbriteOrganizer {
    name: Option<String>,
}

#[async_trait]
impl EventProvider for EventbriteProvider {
    fn provider_type(&self) -> String {
        "eventbrite".to_string()
    }

    async fn test_connection(&self, api_key: &str) -> Result<bool, String> {
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/users/me/", EVENTBRITE_API_BASE))
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if resp.status().is_success() {
            Ok(true)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("Eventbrite API returned {}: {}", status, body))
        }
    }

    async fn fetch_events(&self, config: &ProviderConfig) -> Result<Vec<RawEvent>, String> {
        let client = reqwest::Client::new();
        let mut all_raw: Vec<RawEvent> = Vec::new();

        let location = if let Some(ref state) = config.state {
            format!("{}+{}", config.city, state)
        } else {
            config.city.clone()
        };

        let radius = config.radius_miles.unwrap_or(25);
        let page_size: i32 = 50;

        // First request to get pagination info
        let base_url = format!(
            "{}/events/search/?location.address={}&location.within={}mi&expand=venue&page_size={}",
            EVENTBRITE_API_BASE, location, radius, page_size
        );

        let first_resp = client
            .get(&base_url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        let status = first_resp.status();
        if !status.is_success() {
            let body = first_resp.text().await.unwrap_or_default();
            return Err(format!("Eventbrite API returned {}: {}", status, body));
        }

        let first_body: EventbriteEventsResponse = first_resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse Eventbrite response: {}", e))?;

        let page_count = first_body.pagination.page_count;
        all_raw.extend(map_events(first_body.events));

        // Fetch remaining pages (pages 2..page_count)
        for page in 2..=page_count {
            let page_url = format!("{}&page={}", base_url, page);
            let resp = client
                .get(&page_url)
                .header("Authorization", format!("Bearer {}", config.api_key))
                .send()
                .await
                .map_err(|e| format!("HTTP request for page {} failed: {}", page, e))?;

            if !resp.status().is_success() {
                tracing::warn!("Eventbrite page {} returned non-success: {}", page, resp.status());
                continue;
            }

            let body: EventbriteEventsResponse = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse Eventbrite page {}: {}", page, e))?;

            all_raw.extend(map_events(body.events));
        }

        Ok(all_raw)
    }
}

/// Map a batch of Eventbrite events into our normalized RawEvent format.
fn map_events(eb_events: Vec<EventbriteEvent>) -> Vec<RawEvent> {
    eb_events.into_iter().map(map_one).collect()
}

fn map_one(e: EventbriteEvent) -> RawEvent {
    let price_text = e
        .ticket_availability
        .as_ref()
        .and_then(|t| {
            let lo = t
                .minimum_ticket_price
                .as_ref()
                .and_then(|p| p.major_value.as_deref());
            let hi = t
                .maximum_ticket_price
                .as_ref()
                .and_then(|p| p.major_value.as_deref());
            match (lo, hi) {
                (Some(l), Some(h)) if l == h => Some(format!("${}", l)),
                (Some(l), Some(h)) => Some(format!("${} - ${}", l, h)),
                (Some(l), None) => Some(format!("From ${}", l)),
                (None, Some(h)) => Some(format!("Up to ${}", h)),
                (None, None) => None,
            }
        });

    let description = e
        .description
        .and_then(|d| d.text)
        .filter(|t| !t.is_empty())
        .or(e.summary);

    let venue = e.venue.as_ref();

    RawEvent {
        source_id: e.id,
        title: e.name.text,
        description,
        start_time: e.start.local,
        end_time: Some(e.end.local),
        venue_name: venue.and_then(|v| v.name.clone()),
        venue_address: venue
            .and_then(|v| v.address.as_ref())
            .and_then(|a| a.localized_address_display.clone()),
        venue_city: venue
            .and_then(|v| v.address.as_ref())
            .and_then(|a| a.city.clone()),
        venue_state: venue
            .and_then(|v| v.address.as_ref())
            .and_then(|a| a.region.clone()),
        url: e.url,
        image_url: e.logo.and_then(|l| l.url),
        is_free: e.is_free.unwrap_or(false),
        price_text,
        category: e.category.and_then(|c| c.name),
        organizer_name: e.organizer.and_then(|o| o.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify map_one correctly maps a full Eventbrite event with all fields.
    #[test]
    fn test_map_full_event() {
        let eb = EventbriteEvent {
            id: "evt-123".into(),
            name: EventbriteName {
                text: "Summer Jazz Night".into(),
            },
            description: Some(EventbriteDescription {
                text: Some("A wonderful evening of jazz.".into()),
            }),
            start: EventbriteTime {
                local: "2026-08-15T19:00:00".into(),
                timezone: "America/New_York".into(),
            },
            end: EventbriteTime {
                local: "2026-08-15T22:00:00".into(),
                timezone: "America/New_York".into(),
            },
            url: Some("https://eventbrite.com/e/evt-123".into()),
            logo: Some(EventbriteLogo {
                url: Some("https://img.com/logo.png".into()),
            }),
            venue: Some(EventbriteVenue {
                name: Some("The Blue Note".into()),
                address: Some(EventbriteAddress {
                    localized_address_display: Some("123 Main St".into()),
                    city: Some("Melbourne".into()),
                    region: Some("FL".into()),
                    postal_code: Some("32901".into()),
                    country: Some("US".into()),
                }),
            }),
            is_free: Some(false),
            ticket_availability: Some(EventbriteTicketAvailability {
                minimum_ticket_price: Some(EventbritePrice {
                    major_value: Some("25".into()),
                    currency: Some("USD".into()),
                }),
                maximum_ticket_price: Some(EventbritePrice {
                    major_value: Some("50".into()),
                    currency: Some("USD".into()),
                }),
            }),
            category: Some(EventbriteCategory {
                name: Some("Music".into()),
            }),
            organizer: Some(EventbriteOrganizer {
                name: Some("Jazz Productions Inc".into()),
            }),
            summary: None,
        };

        let raw = map_one(eb);

        assert_eq!(raw.source_id, "evt-123");
        assert_eq!(raw.title, "Summer Jazz Night");
        assert_eq!(
            raw.description.as_deref(),
            Some("A wonderful evening of jazz.")
        );
        assert_eq!(raw.start_time, "2026-08-15T19:00:00");
        assert_eq!(raw.end_time.as_deref(), Some("2026-08-15T22:00:00"));
        assert_eq!(raw.venue_name.as_deref(), Some("The Blue Note"));
        assert_eq!(raw.venue_address.as_deref(), Some("123 Main St"));
        assert_eq!(raw.venue_city.as_deref(), Some("Melbourne"));
        assert_eq!(raw.venue_state.as_deref(), Some("FL"));
        assert_eq!(
            raw.url.as_deref(),
            Some("https://eventbrite.com/e/evt-123")
        );
        assert_eq!(raw.image_url.as_deref(), Some("https://img.com/logo.png"));
        assert!(!raw.is_free);
        assert_eq!(raw.price_text.as_deref(), Some("$25 - $50"));
        assert_eq!(raw.category.as_deref(), Some("Music"));
        assert_eq!(
            raw.organizer_name.as_deref(),
            Some("Jazz Productions Inc")
        );
    }

    /// Free event with no venue → all optional fields should be None.
    #[test]
    fn test_map_free_event_no_venue() {
        let eb = EventbriteEvent {
            id: "evt-free".into(),
            name: EventbriteName {
                text: "Park Yoga".into(),
            },
            description: None,
            start: EventbriteTime {
                local: "2026-09-01T08:00:00".into(),
                timezone: "America/Chicago".into(),
            },
            end: EventbriteTime {
                local: "2026-09-01T09:00:00".into(),
                timezone: "America/Chicago".into(),
            },
            url: None,
            logo: None,
            venue: None,
            is_free: Some(true),
            ticket_availability: None,
            category: None,
            organizer: None,
            summary: Some("Morning yoga in the park".into()),
        };

        let raw = map_one(eb);

        assert_eq!(raw.source_id, "evt-free");
        assert_eq!(raw.title, "Park Yoga");
        // Summary is used as fallback when description is missing
        assert_eq!(
            raw.description.as_deref(),
            Some("Morning yoga in the park")
        );
        assert!(raw.is_free);
        assert!(raw.price_text.is_none());
        assert!(raw.venue_name.is_none());
        assert!(raw.venue_address.is_none());
        assert!(raw.venue_city.is_none());
        assert!(raw.venue_state.is_none());
        assert!(raw.category.is_none());
        assert!(raw.organizer_name.is_none());
        assert!(raw.url.is_none());
        assert!(raw.image_url.is_none());
    }

    /// Single-price ticket (min == max) should show just one price.
    #[test]
    fn test_map_single_price_ticket() {
        let eb = EventbriteEvent {
            id: "evt-single".into(),
            name: EventbriteName {
                text: "Workshop".into(),
            },
            description: None,
            start: EventbriteTime {
                local: "2026-10-01T10:00:00".into(),
                timezone: "UTC".into(),
            },
            end: EventbriteTime {
                local: "2026-10-01T15:00:00".into(),
                timezone: "UTC".into(),
            },
            url: None,
            logo: None,
            venue: None,
            is_free: Some(false),
            ticket_availability: Some(EventbriteTicketAvailability {
                minimum_ticket_price: Some(EventbritePrice {
                    major_value: Some("99".into()),
                    currency: Some("USD".into()),
                }),
                maximum_ticket_price: Some(EventbritePrice {
                    major_value: Some("99".into()),
                    currency: Some("USD".into()),
                }),
            }),
            category: None,
            organizer: None,
            summary: None,
        };

        let raw = map_one(eb);
        assert_eq!(raw.price_text.as_deref(), Some("$99"));
    }
}
