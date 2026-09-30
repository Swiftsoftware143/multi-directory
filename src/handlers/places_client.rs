//! Google Places (LEGACY) response handling — the answer, never swallowed.
//!
//! The Maps **Places** API answers HTTP 200 even when it REFUSED the request: the
//! refusal lives in the JSON body's `status` field (`REQUEST_DENIED`,
//! `OVER_QUERY_LIMIT`, `INVALID_REQUEST`, `UNKNOWN_ERROR` …) with a human
//! `error_message`. Treating that body like a successful "no results" response —
//! the bug behind card B78 — makes a rejected key indistinguishable from a genuine
//! miss, so a wrong/expired key looked exactly like an empty directory for weeks.
//!
//! Every Places call in this app must run the parsed body through
//! [`check_response`]. It logs the HTTP status, Google's own `status` and Google's
//! own `error_message` at **WARN** (never the request URL — that carries the key),
//! and returns a structured [`PlacesError`] the caller can surface to the API
//! response, the enrichment log and the admin UI.
//!
//! A `ZERO_RESULTS` (or an empty result array) is NOT an error — it is an honest
//! "no such business", and it is logged at INFO so the two can never be confused.

use serde_json::Value;

/// The two Places `status` values that mean "the provider answered us": `OK`
/// (a reply, possibly empty) and `ZERO_RESULTS` (a reply that found nothing).
pub const PLACES_NON_ERROR_STATUSES: [&str; 2] = ["OK", "ZERO_RESULTS"];

/// Which Places sub-API was called — used as the log/response context so an error
/// names the endpoint without ever printing the URL (which contains the API key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacesCall {
    Autocomplete,
    FindPlace,
    TextSearch,
    Details,
}

impl PlacesCall {
    pub fn as_str(self) -> &'static str {
        match self {
            PlacesCall::Autocomplete => "place/autocomplete",
            PlacesCall::FindPlace => "place/findplacefromtext",
            PlacesCall::TextSearch => "place/textsearch",
            PlacesCall::Details => "place/details",
        }
    }
}

/// A refused or failed Places call, carrying everything an operator needs to fix
/// it and nothing that leaks the key.
#[derive(Debug, Clone)]
pub struct PlacesError {
    /// Which Places endpoint refused us (`place/findplacefromtext`, …).
    pub call: String,
    /// The HTTP status the transport returned (usually 200 even on refusal).
    pub http_status: u16,
    /// Google's own `status` field, or `"UNKNOWN"` when the body had none.
    pub provider_status: String,
    /// Google's own `error_message`, when it sent one.
    pub provider_error: Option<String>,
}

impl std::fmt::Display for PlacesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "google_places {} refused: HTTP {} provider_status={} error={}",
            self.call,
            self.http_status,
            self.provider_status,
            self.provider_error.as_deref().unwrap_or("-")
        )
    }
}

impl std::error::Error for PlacesError {}

// Let handlers use `?` on a Places call: the provider's own message travels up to the API response
// and the log instead of being swallowed. A rejected key or an exceeded quota must be visible —
// invisible provider errors are what made a working key look broken (card B78).
impl From<PlacesError> for crate::error::AppError {
    fn from(e: PlacesError) -> Self {
        crate::error::AppError::BadRequest(e.to_string())
    }
}

impl PlacesError {
    /// The distinguishable outcome tag for an API response — `provider_error` when
    /// Google named a status (key rejected, quota, bad request), `transport_error`
    /// when we never got a parseable Places answer at all.
    pub fn outcome(&self) -> &'static str {
        if self.provider_status.is_empty() || self.provider_status == "UNKNOWN" {
            "transport_error"
        } else {
            "provider_error"
        }
    }
}

/// Google's `status` from a Places body, upper-cased and trimmed. Missing ⇒ `"UNKNOWN"`.
pub fn provider_status(body: &Value) -> String {
    body.get("status")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

/// Google's `error_message` from a Places body, trimmed.
pub fn provider_error(body: &Value) -> Option<String> {
    body.get("error_message")
        .and_then(|m| m.as_str())
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
}

/// Inspect a Places response. `Ok(())` means Google answered (`OK`/`ZERO_RESULTS`;
/// an empty result set is returned as `Ok` and the caller reports a real miss).
/// `Err` means Google REFUSED or the transport failed — always logged at WARN with
/// the HTTP status, Google's `status` and Google's `error_message`, never the URL.
pub fn check_response(body: &Value, http_status: u16, call: PlacesCall) -> Result<(), PlacesError> {
    let status = provider_status(body);
    let err = provider_error(body);

    let transport_ok = (200..300).contains(&http_status);
    let answered = PLACES_NON_ERROR_STATUSES.contains(&status.as_str());

    if transport_ok && answered {
        if status == "ZERO_RESULTS" {
            tracing::info!(
                "[places] {} — HTTP {} status=ZERO_RESULTS (honest no-result, not an error)",
                call.as_str(),
                http_status
            );
        }
        return Ok(());
    }

    let e = PlacesError {
        call: call.as_str().to_string(),
        http_status,
        provider_status: status,
        provider_error: err,
    };
    tracing::warn!("[places] {}", e);
    Err(e)
}

/// Build a `PlacesError` for a response we could not even parse (non-JSON body,
/// connection failure): the caller passes the transport detail as the message.
pub fn transport_error(call: PlacesCall, http_status: u16, detail: &str) -> PlacesError {
    let e = PlacesError {
        call: call.as_str().to_string(),
        http_status,
        provider_status: "UNKNOWN".to_string(),
        provider_error: Some(detail.to_string()),
    };
    tracing::warn!("[places] {}", e);
    e
}
